//! 核心层:Session、事件流、键盘编码、粘贴处理。不依赖 gpui(ARCHITECTURE.md 硬约束)。

pub mod key;
pub mod mouse;
pub mod paste;
pub mod proxy;
pub mod selection;
pub mod session;

/// TERM 模式位:应用光标键模式(KEY-005 编码需要)。
pub fn mode_flag_app_cursor() -> alacritty_terminal::term::TermMode {
    alacritty_terminal::term::TermMode::APP_CURSOR
}
