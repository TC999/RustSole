//! 标签条:标题栏一行内的标签列表与「新建标签」按钮。
//!
//! 标题栏 = 标签条(左,可伸缩) + 窗口控制按钮(右,固定宽)。没有把标签条单独放一行,
//! 是因为本项目使用客户端装饰自绘标题栏,空出一整行只为显示窗口标题并不划算;
//! 同一行里标签本身就是窗口标识。
//!
//! 与 titlebar.rs 一样,这里只负责绘制与回调转发,不持有任何业务状态。

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, App, Hsla, IntoElement, MouseButton, Pixels, Point, SharedString, Window,
};

use crate::theme::{
    tabbar, NEW_TAB_BUTTON_WIDTH, TAB_CLOSE_WIDTH, TAB_MAX_WIDTH, TAB_MIN_WIDTH, TITLEBAR_HEIGHT,
};

/// 一个标签在标签条上的呈现数据(全部来自视图状态,不含 gpui 句柄)。
#[derive(Debug, Clone)]
pub struct TabItem {
    /// 标签标题。子进程未通过 OSC 0/2 设置标题时沿用入口给的默认标题。
    pub title: SharedString,
    /// 是否处于激活状态(决定底色与文字颜色)。
    pub active: bool,
    /// 子进程是否已退出(文字变暗提示)。
    pub exited: bool,
    /// 指针是否正悬停在该标签的关闭图标上(决定图标高亮)。
    pub close_hovered: bool,
}

/// 标签条上可被点击的目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabHit {
    /// 第 index 个标签(切换)。
    Select(usize),
    /// 第 index 个标签上的关闭图标。
    Close(usize),
    /// 「新建标签」按钮。
    NewTab,
}

/// 点击回调(参数与 `Context::listener` 的产物签名一致)。
type TabClickHandler = Rc<dyn Fn(&TabHit, &mut Window, &mut App)>;
/// 中键回调。
type TabMiddleHandler = Rc<dyn Fn(&Option<usize>, &mut Window, &mut App)>;
/// 双击回调。
pub type TabDoubleClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;
/// 悬停回调(Some = 某个标签,None = 不在任何标签上)。
type TabHoverHandler = Rc<dyn Fn(&Option<usize>, &mut Window, &mut App)>;

/// 关闭图标悬停回调。
type TabCloseHoverHandler = Rc<dyn Fn(&CloseHover, &mut Window, &mut App)>;
/// 空白区左键按下回调:记录拖动起点(坐标)。
pub type TabDragPressedHandler = Rc<dyn Fn(&Point<Pixels>, &mut Window, &mut App)>;

/// 关闭图标的悬停状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseHover {
    /// 标签下标。
    pub index: usize,
    /// 指针是否在关闭图标上。
    pub hovered: bool,
}

/// 标签条回调。
///
/// 中键与双击单独给回调,而不是塞进 click:on_click 只覆盖左键,
/// MOUSE-004(中键关标签 / 中键空白新建)与 MOUSE-005(双击空白新建)需要各自的入口。
pub struct TabbarHandlers {
    /// 左键点击某个标签或「新建标签」按钮。
    pub click: TabClickHandler,
    /// 中键按下:Some(index) = 该标签,None = 标签条空白处。
    pub middle: TabMiddleHandler,
    /// 标签条空白处双击。
    pub double_click_blank: TabDoubleClickHandler,
    /// 标签条空白处按下左键:记录可能的拖动起点(WIN-002)。
    /// 位移超过阈值后由视图真正发起拖动。
    pub drag_pressed: TabDragPressedHandler,
    /// 悬停的标签变化。
    pub hover: TabHoverHandler,
    /// 某个标签的关闭图标悬停变化。
    pub close_hover: TabCloseHoverHandler,
}

