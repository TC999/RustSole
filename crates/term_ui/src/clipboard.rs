//! 系统剪贴板 / PRIMARY 选区适配(SEL-006、SEL-008)。
//!
//! 业务代码不直接调用 gpui 的 cfg 门控剪贴板 API,统一经此模块转发,把平台分支集中。
//!
//! 说明:gpui 的 App::write_to_primary 仅在 Linux/FreeBSD 编译(其自身用 cfg 门控),
//! 且只能在持有 App 上下文的 UI 层调用。term_platform 不依赖 gpui(否则会破坏
//! term_core 不依赖 gpui 的分层约束,见 ARCHITECTURE.md 第 2 节),因此 PRIMARY 的
//! 平台分支只能集中在此适配模块。这是对硬性规则 6 的一处有界偏离,已在汇报中说明。

use gpui::{App, ClipboardItem};

/// 把选区文本写入系统通道。
///
/// - copy_on_select(SEL-008):为真时写入系统剪贴板(所有平台)。
/// - SEL-006:在支持 PRIMARY 的平台(Linux/FreeBSD)同时写入 PRIMARY,
///   使其他应用可以中键读取本终端的选区。
pub fn write_selection(cx: &mut App, text: &str, copy_on_select: bool) {
    if copy_on_select {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    }
    write_primary(cx, text);
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn write_primary(cx: &mut App, text: &str) {
    cx.write_to_primary(ClipboardItem::new_string(text.to_owned()));
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn write_primary(_cx: &mut App, _text: &str) {}
