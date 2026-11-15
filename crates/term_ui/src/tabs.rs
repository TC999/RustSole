//! 标签页容器:窗口内的多个终端标签 + 自绘标题栏 + 标签条。
//!
//! 布局(自上而下只有一行 chrome):
//!   ┌──────────────────────────────────────────────┐
//!   │ 标签条(可伸缩)          │ 最小化/最大化/关闭 │  ← TITLEBAR_HEIGHT
//!   ├──────────────────────────────────────────────┤
//!   │ 当前标签的终端内容(TerminalView)           │
//!   └──────────────────────────────────────────────┘
//!
//! 为什么由 TabsView 而不是每个 TerminalView 自己画标题栏:窗口只有一个标题栏与一组
//! 窗口按钮,标签共享它们。TerminalView 因此只负责终端内容(ARCHITECTURE.md 不变量 1:
//! Session 独立于 View,标签切换只是换一个 View 显示)。
//!
//! 键盘:焦点始终在某个 TerminalView 的终端上,Action 沿焦点路径冒泡到本视图的根节点,
//! 因此在根节点上注册 on_action 就能同时覆盖所有标签,不需要抢键盘事件。

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rgb, AnyElement, App, ClickEvent, Context, Entity, Focusable, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render, SharedString, Window,
};

use crate::actions::{CloseActiveTab, NewTab, NextTab, PrevTab};
use crate::tabbar::{
    render_tabbar, CloseHover, TabDoubleClickHandler, TabHit, TabItem, TabbarHandlers,
};
use crate::theme::{window as window_theme, WindowControl, WindowControlsShown, TITLEBAR_HEIGHT};
use crate::titlebar::{render_titlebar, TitlebarHandlers, TitlebarModel};
use crate::view::{TerminalEvent, TerminalView, DEFAULT_TAB_TITLE};

/// 判定"用户想拖动窗口"的位移阈值(逻辑像素)。
///
/// 低于它视为手抖,仍按点击处理,保证双击空白新建标签(MOUSE-005)可用。
/// 取 4px,与 gpui 自身的 DRAG_THRESHOLD(2px,div.rs:47)同量级。
const DRAG_THRESHOLD: f32 = 4.0;

/// 标签页容器视图。
pub struct TabsView {
    /// 所有标签,顺序即标签条上的顺序。
    tabs: Vec<Entity<TerminalView>>,
    /// 当前激活标签的下标。始终指向 `tabs` 内的有效元素(只要 tabs 非空)。
    active: usize,
    /// 标签条上悬停的标签(仅用于绘制悬停底色)。
    hovered_tab: Option<usize>,
    /// 标题栏上当前悬停的窗口控制按钮。
    hovered_control: Option<WindowControl>,
    /// 当前悬停的标签关闭图标(标签下标)。None = 没有悬停在任何关闭图标上。
    hovered_close: Option<usize>,
    /// 在标签条空白处按下左键时的坐标(视口坐标)。
    ///
    /// 只是"可能的拖动起点":位移超过阈值才真正发起窗口拖动。
    /// 不能在按下时就发起 —— Windows 的 begin_window_drag 会阻塞到鼠标松开,
    /// 那样双击空白新建标签就永远等不到第二下。
    drag_pending: Option<Point<Pixels>>,
    /// 新标签的初始网格尺寸(与创建窗口时用的常量一致)。
    init_cols: u16,
    init_rows: u16,
    /// 最近一次新建标签失败的原因(标签条上显示);成功后清空。
    status: Option<SharedString>,
    /// 各标签的标题变更订阅。与 `tabs` 一一对应;标签被移除时一并丢弃订阅。
    title_subscriptions: Vec<gpui::Subscription>,
    /// 窗口激活状态观察者(首次 render 时注册)。
    ///
    /// 窗口从最小化恢复时,GPUI 那一帧的呈现可能不包含自绘内容,表现为标签条与
    /// 窗口按钮整组消失、点击其他窗口才恢复(用户实测)。标题栏由本视图绘制,
    /// 因此补偿重绘必须挂在这一层,挂到子视图上管不到标题栏。
    activation_subscription: Option<gpui::Subscription>,
}

