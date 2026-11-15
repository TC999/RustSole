//! 应用工具栏:位于标签条与终端内容之间的一行操作按钮。
//!
//! 布局参照 Konsole 默认布局:左侧是窗口级操作(新建标签页、拆分视图),
//! 右侧是终端级操作(复制、粘贴、查找、更多菜单)。
//!
//! 与 titlebar.rs / tabbar.rs 一样,本模块只负责绘制与回调转发,不持有业务状态:
//! 按钮点了做什么由 `TabsView` 决定(ARCHITECTURE.md 不变量 5:所有用户操作走 Action)。
//!
//! 图标全部用 `div` 的边框与文字自绘:项目未注册 `AssetSource`,`svg()` 取不到资源;
//! canvas 线条绘制在窗口最小化恢复后可能丢失,常规元素不会(见 titlebar.rs 的说明)。

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, rgb, App, Hsla, IntoElement, Pixels, Point, Window};

use crate::theme::{
    toolbar, TOOLBAR_BUTTON_HEIGHT, TOOLBAR_BUTTON_PADDING, TOOLBAR_HEIGHT, TOOLBAR_ICON_SIZE,
};

/// 工具栏上的按钮种类。
///
/// 顺序与 Konsole 一致:左区(新建标签页、拆分视图) → 右区(复制、粘贴、查找、更多)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarButton {
    /// 新建标签页(等价于标签条上的「+」)。
    NewTab,
    /// 拆分视图:分屏功能属阶段 3,本阶段置灰不可用。
    SplitView,
    /// 复制:把当前选区复制到系统剪贴板(KEY-001)。
    Copy,
    /// 粘贴:把系统剪贴板内容粘贴到终端(KEY-002)。
    Paste,
    /// 查找:搜索栏属阶段 4,本阶段置灰不可用。
    Find,
    /// 更多菜单。
    More,
}

/// 一个工具栏按钮的呈现数据。
#[derive(Debug, Clone, Copy)]
pub struct ToolbarItem {
    /// 按钮种类。
    pub kind: ToolbarButton,
    /// 是否可用。不可用的按钮置灰且不响应点击。
    pub enabled: bool,
    /// 指针是否悬停在该按钮上(决定高亮)。
    pub hovered: bool,
}

/// 工具栏按钮点击回调(参数与 `Context::listener` 的产物签名一致)。
pub type ToolbarClickHandler = Rc<dyn Fn(&ToolbarButton, &mut Window, &mut App)>;
/// 工具栏按钮悬停回调(Some = 某个按钮,None = 不在任何按钮上)。
pub type ToolbarHoverHandler = Rc<dyn Fn(&Option<ToolbarButton>, &mut Window, &mut App)>;
/// 「更多」按钮点击回调:参数是点击位置(窗口坐标),用于定位弹出菜单。
pub type ToolbarMoreHandler = Rc<dyn Fn(&Point<Pixels>, &mut Window, &mut App)>;

/// 工具栏回调。
pub struct ToolbarHandlers {
    /// 某个按钮被点击。只有可用按钮才会触发。
    pub click: ToolbarClickHandler,
    /// 悬停的按钮变化(None = 不在任何按钮上)。
    pub hover: ToolbarHoverHandler,
    /// 「更多」按钮被点击:参数是点击位置(窗口坐标),用于定位弹出菜单。
    pub more: ToolbarMoreHandler,
}

/// 渲染应用工具栏。
///
/// 左侧按钮组靠左,右侧按钮组靠右,中间留空:与 Konsole 的分组一致。
/// 整行不参与纵向伸缩,高度固定为 [`TOOLBAR_HEIGHT`]。
pub fn render_toolbar(items: &[ToolbarItem], handlers: ToolbarHandlers) -> impl IntoElement {
    let mut left = div().flex().flex_row().items_center().gap_2();
    let mut right = div().flex().flex_row().items_center().gap_2();

    for item in items {
        let element = toolbar_button(item, &handlers);
        // 分组:NewTab 与 SplitView 在左,其余在右。
        match item.kind {
            ToolbarButton::NewTab | ToolbarButton::SplitView => left = left.child(element),
            _ => right = right.child(element),
        }
    }

    div()
        .id("toolbar")
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(TOOLBAR_HEIGHT))
        .w_full()
        .px(px(TOOLBAR_BUTTON_PADDING))
        .gap_2()
        .bg(rgb(toolbar::BG))
        .border_b_1()
        .border_color(rgb(toolbar::BORDER))
        .child(left)
        .child(div().flex_1().min_w_0())
        .child(right)
}