/// 渲染标签条(标题栏左半部分,占据窗口控制按钮以外的宽度)。
///
/// 每个标签以 TAB_MIN_WIDTH 为 flex_basis、flex_grow = 1、max_w = TAB_MAX_WIDTH:
/// 标签少时等分到最大宽度为止,标签多时不低于最小宽度,超出部分由 overflow_hidden
/// 裁掉(不换行,避免标签条把终端挤走一整行)。
pub fn render_tabbar(
    items: &[TabItem],
    hovered: Option<usize>,
    handlers: TabbarHandlers,
) -> impl IntoElement {
    let mut strip = div()
        .id("tab-strip")
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        // 允许收缩:标签多到放不下时压窄标签(标签自身 min_w 兜底),
        // 把空间留给右侧的拖动区。
        .flex_shrink()
        .min_w_0();

    for (index, item) in items.iter().enumerate() {
        strip = strip.child(tab_element(index, item, hovered == Some(index), &handlers));
    }

    let click = Rc::clone(&handlers.click);
    let new_tab_button = div()
        .id("tab-new")
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .w(px(NEW_TAB_BUTTON_WIDTH))
        .h_full()
        .cursor_pointer()
        .on_click(move |_event, window, cx| (click)(&TabHit::NewTab, window, cx))
        .child(
            div()
                .text_size(px(14.0))
                .text_color(rgb(tabbar::NEW_TAB_ICON))
                .child("+"),
        );

    // 空白区:标签与「+」以外的全部宽度。
    //
    // 同时承担两件事:窗口拖动(WIN-002)与双击新建标签(MOUSE-005)。
    // 二者不能都在鼠标按下时完成 —— Windows 的 begin_window_drag 内部是
    // SendMessageW(WM_NCLBUTTONDOWN),会**同步阻塞**到用户松开鼠标
    // (见 term_platform/win32.rs:160),若在 mouse_down 就发起拖动,
    // 双击的第二下永远等不到,双击新建会失效。
    //
    // 因此拖动改为"按下后移动超过阈值才发起":单击/双击走点击路径,
    // 按住并移动走拖动路径,两者互不吞并。
    //
    // 移动判定必须用**窗口级**监听:元素级 on_mouse_move 带 hitbox.is_hovered
    // 过滤(div.rs:269),指针一旦移出空白区就再也收不到事件,拖动永远发不起来。
    let middle = Rc::clone(&handlers.middle);
    let double_click = Rc::clone(&handlers.double_click_blank);
    let hover = Rc::clone(&handlers.hover);
    let press = Rc::clone(&handlers.drag_pressed);

    let blank = div()
        .id("tabbar-blank")
        .flex_1()
        .min_w_0()
        .h_full()
        // 左键按下只记录"可能的拖动起点",不立即发起拖动:
        // 立即发起会阻塞到鼠标松开(Windows 的 SendMessageW),把双击新建吞掉。
        // 真正的发起由 TabsView 的窗口级移动监听在位移超过阈值时完成。
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            (press)(&event.position, window, cx)
        })
        .on_mouse_down(MouseButton::Middle, move |_event, window, cx| {
            (middle)(&None, window, cx)
        })
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                (double_click)(window, cx);
            }
        })
        .on_hover(move |&is_hovered, window, cx| {
            // 进入空白区说明指针已离开所有标签
            if is_hovered {
                (hover)(&None, window, cx);
            }
        });

    // 布局要点:标签区(strip + 「+」)按内容宽度排布且**不参与拉伸**,
    // 剩余宽度全部让给空白拖动区。这样标签少时拖动区很宽,标签多时先挤压标签
    // 宽度(标签自身 flex_shrink),而不是把拖动区的最后一点空间也吃掉——
    // 否则标签一多窗口就再也拖不动。
    div()
        .id("tabbar")
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
        .min_w_0()
        .h(px(TITLEBAR_HEIGHT))
        .overflow_hidden()
        .bg(rgb(tabbar::BG))
        .child(strip)
        .child(new_tab_button)
        .child(blank)
}