impl TabsView {
    /// 创建一个只含一个标签的容器。
    ///
    /// # Errors
    /// 首个终端会话启动失败时返回(PTY/shell 无法分配),由入口显示给用户。
    pub fn new(cx: &mut App, cols: u16, rows: u16) -> gpui::Result<Entity<Self>> {
        let first = TerminalView::new(cx, cols, rows, TITLEBAR_HEIGHT)?;
        Ok(cx.new(|cx| {
            // 订阅首个标签的标题变化(WIN-008);其余标签在 new_tab 里同样订阅
            let subscription = cx.subscribe(&first, |_view, _tab, _event: &TerminalEvent, cx| {
                cx.notify();
            });
            Self {
                tabs: vec![first],
                active: 0,
                hovered_tab: None,
                hovered_control: None,
                hovered_close: None,
                drag_pending: None,
                init_cols: cols,
                init_rows: rows,
                status: None,
                title_subscriptions: vec![subscription],
                activation_subscription: None,
            }
        }))
    }

    /// 当前激活标签。`None` 只在标签列表被清空后短暂出现(随即关闭窗口)。
    pub fn active_tab(&self) -> Option<&Entity<TerminalView>> {
        self.tabs.get(self.active)
    }

    /// 标签数量。
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// 新建标签并切换到它。
    ///
    /// 新会话按当前激活标签的网格尺寸创建:标签在同一窗口内显示,尺寸必然相同,
    /// 用窗口当前尺寸可以避免新标签启动后再 resize 一次(会触发子进程重绘)。
    fn new_tab(&mut self, cx: &mut Context<Self>) {
        // 优先按当前激活标签的实际网格建新会话:窗口可能已被缩放,用启动常量会让
        // 新标签先按错误的行列启动、下一帧再 resize,子进程会看到两次窗口变化。
        let (cols, rows) = match self.tabs.get(self.active) {
            Some(tab) => {
                let tab = tab.read(cx);
                (tab.cols(), tab.rows())
            }
            None => (self.init_cols, self.init_rows),
        };
        let created = TerminalView::new(cx, cols, rows, TITLEBAR_HEIGHT);
        let tab = match created {
            Ok(tab) => tab,
            Err(err) => {
                // 启动失败属于用户可感知但不致命的错误(CODING_STANDARDS.md 第 3 节:
                // 用户可恢复的错误显示在界面上,不 panic)。保持现有标签可用,把原因
                // 挂在标签条上显示,下一次成功新建时清掉。
                // 注意:错误文本只含系统错误描述,不含终端输出或粘贴内容(第 4 节)。
                self.status = Some(SharedString::from(format!("新建标签失败: {err}")));
                cx.notify();
                return;
            }
        };
        let subscription = cx.subscribe(&tab, |_view, _tab, _event: &TerminalEvent, cx| {
            cx.notify();
        });
        self.tabs.push(tab);
        self.title_subscriptions.push(subscription);
        self.active = self.tabs.len() - 1;
        self.status = None;
        cx.notify();
    }

    /// 关闭指定标签。
    ///
    /// 关闭最后一个标签时:让调用方移除窗口(`close_window_when_empty` 由渲染阶段判断)。
    /// 这里只负责列表与激活下标的维护,窗口动作留给渲染阶段的 `window.remove_window()`,
    /// 因为 `Context::remove_window` 不在这里可用(需要 `Window`)。
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        // 与 tabs 一一对应:丢弃该标签的订阅,避免订阅表与标签列表错位。
        // Subscription 在 drop 时自动注销,这里显式丢弃即可。
        drop(self.title_subscriptions.remove(index));
        if self.tabs.is_empty() {
            self.active = 0;
        } else if index < self.active {
            // 关掉的是左边的标签:激活下标左移一位,保持同一个标签仍然是激活的
            self.active -= 1;
        } else if index == self.active {
            // 关掉的是当前标签:优先激活右邻居,没有则退回最后一个
            self.active = self.active.min(self.tabs.len() - 1);
        }
        self.hovered_tab = None;
        // 下标在移除后已失效:不清掉会让另一个标签错误地显示关闭图标高亮
        self.hovered_close = None;
        // 关掉的是当前标签时,焦点原本在已释放的视图上:交给新激活标签
        if !self.tabs.is_empty() {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    /// 切换激活标签(越界或重复点击当前标签时忽略)。
    ///
    /// 切换后必须把键盘焦点移到新标签的终端:焦点原本留在旧标签上,而旧标签
    /// 不再被渲染,键盘输入会落进一个看不见的会话。
    fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.active = index;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// 把键盘焦点交给激活标签的终端,并让它刷新一次画面。
    fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(tab) = self.tabs.get(self.active) {
            let tab = tab.clone();
            tab.update(cx, |tab, cx| {
                tab.focus_terminal(window);
                tab.refresh_on_activate(cx);
            });
        }
    }