/// 单个工具栏按钮。
///
/// 不可用按钮不注册点击处理:置灰只是视觉提示,不应被误触发。
fn toolbar_button(item: &ToolbarItem, handlers: &ToolbarHandlers) -> gpui::AnyElement {
    let kind = item.kind;
    let enabled = item.enabled;
    let hovered = item.hovered && enabled;

    let icon_color: Hsla = if enabled {
        rgb(toolbar::ICON).into()
    } else {
        rgb(toolbar::ICON_DISABLED).into()
    };
    let bg: Hsla = if hovered {
        rgb(toolbar::HOVER_BG).into()
    } else {
        // 未悬停时保持工具栏底色,不画独立色块。
        rgb(toolbar::BG).into()
    };

    let mut button = div()
        // ElementId 不接受 (&str, &str):用数字后缀区分同类按钮的不同实例。
        .id(("toolbar-button", kind_index(kind)))
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(TOOLBAR_BUTTON_HEIGHT))
        .px(px(TOOLBAR_BUTTON_PADDING))
        .rounded(px(4.0))
        .bg(bg);

    if enabled {
        let click = Rc::clone(&handlers.click);
        button = button
            .cursor_pointer()
            .on_click(move |_event, window, cx| (click)(&kind, window, cx));
    }
    // 「更多」按钮:点击位置要传给上层,菜单才能贴着按钮展开。
    // 它与普通按钮分开处理:菜单锚点需要事件里的坐标,普通按钮不需要。
    if kind == ToolbarButton::More {
        let more = Rc::clone(&handlers.more);
        button = button.on_click(move |event, window, cx| (more)(&event.position(), window, cx));
    }

    // 悬停状态由视图持有:按钮只负责上报"鼠标在/不在我上面"。
    // 不可用按钮也上报悬停,置灰按钮同样需要"指针在上"的反馈,
    // 只是不给出可点击的高亮底色。
    let hover = Rc::clone(&handlers.hover);
    button = button.on_hover(move |&is_hovered, window, cx| {
        let hovered = if is_hovered { Some(kind) } else { None };
        (hover)(&hovered, window, cx);
    });

    button
        .child(button_icon(kind, icon_color))
        .child(button_label(kind, enabled))
        .into_any_element()
}

/// 按钮的序号,用于构造元素 id。
///
/// gpui 的 `ElementId` 只支持 `(&str, u32)` 之类的组合,不能直接用两个字符串,
/// 因此这里给每个按钮一个稳定序号。
fn kind_index(kind: ToolbarButton) -> u32 {
    match kind {
        ToolbarButton::NewTab => 0,
        ToolbarButton::SplitView => 1,
        ToolbarButton::Copy => 2,
        ToolbarButton::Paste => 3,
        ToolbarButton::Find => 4,
        ToolbarButton::More => 5,
    }
}