/// 单个标签。
fn tab_element(
    index: usize,
    item: &TabItem,
    hovered: bool,
    handlers: &TabbarHandlers,
) -> gpui::AnyElement {
    let close_hovered = item.close_hovered;
    let (bg, text_color): (Hsla, Hsla) = if item.active {
        (
            rgb(tabbar::ACTIVE_BG).into(),
            rgb(tabbar::ACTIVE_TEXT).into(),
        )
    } else if item.exited {
        (rgb(tabbar::BG).into(), rgb(tabbar::EXITED_TEXT).into())
    } else if hovered {
        (
            rgb(tabbar::HOVER_BG).into(),
            rgb(tabbar::ACTIVE_TEXT).into(),
        )
    } else {
        (rgb(tabbar::BG).into(), rgb(tabbar::INACTIVE_TEXT).into())
    };

    let click = Rc::clone(&handlers.click);
    let middle = Rc::clone(&handlers.middle);
    let hover = Rc::clone(&handlers.hover);

    // 关闭图标的配色:悬停在图标上时才高亮。
    // 是否悬停由视图状态决定并作为参数传入(与标签悬停同一套机制),
    // 这样重绘时机与标签高亮一致,不需要额外的共享可变状态。
    let (close_bg, close_color): (Hsla, Hsla) = if close_hovered {
        (
            rgb(tabbar::CLOSE_ICON_HOVER_BG).into(),
            rgb(tabbar::CLOSE_ICON_HOVER).into(),
        )
    } else {
        (rgb(tabbar::BG).into(), rgb(tabbar::CLOSE_ICON).into())
    };

    // 关闭图标:放在标签右端。
    //
    // 关键点是 occlude():它把该 hitbox 标记为 BlockMouse,命中测试遇到它即停止
    // (gpui 0.2.2 window.rs:788-790),外层标签的 hitbox 因此不在命中集合里,
    // 标签的 on_click(切换)不会跟着触发。没有它,点关闭会先切到该标签再关闭,
    // 表现为"点关闭时画面闪一下"。
    let close_click = Rc::clone(&handlers.click);
    let close_hover = Rc::clone(&handlers.close_hover);
    let close_button = div()
        .id(("tab-close", index))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .w(px(TAB_CLOSE_WIDTH))
        .h_full()
        .occlude()
        .cursor_pointer()
        .on_click(move |_event, window, cx| (close_click)(&TabHit::Close(index), window, cx))
        .on_hover(move |&is_hovered, window, cx| {
            (close_hover)(
                &CloseHover {
                    index,
                    hovered: is_hovered,
                },
                window,
                cx,
            );
        })
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                // 常态不显示底色,悬停时用圆形底色提示可点(见下方 on_hover)
                .rounded(px(3.0))
                .bg(close_bg)
                .text_size(px(12.0))
                .text_color(close_color)
                .child("✕"),
        );

    div()
        .id(("tab", index))
        .flex()
        .flex_row()
        .items_center()
        // 左侧分隔线:与 titlebar.rs 一致,用 border 而不是额外元素,不影响布局宽度
        .border_l_1()
        .border_color(rgb(tabbar::SEPARATOR))
        .h_full()
        .flex_basis(px(TAB_MIN_WIDTH))
        .flex_grow()
        .flex_shrink()
        .min_w(px(TAB_MIN_WIDTH))
        .max_w(px(TAB_MAX_WIDTH))
        .overflow_hidden()
        .bg(bg)
        .cursor_pointer()
        .on_click(move |_event, window, cx| (click)(&TabHit::Select(index), window, cx))
        .on_mouse_down(MouseButton::Middle, move |_event, window, cx| {
            (middle)(&Some(index), window, cx)
        })
        .on_hover(move |&is_hovered, window, cx| {
            if is_hovered {
                (hover)(&Some(index), window, cx);
            }
        })
        // 标题占满图标以外的宽度;min_w_0 让长标题截断而不是把关闭图标挤出标签
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl(px(10.0))
                .text_sm()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(text_color)
                .child(item.title.clone()),
        )
        .child(close_button)
        .into_any_element()
}