    /// 循环切换到下一个标签。
    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        self.active = (self.active + 1) % self.tabs.len();
        self.focus_active(window, cx);
        cx.notify();
    }

    /// 循环切换到上一个标签。
    fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        self.focus_active(window, cx);
        cx.notify();
    }

    /// 标签条的呈现数据。
    fn tab_items(&self, cx: &App) -> Vec<TabItem> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let tab = tab.read(cx);
                TabItem {
                    title: tab.title(),
                    active: index == self.active,
                    exited: tab.exited(),
                    close_hovered: self.hovered_close == Some(index),
                }
            })
            .collect()
    }
}

/// 关闭标签后的窗口处理:最后一个标签关闭时关闭整个窗口。
///
/// Windows 与 Linux 行为一致(WIN-003 关闭按钮之外的第二条退出路径)。
impl TabsView {
    fn close_tab_or_window(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab(index, window, cx);
        if self.tabs.is_empty() {
            window.remove_window();
        }
    }
}

impl Focusable for TabsView {
    /// 返回激活标签的焦点句柄(容器本身不接收键盘)。
    ///
    /// 入口在窗口创建后用它把焦点交给当前终端。标签列表为空的瞬间窗口正在关闭,
    /// 此时返回一个临时句柄即可,不应 panic(CODING_STANDARDS.md 第 3 节)。
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        match self.active_tab() {
            Some(tab) => tab.read(cx).focus_handle(cx),
            None => cx.focus_handle(),
        }
    }
}

