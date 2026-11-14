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
    canvas, div, point, px, rgb, App, Bounds, ClickEvent, Hsla, IntoElement, MouseButton,
    MouseDownEvent, PathBuilder, Pixels, SharedString, Window, WindowControlArea,
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
/// `bar_width` 是标题栏可用宽度(px)。拖动区与按钮区都用**固定宽度**相加等于它:
/// 实测在 gpui 0.2.2 里,若拖动区用 `flex_1`(basis 0)、按钮区用内容宽度,
/// 按钮区会被压成 0 宽(按钮整体消失);因此这里不依赖 flex 增长,直接按已知宽度分配。
pub fn render_titlebar(
    model: &TitlebarModel,
    handlers: TitlebarHandlers,
    bar_width: f32,
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
    let drag_width = (bar_width - buttons_width).max(0.0);

    let mut buttons = div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .flex_shrink_0()
        .w(px(buttons_width));
    for spec in specs {
        buttons = buttons.child(control_button(
            &spec,
            Some(spec.kind) == hovered,
            &handlers.control,
            &handlers.hover,
        ));
    }

    // 拖动区占据除按钮区以外的整条标题栏,宽度按已知总宽算出,与按钮区互不重叠。
    let drag_area = {
        let drag = Rc::clone(&handlers.drag);
        let hover = Rc::clone(&handlers.hover);
        div()
            .id("titlebar-drag")
            .flex_shrink_0()
            .w(px(drag_width))
            .h_full()
            .flex()
            .items_center()
            .pl(px(12.0))
            .window_control_area(WindowControlArea::Drag)
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

    div()
        .id("titlebar")
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(TITLEBAR_HEIGHT))
        .w_full()
        .bg(rgb(window::TITLEBAR_BG))
        .border_b_1()
        .border_color(rgb(window::TITLEBAR_BORDER))
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                (double_click)(event, window, cx);
            }
        })
        .child(drag_area)
        .child(buttons)
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

    let area = match kind {
        WindowControl::Minimize => WindowControlArea::Min,
        WindowControl::Maximize => WindowControlArea::Max,
        WindowControl::Close => WindowControlArea::Close,
    };

    let click = Rc::clone(on_click);
    let hover = Rc::clone(on_hover);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .w(px(WINDOW_CONTROL_WIDTH))
        .h_full()
        .cursor_pointer()
        // 命中区标记:让平台知道这是窗口控制按钮(Windows 上对应 HTMIN/HTMAX/HTCLOSE)。
        // 实测 gpui 0.2.2 在 Windows 上这条 NC 命中路径没有生效(见汇报),
        // 因此按钮动作统一由下面的 on_click 在应用侧执行;标记保留是为了平台修复后
        // 能自动获得系统级按钮语义(Snap 布局等),Linux 合成器也可据此排布窗口控制区。
        .window_control_area(area)
        .on_click(move |_event, window, cx| (click)(&kind, window, cx))
        .on_hover(move |&is_hovered, window, cx| {
            let hovered = if is_hovered { Some(kind) } else { None };
            (hover)(&hovered, window, cx)
        })
        .child(
            canvas(
                move |_bounds, _window, _cx| (),
                move |bounds, (), window, _cx| {
                    paint_control_icon(bounds, kind, active, bg, icon_color, window);
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// 在按钮区域内绘制图标。纯几何自绘,不依赖资源文件。
fn paint_control_icon(
    bounds: Bounds<Pixels>,
    kind: WindowControl,
    active: bool,
    bg: Hsla,
    icon_color: Hsla,
    window: &mut Window,
) {
    window.paint_quad(gpui::fill(bounds, bg));

    let center = bounds.center();
    let stroke = px(1.0);

    // 以按钮中心为原点绘制折线;坐标是固定常量,构建失败只跳过绘制,不 panic。
    let mut draw = |points: &[(f32, f32)], close: bool| {
        if points.is_empty() {
            return;
        }
        let mut builder = PathBuilder::stroke(stroke);
        builder.move_to(point(
            center.x + px(points[0].0),
            center.y + px(points[0].1),
        ));
        for (dx, dy) in &points[1..] {
            builder.line_to(point(center.x + px(*dx), center.y + px(*dy)));
        }
        if close {
            builder.close();
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, icon_color);
        }
    };

    match kind {
        // 最小化:一条水平线
        WindowControl::Minimize => draw(&[(-4.0, 0.0), (4.0, 0.0)], false),
        // 还原:两个错位的方框(后画的在前)
        WindowControl::Maximize if active => {
            draw(&[(-3.0, -4.0), (4.0, -4.0), (4.0, 3.0), (2.0, 3.0)], false);
            draw(&[(-4.0, -2.0), (-4.0, 4.0), (3.0, 4.0), (3.0, 2.0)], false);
        }
        // 最大化:一个方框
        WindowControl::Maximize => {
            draw(&[(-4.0, -4.0), (4.0, -4.0), (4.0, 4.0), (-4.0, 4.0)], true)
        }
        // 关闭:两条对角线
        WindowControl::Close => {
            draw(&[(-4.0, -4.0), (4.0, 4.0)], false);
            draw(&[(-4.0, 4.0), (4.0, -4.0)], false);
        }
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
