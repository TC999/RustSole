//! 平台差异层。PTY 已由 alacritty_terminal::tty 内聚(cfg 在其内部),
//! 本 crate 承担剩余的平台差异:默认字体、窗口装饰能力(窗口标题栏由谁绘制/拖动)。
//! 所有 `cfg(target_os)` 集中在本 crate(硬性规则 6),对外只暴露统一接口(ARCHITECTURE.md 不变量 6)。

/// 平台相关的默认值集合。
#[derive(Debug, Clone, Copy)]
pub struct PlatformInfo {
    /// 默认等宽字体族名。来源:各平台系统自带等宽字体。
    pub default_font_family: &'static str,
}

/// 返回当前平台的默认信息。
pub fn platform_info() -> PlatformInfo {
    PlatformInfo {
        default_font_family: default_font_family(),
    }
}

/// 自绘窗口标题栏时,窗口移动与按钮动作由谁执行。
///
/// 这个能力位存在的原因:gpui 0.2.2 在 Windows 上把 `WindowControlArea` 命中框
/// 映射为 HTCAPTION/HTMINBUTTON/HTMAXBUTTON/HTCLOSE(`platform/windows/events.rs:868-878`),
/// 设计上由系统完成拖动与按钮动作;而 X11/Wayland 的 `on_hit_test_window_control` 是空实现,
/// 应用必须自己调用 `Window::start_window_move` / `minimize_window` / `zoom_window`。
///
/// 现实约束(本次实测):Windows 上这条 NC 命中路径未能在开发环境验证成立,
/// 因此按钮动作统一由应用侧 `on_click` 执行(`apply_window_control`),
/// 只有窗口拖动仍依赖 `SystemHandled` 的系统路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChrome {
    /// 系统通过 `WindowControlArea` 接管窗口拖动(Windows)。
    SystemHandled,
    /// 应用自己处理拖动与按钮动作(Linux)。
    AppHandled,
}

/// 返回当前平台的自绘标题栏分工。
pub fn window_chrome() -> WindowChrome {
    #[cfg(target_os = "windows")]
    {
        WindowChrome::SystemHandled
    }
    #[cfg(not(target_os = "windows"))]
    {
        WindowChrome::AppHandled
    }
}

#[cfg(target_os = "windows")]
fn default_font_family() -> &'static str {
    // Windows 系统自带等宽字体
    "Consolas"
}

#[cfg(not(target_os = "windows"))]
fn default_font_family() -> &'static str {
    // Linux/macOS:fontconfig 在多数发行版提供 monospace 别名,交给字体系统解析
    "monospace"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认字体族名非空(两平台编译都需满足)
    #[test]
    fn default_font_family_non_empty() {
        assert!(!platform_info().default_font_family.is_empty());
    }

    /// WIN-006:两个平台的窗口装饰分工必须是明确定义的一种,不能"两边都做"或"两边都不做"
    #[test]
    fn win_006_window_chrome_is_defined_per_platform() {
        let chrome = window_chrome();
        #[cfg(target_os = "windows")]
        assert_eq!(chrome, WindowChrome::SystemHandled);
        #[cfg(not(target_os = "windows"))]
        assert_eq!(chrome, WindowChrome::AppHandled);
    }
}