impl Render for TabsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 窗口重新激活(含从最小化恢复)后延迟一小段再刷新一次,确保有一帧完整的
        // 重绘被提交。标题栏由本视图绘制,补偿只能挂在这里(见字段注释)。
        if self.activation_subscription.is_none() {
            let subscription = cx.observe_window_activation(window, |_view, window, cx| {
                if window.is_window_active() {
                    cx.spawn(async move |this, cx| {
                        gpui::Timer::after(std::time::Duration::from_millis(120)).await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                }
            });
            self.activation_subscription = Some(subscription);
        }

        let title = self
            .active_tab()
            .map(|tab| tab.read(cx).title())
            .unwrap_or_else(|| SharedString::from(DEFAULT_TAB_TITLE));

        let items = self.tab_items(cx);
        let hovered_tab = self.hovered_tab;
        let status = self.status.clone();
        let handlers = TabbarHandlers {
            click: Rc::new(cx.listener(|view, hit: &TabHit, window, cx| match hit {
                TabHit::Select(index) => view.select_tab(*index, window, cx),
                // 关闭图标:与中键、Ctrl+Shift+W 走同一条路径,最后一个标签时关闭窗口
                TabHit::Close(index) => view.close_tab_or_window(*index, window, cx),
                TabHit::NewTab => view.new_tab(cx),
            })),
            middle: Rc::new(cx.listener(
                |view, target: &Option<usize>, window: &mut Window, cx| match target {
                    // MOUSE-004:中键点标签 = 关闭它;中键点空白 = 新建标签
                    Some(index) => view.close_tab_or_window(*index, window, cx),
                    None => view.new_tab(cx),
                },
            )),
            // MOUSE-005:双击标签条空白处新建标签。
            // 这里不用 cx.listener:它要求回调接收事件引用,而双击的判定已在
            // tabbar 内完成,回调与事件内容无关。
            double_click_blank: {
                let view = cx.entity().downgrade();
                let handler: TabDoubleClickHandler =
                    Rc::new(move |_window: &mut Window, cx: &mut App| {
                        view.update(cx, |view, cx| view.new_tab(cx)).ok();
                    });
                handler
            },
            hover: Rc::new(cx.listener(
                |view, hovered: &Option<usize>, _window: &mut Window, cx| {
                    if view.hovered_tab != *hovered {
                        view.hovered_tab = *hovered;
                        cx.notify();
                    }
                },
            )),
            close_hover: Rc::new(cx.listener(|view, state: &CloseHover, _window, cx| {
                let next = if state.hovered {
                    Some(state.index)
                } else {
                    None
                };
                if view.hovered_close != next {
                    view.hovered_close = next;
                    cx.notify();
                }
            })),
            // WIN-002:标题栏空白处拖动窗口。
            //
            // 按下只记录起点;真正的发起交给窗口级移动监听(阈值判定),原因见该处注释。
            drag_pressed: Rc::new(cx.listener(|view, origin: &Point<Pixels>, _window, cx| {
                view.drag_pending = Some(*origin);
                cx.notify();
            })),
        };

        // 标签条外面套一层 canvas,只是为了拿到一个**paint 阶段**的钩子。
        //
        // window.on_mouse_event 只能在 paint 期间调用(gpui 0.2.2 window.rs:3425
        // 的 debug_assert_paint),写在 render 里会直接 panic。
        // view.rs 的选区监听也是同样做法(画布 paint 闭包内注册)。
        //
        // 拖动判定为什么需要窗口级监听:元素级 on_mouse_move 带 hitbox.is_hovered
        // 过滤(div.rs:269),指针一移出标签条就收不到事件,拖动发不起来。
        // 为什么不在按下时直接发起:Windows 的 begin_window_drag 内部
        // SendMessageW(WM_NCLBUTTONDOWN) 会阻塞到用户松开(term_platform/win32.rs:160),
        // 那样双击空白新建标签(MOUSE-005)的第二下永远等不到。
        let drag_entity = cx.entity();
        let drag_hook = canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, (), window, _cx| {
                // 移动:超过阈值才发起拖动
                let move_entity = drag_entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    let pressed = event.pressed_button == Some(MouseButton::Left);
                    let position = event.position;
                    let should_drag = move_entity.update(cx, |view, _| {
                        let Some(origin) = view.drag_pending else {
                            return false;
                        };
                        if !pressed {
                            view.drag_pending = None;
                            return false;
                        }
                        let dx: f32 = f32::from(position.x) - f32::from(origin.x);
                        let dy: f32 = f32::from(position.y) - f32::from(origin.y);
                        if dx * dx + dy * dy < DRAG_THRESHOLD * DRAG_THRESHOLD {
                            return false;
                        }
                        // 超过阈值:清掉待拖动状态,交给系统拖动循环(阻塞到用户松开)
                        view.drag_pending = None;
                        true
                    });
                    if should_drag {
                        crate::view::start_window_drag(window);
                    }
                });

                // 抬起:清除待拖动状态。
                // 不清的话,空白处按下又原地松手会留下过期起点,
                // 之后拖标签会被误判成拖窗口。
                let up_entity = drag_entity.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _window, cx| {
                    if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    up_entity.update(cx, |view, _| view.drag_pending = None);
                });
            },
        )
        // 只借用它的 paint 时机,不参与布局:绝对定位 + 0 尺寸,
        // 否则它会作为 flex 兄弟节点占满剩余宽度,把标签条挤走。
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .w(px(0.0))
        .h(px(0.0));

        let tabbar = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(render_tabbar(&items, hovered_tab, handlers))
            .child(drag_hook)
            .into_any_element();

        let titlebar = render_titlebar(
            &TitlebarModel {
                title,
                maximized: window.is_maximized(),
                focused: window.is_window_active(),
                controls: WindowControlsShown {
                    minimize: true,
                    maximize: true,
                },
                hovered: self.hovered_control,
            },
            Some(tabbar),
            TitlebarHandlers {
                drag: Rc::new(cx.listener(
                    |_view, _event: &MouseDownEvent, window: &mut Window, _cx| {
                        crate::view::start_window_drag(window);
                    },
                )),
                double_click: Rc::new(cx.listener(
                    |_view, _event: &ClickEvent, window: &mut Window, _cx| {
                        crate::view::toggle_maximize(window);
                    },
                )),
                control: Rc::new(cx.listener(
                    |_view, kind: &WindowControl, window: &mut Window, _cx| {
                        crate::view::apply_window_control(*kind, window);
                    },
                )),
                hover: Rc::new(cx.listener(
                    |view, hovered: &Option<WindowControl>, _window, cx| {
                        if view.hovered_control != *hovered {
                            view.hovered_control = *hovered;
                            cx.notify();
                        }
                    },
                )),
            },
        );

        // 只渲染激活标签:隐藏的标签不需要布局与绘制,PTY 仍在后台运行。
        //
        // chrome 高度在渲染**之前**通过 defer 同步给激活标签:窗口尺寸变化时靠它
        // 重算网格行数,否则从别的标签切回来会沿用旧行数,最后一行被标签条遮住(WIN-007)。
        // 不在这里直接 update 子实体:渲染期间修改其它实体状态可能触发嵌套更新。
        let body: AnyElement = match self.active_tab().cloned() {
            Some(tab) => {
                let chrome = tab.clone();
                cx.defer(move |cx| {
                    chrome.update(cx, |tab, cx| tab.set_chrome_height(TITLEBAR_HEIGHT, cx));
                });
                tab.into_any_element()
            }
            None => div().into_any_element(),
        };

        // 标题栏(含标签条与窗口按钮)由本视图绘制,因此**必须**用实测客户区尺寸显式
        // 设置宽高,不能用 size_full():窗口「最大化 -> 最小化 -> 还原」后 gpui 的
        // viewport_size 可能仍停留在最大化时的大小,按它排布会把整组窗口按钮推到窗口外
        // (用户实测:三个按钮消失,点别的窗口才恢复)。终端内容区用的是同一套校正值。
        let layout = crate::view::layout_size(window);

        div()
            .id("tabs-root")
            .key_context("TabsRoot")
            .on_action(cx.listener(|view, _: &NewTab, _window, cx| view.new_tab(cx)))
            .on_action(cx.listener(|view, _: &CloseActiveTab, window, cx| {
                view.close_tab_or_window(view.active, window, cx)
            }))
            .on_action(cx.listener(|view, _: &NextTab, window, cx| view.next_tab(window, cx)))
            .on_action(cx.listener(|view, _: &PrevTab, window, cx| view.prev_tab(window, cx)))
            .flex()
            .flex_col()
            .w(px(layout.0))
            .h(px(layout.1))
            .bg(rgb(window_theme::WINDOW_BG))
            .child(titlebar)
            // 新建标签失败等可恢复错误显示在界面底部,不打断使用
            .children(status.map(|text| {
                div()
                    .flex_shrink_0()
                    .h(px(20.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .bg(rgb(window_theme::TITLEBAR_BG))
                    .text_xs()
                    .text_color(rgb(window_theme::TITLE_TEXT_INACTIVE))
                    .child(text)
            }))
            .child(div().flex_1().overflow_hidden().child(body))
    }
}

