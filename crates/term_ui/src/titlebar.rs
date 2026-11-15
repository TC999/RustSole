//! 自绘标题栏(客户端窗口装饰):标题文字 + 拖动区 + 最小化/最大化/关闭按钮。
//!
//! 为什么自己画:入口用 `titlebar.appears_transparent = true` 隐藏系统标题栏后,
//! Windows 与 Linux 都不会再提供系统按钮,必须由应用绘制(WIN-001..WIN-006)。
//!
//! 两平台分工不同,由 `term_platform::window_chrome()` 决定(硬性规则 6:
//! `cfg(target_os)` 只允许出现在 `term_platform`):
//! - `SystemHandled`(Windows):gpui 把 `WindowControlArea` 命中框映射为
//!   HTCAPTION/HTMINBUTTON/HTMAXBUTTON/HTCLOSE,拖动、双击最大化、按钮动作都由系统完成
//!   (gpui 0.2.2 `platform/windows/events.rs:868-878`、`window.rs:1133-1148`)。
//!   应用此时**只绘制**,不再执行动作,否则最大化会被切换两次。
//! - `AppHandled`(X11/Wayland):gpui 的 `on_hit_test_window_control` 是空实现,
//!   应用自己调用 `Window::start_window_move` / `minimize_window` / `zoom_window` /
//!   `remove_window`。
//!
//! 图标用 `canvas` + `PathBuilder` 自绘:项目未注册 `AssetSource`,`svg()` 取不到资源。

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, App, ClickEvent, Hsla, IntoElement, MouseButton, MouseDownEvent, SharedString,
    Window,
};

use crate::theme::{
    window, WindowControl, WindowControlsShown, TITLEBAR_HEIGHT, WINDOW_CONTROL_WIDTH,
};

/// 标题栏数据(全部来自视图状态,不含 gpui 句柄)。
#[derive(Debug, Clone)]
pub struct TitlebarModel {
    /// 标题文字。
    pub title: SharedString,
    /// 窗口是否最大化(决定最大化按钮显示"还原"图标)。
    pub maximized: bool,
    /// 窗口是否处于激活状态(决定标题文字颜色)。
    pub focused: bool,
    /// 平台支持哪些窗口控制按钮。
    pub controls: WindowControlsShown,
    /// 当前悬停的按钮(决定高亮)。
    pub hovered: Option<WindowControl>,
}

/// 无事件数据的回调(双击)。
type SimpleHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
/// 带控制按钮参数的回调(按钮点击)。
type ControlHandler = Rc<dyn Fn(&WindowControl, &mut Window, &mut App)>;
/// 带悬停状态参数的回调(悬停变化)。
type HoverHandler = Rc<dyn Fn(&Option<WindowControl>, &mut Window, &mut App)>;
/// 拖动区按下回调。
type DragHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

/// 标题栏回调。类型与 `Context::listener` 的产物一致(参数是事件引用),
/// 因此视图可以直接用 `Rc::new(cx.listener(...))` 构造。
pub struct TitlebarHandlers {
    /// 拖动区按下(仅 AppHandled 平台会真正移动窗口)。
    pub drag: DragHandler,
    /// 拖动区双击(最大化/还原)。
    pub double_click: SimpleHandler,
    /// 某个控制按钮被点击。
    pub control: ControlHandler,
    /// 悬停按钮变化(None = 不在任何按钮上)。
    pub hover: HoverHandler,
}

/// 一个控制按钮的呈现规格(纯数据,可单测)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlSpec {
    /// 按钮种类。
    pub kind: WindowControl,
    /// 是否显示。
    pub visible: bool,
    /// 是否处于"已生效"状态(仅最大化按钮在最大化时)= 显示还原图标。
    pub active: bool,
}

/// 按平台能力与窗口状态计算按钮列表,顺序固定为 最小化 → 最大化/还原 → 关闭。
///
/// 不支持的按钮不显示也不占位:最小化在平台不支持最大化时会右移补位(WIN-005)。
pub fn control_layout(model: &TitlebarModel) -> [ControlSpec; 3] {
    [
        ControlSpec {
            kind: WindowControl::Minimize,
            visible: model.controls.minimize,
            active: false,
        },
        ControlSpec {
            kind: WindowControl::Maximize,
            visible: model.controls.maximize,
            active: model.maximized,
        },
        ControlSpec {
            kind: WindowControl::Close,
            visible: true,
            active: false,
        },
    ]
}