/// 按钮图标。
///
/// 用边框与文字拼出 Konsole 风格的线性图标:
/// - 新建标签页:一个带「+」的窗口轮廓
/// - 拆分视图:一个被竖线分成两半的窗口轮廓
/// - 复制:两张重叠的纸片
/// - 粘贴:一张带折角的纸片
/// - 查找:放大镜
/// - 更多:三横线
fn button_icon(kind: ToolbarButton, color: Hsla) -> gpui::AnyElement {
    let size = px(TOOLBAR_ICON_SIZE);
    match kind {
        ToolbarButton::NewTab => div()
            .relative()
            .w(size)
            .h(size)
            .border_1()
            .border_color(color)
            .rounded(px(2.0))
            // 左上角一个「+」:两条短线。
            .child(plus_glyph(color))
            .into_any_element(),
        ToolbarButton::SplitView => div()
            .relative()
            .w(size)
            .h(size)
            .border_1()
            .border_color(color)
            .rounded(px(2.0))
            // 中间一条竖线:左右分栏。
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .bottom(px(0.0))
                    .left(px(TOOLBAR_ICON_SIZE / 2.0))
                    .w(px(1.0))
                    .bg(color),
            )
            .into_any_element(),
        ToolbarButton::Copy => div()
            .relative()
            .w(size)
            .h(size)
            // 后一张纸片(实心描边)。
            .child(
                div()
                    .absolute()
                    .top(px(4.0))
                    .left(px(5.0))
                    .w(px(10.0))
                    .h(px(11.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(1.0)),
            )
            // 前一张纸片(用更细的边框位置错开)。
            .child(
                div()
                    .absolute()
                    .top(px(1.0))
                    .left(px(1.0))
                    .w(px(10.0))
                    .h(px(11.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(1.0)),
            )
            .into_any_element(),
        ToolbarButton::Paste => div()
            .relative()
            .w(size)
            .h(size)
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(2.0))
                    .w(px(12.0))
                    .h(px(13.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(1.0)),
            )
            // 折角:小三角画不出来,改用短横线示意。
            .child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .left(px(6.0))
                    .w(px(6.0))
                    .h(px(1.0))
                    .bg(color),
            )
            .into_any_element(),
        ToolbarButton::Find => div()
            .relative()
            .w(size)
            .h(size)
            // 放大镜镜片:布局不支持圆形,只能用圆角近似。
            .child(
                div()
                    .absolute()
                    .top(px(1.0))
                    .left(px(1.0))
                    .w(px(10.0))
                    .h(px(10.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(5.0)),
            )
            // 手柄。
            .child(
                div()
                    .absolute()
                    .top(px(11.0))
                    .left(px(11.0))
                    .w(px(5.0))
                    .h(px(2.0))
                    .bg(color)
                    .rounded(px(1.0)),
            )
            .into_any_element(),
        ToolbarButton::More => div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.0))
            .w(size)
            .h(size)
            .child(bar(color))
            .child(bar(color))
            .child(bar(color))
            .into_any_element(),
    }
}

/// 一条水平短线(放大镜的「+」与汉堡菜单的三横线共用)。
fn bar(color: Hsla) -> gpui::AnyElement {
    div()
        .w(px(TOOLBAR_ICON_SIZE))
        .h(px(1.5))
        .bg(color)
        .rounded(px(1.0))
        .into_any_element()
}

/// 「新建标签页」图标里的加号:一横一竖。
fn plus_glyph(color: Hsla) -> gpui::AnyElement {
    div()
        .absolute()
        .top(px(3.0))
        .left(px(3.0))
        .w(px(8.0))
        .h(px(8.0))
        .child(
            div()
                .absolute()
                .top(px(3.5))
                .left(px(0.0))
                .w(px(8.0))
                .h(px(1.5))
                .bg(color),
        )
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(3.5))
                .w(px(1.5))
                .h(px(8.0))
                .bg(color),
        )
        .into_any_element()
}

/// 按钮文字。
///
/// Konsole 的工具栏按钮带文字说明。这里跟随同样的做法:文字让「拆分视图」等
/// 尚未实现的按钮不至于含义不明。不可用按钮的文字同样置灰。
fn button_label(kind: ToolbarButton, enabled: bool) -> gpui::AnyElement {
    let text = match kind {
        ToolbarButton::NewTab => "新建标签页",
        ToolbarButton::SplitView => "拆分视图",
        ToolbarButton::Copy => "复制",
        ToolbarButton::Paste => "粘贴",
        ToolbarButton::Find => "查找",
        ToolbarButton::More => "",
    };
    if text.is_empty() {
        return div().into_any_element();
    }
    let color: Hsla = if enabled {
        rgb(toolbar::LABEL).into()
    } else {
        rgb(toolbar::ICON_DISABLED).into()
    };
    div()
        .text_sm()
        .whitespace_nowrap()
        .text_color(color)
        .child(text)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 工具栏按钮顺序固定:新建、拆分 → 复制、粘贴、查找、更多。
    #[test]
    fn toolbar_button_order_is_stable() {
        assert_eq!(kind_index(ToolbarButton::NewTab), 0);
        assert_eq!(kind_index(ToolbarButton::SplitView), 1);
        assert_eq!(kind_index(ToolbarButton::Copy), 2);
        assert_eq!(kind_index(ToolbarButton::Paste), 3);
        assert_eq!(kind_index(ToolbarButton::Find), 4);
        assert_eq!(kind_index(ToolbarButton::More), 5);
    }
}
