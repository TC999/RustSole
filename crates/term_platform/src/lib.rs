//! 平台差异层。PTY 已由 alacritty_terminal::tty 内聚(cfg 在其内部),
//! 本 crate 承担剩余的平台差异:默认字体、窗口装饰能力(标题栏由谁绘制/拖动/还原)。
//! 所有 `cfg(target_os)` 集中在本 crate(硬性规则 6),对外只暴露统一接口(ARCHITECTURE.md 不变量 6)。

#[cfg(target_os = "windows")]
mod win32;

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

/// 当前平台是否支持系统 PRIMARY 选区(SEL-006)。
///
/// 只有 X11/Wayland 等有 PRIMARY 选区概念的平台返回 true;Windows/macOS 没有,
/// 业务代码据此决定是否把选区缓冲镜像到系统选区。平台分支集中在此(硬性规则 6)。
pub fn supports_primary_selection() -> bool {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        false
    }
}

/// 自绘窗口标题栏时,窗口移动与还原由谁执行。
///
/// 背景:gpui 0.2.2 提供 `WindowControlArea`,Windows 上会把命中框映射为
/// HTCAPTION/HTMINBUTTON/HTMAXBUTTON/HTCLOSE(`platform/windows/events.rs:868-878`),
/// 由系统接管拖动与按钮。但本项目**刻意不使用它**:那会让系统认为标题栏上存在真正的
/// 系统按钮(悬停最大化键会弹出 Windows 11 的 Snap 布局面板),与"自绘按钮 + 应用侧
/// 处理点击"形成两套语义,互相干扰(浮层遮挡、点击偶尔失效)。
///
/// 因此两种平台的按钮点击都由应用侧 `on_click` 处理,差异只在拖动与还原的能力来源:
/// - Windows:`Window::start_window_move` 是空实现、也没有还原 API,由本 crate 的
///   [`begin_window_drag`] / [`restore_window`] 直连 Win32;
/// - Linux(X11/Wayland):gpui 已实现 `start_window_move`,且 `zoom()` 本身就是
///   最大化/还原切换,直接用 `Window` 的方法即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChrome {
    /// Windows:拖动与还原走本 crate 的 Win32 实现,按钮走应用侧。
    SystemHandled,
    /// Linux:拖动与按钮动作都通过 gpui 的 `Window` 方法完成。
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

/// 发起窗口拖动,阻塞到用户松开左键。
///
/// Windows 实现:`ReleaseCapture` + `WM_NCLBUTTONDOWN`/`HTCAPTION`,把拖动交还给系统。
/// 其他平台:返回 `false`,由调用方改用 `Window::start_window_move`(X11/Wayland 已实现)。
///
/// 返回值只表示"是否成功找到窗口并发起拖动";失败时表现为按住标题栏无反应,不会 panic。
pub fn begin_window_drag() -> bool {
    #[cfg(target_os = "windows")]
    {
        win32::begin_drag()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// 还原已最大化的窗口。
///
/// Windows 实现:`ShowWindow(SW_RESTORE)` —— gpui 0.2.2 的 `zoom_window` 只会
/// `SW_MAXIMIZE`,没有还原入口,所以这里直连 Win32。
/// 其他平台:返回 `false`,由调用方改用 `Window::zoom_window`(其 `zoom` 本身是切换语义)。
pub fn restore_window() -> bool {
    #[cfg(target_os = "windows")]
    {
        win32::restore()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// 窗口客户区的实际尺寸,换算成逻辑像素(已除以 `scale_factor`)。
///
/// 为什么需要:窗口"最大化 -> 最小化 -> 还原为普通尺寸"时,gpui 报告的
/// `viewport_size()` 可能仍停留在最大化时的大小,而按它布局的自绘元素(标题栏按钮、
/// 滚动条)会被画到窗口外,表现为"按钮和滚动条消失、点一下别的窗口又回来"。
/// 这里直接向系统要真实客户区尺寸,供上层校正布局。
///
/// 返回 `None`:非 Windows 平台,或窗口尚未创建/系统调用失败 —— 调用方应回退到
/// `viewport_size()`。
pub fn client_size_logical(scale_factor: f32) -> Option<(f32, f32)> {
    #[cfg(target_os = "windows")]
    {
        // 缩放因子异常时不换算,避免把布局算成 0 或负数
        if !(scale_factor.is_finite() && scale_factor > 0.0) {
            return None;
        }
        let (w, h) = win32::client_size()?;
        Some((w / scale_factor, h / scale_factor))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = scale_factor;
        None
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

    /// SEL-006:PRIMARY 选区能力只应在 Linux/FreeBSD 为真
    #[test]
    fn sel_006_primary_selection_capability() {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        assert!(supports_primary_selection());
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        assert!(!supports_primary_selection());
    }
}
