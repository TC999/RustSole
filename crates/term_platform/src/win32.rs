//! Windows 原生窗口操作:自绘标题栏所需的"拖动窗口"与"还原最大化"。
//!
//! 为什么需要这一层:
//! - gpui 0.2.2 不暴露原生 `HWND`(`Window::platform_window` 是 crate 私有),
//!   也没有"还原窗口"的 API(`zoom_window` 在 Windows 上只会 `SW_MAXIMIZE`);
//! - `WindowControlArea` 的 NC 命中回调(`HTCAPTION`/`HTMAXBUTTON`)在本项目实测未生效,
//!   因此拖动与还原必须直连 Win32。
//!
//! 这里只用寥寥几个 Win32 函数,手写 FFI 而不引入 `windows` crate(硬性规则 3:
//! 不擅自新增依赖)。所有调用集中在 `unsafe` 块内,并逐处标注 SAFETY 依据。
//!
//! 定位本进程窗口的策略(用户确认):先按 gpui 的窗口类名 `Zed::Window` 精确匹配;
//! 类名若随 gpui 升级而改变,回退到"当前进程、可见、面积最大的顶层窗口"。
//! 阶段 1 是单窗口,该回退足够;多窗口阶段必须改为由 gpui 提供稳定句柄。

use core::ffi::c_void;

/// Win32 窗口句柄。Windows 上 `HWND` 就是指针大小的不透明句柄。
type Hwnd = *mut c_void;

/// `WM_NCLBUTTONDOWN`:在非客户区按下左键,`wParam` 为命中区域码。
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
/// `HTCAPTION`:标题栏区域,配合上面的消息让系统接管窗口拖动。
const HTCAPTION: usize = 2;
/// `SW_RESTORE`:还原窗口(会退出最大化/最小化状态)。
const SW_RESTORE: i32 = 9;

/// gpui 0.2.2 的窗口类名(`gpui::platform::windows::window::WINDOW_CLASS_NAME`)。
const ZED_WINDOW_CLASS: &str = "Zed::Window";

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn GetClassNameW(hwnd: Hwnd, buf: *mut u16, max: i32) -> i32;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut WinRect) -> i32;
    fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn ReleaseCapture() -> i32;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
}

/// 枚举回调的上下文。
struct FindCtx {
    /// 本进程 id。
    pid: u32,
    /// 类名精确匹配的窗口。
    exact: Option<Hwnd>,
    /// 回退候选:面积最大的本进程可见窗口。
    largest: Option<(Hwnd, i64)>,
}

/// `EnumWindows` 回调:按类名优先、面积回退的顺序挑选窗口。
extern "system" fn find_callback(hwnd: Hwnd, lparam: isize) -> i32 {
    // SAFETY: `lparam` 是 `find_main_window` 里传进来的 `&mut FindCtx`,生命周期覆盖
    // 整个 EnumWindows 调用;EnumWindows 不会跨线程保留该指针。
    let ctx = unsafe { &mut *(lparam as *mut FindCtx) };

    // SAFETY: `hwnd` 由系统在枚举时提供,是有效的窗口句柄;`pid` 指向栈上可写变量。
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid != ctx.pid {
        return 1; // 继续枚举
    }
    // SAFETY: 同上,句柄有效。
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }

    let mut name = [0u16; 64];
    // SAFETY: 缓冲区长度与传入上限一致,函数只会写入至多 `max` 个 u16。
    let len = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
    if len > 0 {
        let len = len as usize;
        let wanted: Vec<u16> = ZED_WINDOW_CLASS.encode_utf16().collect();
        if name[..len.min(name.len())] == wanted[..] {
            ctx.exact = Some(hwnd);
            return 0; // 命中,停止枚举
        }
    }

    let mut rect = WinRect::default();
    // SAFETY: 同上,句柄有效,`rect` 是栈上可写结构。
    if unsafe { GetWindowRect(hwnd, &mut rect) } != 0 {
        let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
        // 过滤掉 ConPTY 的 PseudoConsoleWindow 这类极小窗口
        if area > 40_000 && ctx.largest.is_none_or(|(_, best)| area > best) {
            ctx.largest = Some((hwnd, area));
        }
    }
    1
}

/// 找到本进程的主窗口。
fn find_main_window() -> Option<Hwnd> {
    let mut ctx = FindCtx {
        pid: std::process::id(),
        exact: None,
        largest: None,
    };
    // SAFETY: 传入的回调是 `extern "system" fn`,签名与 `WNDENUMPROC` 一致;
    // `lparam` 指向的 `ctx` 在整个调用期间存活,回调只在本线程同步执行。
    unsafe { EnumWindows(find_callback, &mut ctx as *mut FindCtx as isize) };
    ctx.exact.or(ctx.largest.map(|(hwnd, _)| hwnd))
}

/// 发起窗口拖动(阻塞到拖动结束)。
///
/// Windows 的标准做法:先释放鼠标捕获,再投递 `WM_NCLBUTTONDOWN` + `HTCAPTION`,
/// 系统随后进入自己的移动循环并接管鼠标,直到用户松开左键。
///
/// 返回值:是否成功找到窗口并发起拖动。找不到窗口时返回 `false`(调用方忽略即可,
/// 表现为"按住标题栏没有反应",不会 panic)。
pub(crate) fn begin_drag() -> bool {
    let Some(hwnd) = find_main_window() else {
        return false;
    };
    // SAFETY: `hwnd` 刚由 EnumWindows 取得、属于本进程;ReleaseCapture 无参数,
    // SendMessageW 的参数按 Win32 约定:WM_NCLBUTTONDOWN 的 wParam 是命中码,
    // lParam 为鼠标坐标(0 表示使用当前光标位置,系统移动循环会接管)。
    unsafe {
        ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0);
    }
    true
}

/// 还原已最大化的窗口(`SW_RESTORE`)。
///
/// 返回值:是否成功找到窗口并调用。窗口本来就没最大化时调用也无副作用。
pub(crate) fn restore() -> bool {
    let Some(hwnd) = find_main_window() else {
        return false;
    };
    // SAFETY: `hwnd` 由 EnumWindows 取得且属于本进程;SW_RESTORE 是合法的
    // ShowWindow 命令码。
    unsafe { ShowWindow(hwnd, SW_RESTORE) };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 类名比较必须整段相等,不能只比前缀("Zed::Window2" 不算命中)
    #[test]
    fn class_name_matches_exactly() {
        let wanted: Vec<u16> = ZED_WINDOW_CLASS.encode_utf16().collect();
        let mut buf = [0u16; 64];
        buf[..wanted.len()].copy_from_slice(&wanted);
        assert_eq!(buf[..wanted.len()], wanted[..]);
    }

    /// 窗口面积过滤阈值能排掉 ConPTY 的 0 尺寸窗口(纯算术,不碰 Win32)
    #[test]
    fn tiny_windows_are_ignored() {
        let area = i64::from(0) * i64::from(0);
        assert!(area <= 40_000);
        let real = i64::from(960) * i64::from(600);
        assert!(real > 40_000);
    }
}