#[cfg(test)]
mod tests {
    /// 测试用最小状态:只保留下标运算需要的那部分视图状态。
    ///
    /// 真实视图需要 GPUI 上下文与 PTY,这些行为由手工验收覆盖;
    /// 这里验证的是"关闭/切换后激活下标指向哪个标签"这类纯算术(TESTING.md 第 2 节)。
    struct IndexState {
        len: usize,
        active: usize,
    }

    impl IndexState {
        fn new(len: usize, active: usize) -> Self {
            Self { len, active }
        }

        /// 与 TabsView::close_tab 的索引维护逐行对应。
        fn close(&mut self, index: usize) {
            if index >= self.len {
                return;
            }
            self.len -= 1;
            if self.len == 0 {
                self.active = 0;
            } else if index < self.active {
                self.active -= 1;
            } else if index == self.active {
                self.active = self.active.min(self.len - 1);
            }
        }

        fn next(&mut self) {
            if self.len < 2 {
                return;
            }
            self.active = (self.active + 1) % self.len;
        }

        fn prev(&mut self) {
            if self.len < 2 {
                return;
            }
            self.active = (self.active + self.len - 1) % self.len;
        }
    }

    /// TAB-005:关闭当前标签后,激活右邻居;已是最后一个则退回新的最后一个
    #[test]
    fn tab_005_closing_active_selects_right_neighbor() {
        let mut s = IndexState::new(3, 1);
        s.close(1);
        assert_eq!((s.len, s.active), (2, 1), "关闭中间标签后应激活原右邻居");

        let mut s = IndexState::new(3, 2);
        s.close(2);
        assert_eq!(
            (s.len, s.active),
            (2, 1),
            "关闭末尾标签后应退回新的最后一个"
        );
    }