/// 渲染标题栏。
///
/// 布局方式:中间区 `flex_1` 吃掉剩余宽度,按钮区固定宽度靠右。
/// 为什么不用 `viewport_size` 手工算宽度:窗口最大化/最小化/还原时,GPUI 报告的
/// 视口尺寸可能与窗口实际客户区短暂不一致,按它算出的固定宽度会把按钮推到窗口外
/// (表现为"最大化后最小化再还原,三个按钮消失")。交给 flex 布局则始终贴着实际右边缘。
///
/// `center` 是标签条(多标签时)或 `None`(单标签或标签条未启用时显示标题文字的拖动区)。
/// 有标签条时它占满中间区:此时双击由标签条自己处理(空白处新建标签,MOUSE-005),
/// 标题栏不再叠加"双击最大化",避免同一次双击触发两个动作。
pub fn render_titlebar(
    model: &TitlebarModel,
    center: Option<gpui::AnyElement>,
    handlers: TitlebarHandlers,
) -> impl IntoElement {
    let specs = control_layout(model);
    let hovered = model.hovered;
    let text_color: Hsla = if model.focused {
        rgb(window::TITLE_TEXT).into()
    } else {
        rgb(window::TITLE_TEXT_INACTIVE).into()
    };

    let visible_count = specs.iter().filter(|s| s.visible).count() as f32;
    let buttons_width = visible_count * WINDOW_CONTROL_WIDTH;

    let mut buttons = div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        // 固定宽度 + 禁止收缩:不能参与 flex 收缩,否则会被拖动区压成 0 宽。
        .w(px(buttons_width))
        .flex_shrink_0();
    for spec in specs {
        buttons = buttons.child(control_button(
            &spec,
            Some(spec.kind) == hovered,
            &handlers.control,
            &handlers.hover,
        ));
    }

    // 拖动区吃掉按钮区以外的全部宽度,与按钮区互不重叠。
    //
    // 同样不设 `window_control_area(Drag)`:那会把它变成 HTCAPTION 非客户区,
    // 使整片标题栏脱离应用的鼠标事件系统。拖动改由按下时主动调用平台层的
    // `begin_window_drag`(Windows)或 `start_window_move`(Linux)完成。
    let drag_area = {
        let drag = Rc::clone(&handlers.drag);
        let hover = Rc::clone(&handlers.hover);
        div()
            .id("titlebar-drag")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .pl(px(12.0))
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                (drag)(event, window, cx);
            })
            .on_hover(move |&is_hovered, window, cx| {
                if is_hovered {
                    // 进入拖动区说明鼠标已离开按钮
                    (hover)(&None, window, cx);
                }
            })
            .child(
                div()
                    .text_sm()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(text_color)
                    .child(model.title.clone()),
            )
    };

    let double_click = Rc::clone(&handlers.double_click);
    // 有标签条时中间区由标签条占据,窗口拖动由标签条空白区自己发起
    let has_tabs = center.is_some();

    let mut bar = div()
        .id("titlebar")
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(TITLEBAR_HEIGHT))
        .w_full()
        .bg(rgb(window::TITLEBAR_BG))
        .border_b_1()
        .border_color(rgb(window::TITLEBAR_BORDER));
    if !has_tabs {
        // 无标签条时才由标题栏本身承担双击最大化(WIN-002)
        bar = bar.on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                (double_click)(event, window, cx);
            }
        });
    }

    match center {
        Some(center) => bar.child(center).child(buttons),
        None => bar.child(drag_area).child(buttons),
    }
}

