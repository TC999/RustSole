//! 标签条:标题栏一行内的标签列表与「新建标签」按钮。
//!
//! 标题栏 = 标签条(左,可伸缩) + 窗口控制按钮(右,固定宽)。没有把标签条单独放一行,
//! 是因为本项目使用客户端装饰自绘标题栏,空出一整行只为显示窗口标题并不划算;
//! 同一行里标签本身就是窗口标识。
//!
//! 与 titlebar.rs 一样,这里只负责绘制与回调转发,不持有任何业务状态。

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, rgb, App, Hsla, IntoElement, MouseButton, SharedString, Window};

use crate::theme::{tabbar, NEW_TAB_BUTTON_WIDTH, TAB_MAX_WIDTH, TAB_MIN_WIDTH, TITLEBAR_HEIGHT};

/// 一个标签在标签条上的呈现数据(全部来自视图状态,不含 gpui 句柄)。
#[derive(Debug, Clone)]
pub struct TabItem {
    /// 标签标题。子进程未通过 OSC 0/2 设置标题时沿用入口给的默认标题。
    pub title: SharedString,
    /// 是否处于激活状态(决定底色与文字颜色)。
    pub active: bool,
    /// 子进程是否已退出(文字变暗提示)。
    pub exited: bool,
}

/// 标签条上可被点击的目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabHit {
    /// 第 index 个标签。
    Select(usize),
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
    /// 悬停的标签变化。
    pub hover: TabHoverHandler,
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
        .flex_shrink_0();

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

    // 空白区:吃掉标签与「+」以外的宽度。中键新建、双击新建(MOUSE-004/005)。
    let middle = Rc::clone(&handlers.middle);
    let double_click = Rc::clone(&handlers.double_click_blank);
    let hover = Rc::clone(&handlers.hover);
    let blank = div()
        .id("tabbar-blank")
        .flex_1()
        .h_full()
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

    div()
        .id("tabbar")
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
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

    div()
        .id(("tab", index))
        .flex()
        .items_center()
        .justify_center()
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
        .child(
            div()
                .px(px(10.0))
                .text_sm()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(text_color)
                .child(item.title.clone()),
        )
        .into_any_element()
}