    /// TAB-006:关闭非激活标签不改变当前激活的那个标签
    #[test]
    fn tab_006_closing_inactive_keeps_same_tab_active() {
        let mut s = IndexState::new(3, 2);
        s.close(0);
        assert_eq!((s.len, s.active), (2, 1), "左边的标签关掉后,激活下标应左移");

        let mut s = IndexState::new(4, 1);
        s.close(3);
        assert_eq!((s.len, s.active), (3, 1), "右边的标签关掉后,激活下标不变");
    }

    /// TAB-007:关闭最后一个标签后列表为空(由调用方关闭窗口)
    #[test]
    fn tab_007_closing_last_tab_empties_list() {
        let mut s = IndexState::new(1, 0);
        s.close(0);
        assert_eq!(s.len, 0);
    }

    /// TAB-003/TAB-004:切换在两端环绕
    #[test]
    fn tab_003_next_prev_wrap_around() {
        let mut s = IndexState::new(3, 2);
        s.next();
        assert_eq!(s.active, 0, "最后一个的下一个应是第一个");
        s.prev();
        assert_eq!(s.active, 2, "第一个的上一个应是最后一个");
    }

    /// 只有一个标签时切换是空操作(不应改变激活下标或崩溃)
    #[test]
    fn tab_004_switching_with_single_tab_is_noop() {
        let mut s = IndexState::new(1, 0);
        s.next();
        assert_eq!(s.active, 0);
        s.prev();
        assert_eq!(s.active, 0);
    }

    /// 越界关闭被忽略(防御性:例如悬停状态与列表不同步)
    #[test]
    fn closing_out_of_range_is_ignored() {
        let mut s = IndexState::new(2, 0);
        s.close(5);
        assert_eq!((s.len, s.active), (2, 0));
    }

    /// TAB-011:标签上的关闭图标走与中键/Ctrl+Shift+W 相同的关闭路径。
    ///
    /// 三个入口都映射到 `TabsView::close_tab_or_window`,因此索引维护的预期结果
    /// 与 `IndexState::close` 一致;这里固定住"点关闭图标 = 关闭该标签"这一语义,
    /// 避免将来把关闭图标误接到"切换"或"关闭当前标签"上。
    #[test]
    fn tab_011_close_icon_closes_its_own_tab() {
        // 三个标签、激活第 0 个:点第 2 个标签的关闭图标应关掉第 2 个,激活不变
        let mut s = IndexState::new(3, 0);
        s.close(2);
        assert_eq!((s.len, s.active), (2, 0), "关闭非激活标签不应改变激活标签");

        // 激活中间那个:点它的关闭图标后应激活右邻居
        let mut s = IndexState::new(3, 1);
        s.close(1);
        assert_eq!((s.len, s.active), (2, 1), "关闭激活标签应激活右邻居");
    }

    /// TAB-012:关闭图标在只有一个标签时等价于关闭窗口(列表变空,由调用方关窗)
    #[test]
    fn tab_012_close_icon_on_last_tab_empties_list() {
        let mut s = IndexState::new(1, 0);
        s.close(0);
        assert_eq!(s.len, 0, "最后一个标签关闭后列表为空,调用方据此关闭窗口");
    }
}