/// 单个控制按钮:命中区是 div,图标用 canvas 自绘。
fn control_button(
    spec: &ControlSpec,
    hovered: bool,
    on_click: &ControlHandler,
    on_hover: &HoverHandler,
) -> gpui::AnyElement {
    if !spec.visible {
        return div().into_any_element();
    }
    let kind = spec.kind;
    let active = spec.active;

    let (bg, icon_color): (Hsla, Hsla) = match (kind, hovered) {
        (WindowControl::Close, true) => (
            rgb(window::CONTROL_CLOSE_HOVER_BG).into(),
            rgb(window::CONTROL_CLOSE_HOVER_ICON).into(),
        ),
        (_, true) => (
            rgb(window::CONTROL_HOVER_BG).into(),
            rgb(window::CONTROL_ICON).into(),
        ),
        (_, false) => (
            rgb(window::TITLEBAR_BG).into(),
            rgb(window::CONTROL_ICON).into(),
        ),
    };

    let id: &'static str = match kind {
        WindowControl::Minimize => "win-control-min",
        WindowControl::Maximize => "win-control-max",
        WindowControl::Close => "win-control-close",
    };

    let click = Rc::clone(on_click);
    let hover = Rc::clone(on_hover);

    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .w(px(WINDOW_CONTROL_WIDTH))
        .h_full()
        .cursor_pointer()
        // 刻意**不**调用 `window_control_area`:那会让 Windows 把该区域当作真正的
        // 系统按钮(HTMINBUTTON/HTMAXBUTTON/HTCLOSE),悬停最大化键时弹出 Snap 布局
        // 浮层并接管点击——而我们的按钮是自绘、动作也在应用侧执行,两套语义会打架
        // (浮层遮挡、状态错乱)。按钮完全走应用内事件。
        .on_click(move |_event, window, cx| (click)(&kind, window, cx))
        .on_hover(move |&is_hovered, window, cx| {
            let hovered = if is_hovered { Some(kind) } else { None };
            (hover)(&hovered, window, cx)
        })
        // 图标用普通元素绘制而不是 canvas:窗口最小化后恢复时,canvas 自绘的内容
        // 可能不会重新提交给合成器(实测表现为按钮与滚动条一起消失、点其他窗口才恢复)。
        // div/border/text 这类常规元素走主渲染路径,不受该问题影响。
        .child(div().absolute().size_full().bg(bg))
        .child(control_icon(kind, active, icon_color))
        .into_any_element()
}

/// 控制按钮图标的几何形状。
///
/// 全部用 `div` 的宽高/边框/文字实现,不使用 `paint_path`:
/// canvas 线条绘制在窗口最小化恢复后可能丢失,常规元素不会。
fn control_icon(kind: WindowControl, active: bool, color: Hsla) -> gpui::AnyElement {
    match kind {
        // 最小化:一条水平线
        WindowControl::Minimize => div().w(px(10.0)).h(px(1.0)).bg(color).into_any_element(),
        // 还原:两个错位的方框(外层右上、内层左下错位重叠)
        WindowControl::Maximize if active => div()
            .w(px(9.0))
            .h(px(9.0))
            .border_1()
            .border_color(color)
            .child(
                div()
                    .w(px(5.0))
                    .h(px(5.0))
                    .border_1()
                    .border_color(color)
                    .ml(px(-2.0))
                    .mt(px(2.0)),
            )
            .into_any_element(),
        // 最大化:一个方框
        WindowControl::Maximize => div()
            .w(px(9.0))
            .h(px(9.0))
            .border_1()
            .border_color(color)
            .into_any_element(),
        // 关闭:用字形 "✕"(布局不支持 rotate,无法用两条线条拼出交叉)
        WindowControl::Close => div()
            .text_size(px(13.0))
            .text_color(color)
            .child("✕")
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: WindowControlsShown = WindowControlsShown {
        minimize: true,
        maximize: true,
    };

    fn model(controls: WindowControlsShown, maximized: bool) -> TitlebarModel {
        TitlebarModel {
            title: "RustSole".into(),
            maximized,
            focused: true,
            controls,
            hovered: None,
        }
    }

    /// WIN-003:按钮顺序固定为 最小化 → 最大化 → 关闭
    #[test]
    fn win_003_button_order_is_min_max_close() {
        let specs = control_layout(&model(BOTH, false));
        assert_eq!(
            specs.map(|s| s.kind),
            [
                WindowControl::Minimize,
                WindowControl::Maximize,
                WindowControl::Close
            ]
        );
    }

    /// WIN-004:最大化时只有最大化按钮进入 active(显示还原图标)
    #[test]
    fn win_004_maximized_marks_restore_state() {
        let specs = control_layout(&model(BOTH, true));
        assert!(specs[0].visible && !specs[0].active);
        assert!(specs[1].visible && specs[1].active);
        assert!(specs[2].visible && !specs[2].active);
    }

    /// WIN-005:平台不支持的控制按钮不显示;关闭永远可用(否则窗口无法关闭)
    #[test]
    fn win_005_unsupported_controls_hidden() {
        let only_max = WindowControlsShown {
            minimize: false,
            maximize: true,
        };
        let specs = control_layout(&model(only_max, false));
        assert!(!specs[0].visible);
        assert!(specs[1].visible);
        assert!(specs[2].visible);

        let none = WindowControlsShown {
            minimize: false,
            maximize: false,
        };
        let specs = control_layout(&model(none, false));
        assert_eq!(specs.iter().filter(|s| s.visible).count(), 1);
    }
}
