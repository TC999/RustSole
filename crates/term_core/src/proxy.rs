//! alacritty EventListener 实现:把终端事件经 channel 转交 GPUI 主线程。
//! PTY 读写与 Term 更新都在 EventLoop 线程(ARCHITECTURE.md 不变量 3)。

use std::sync::mpsc;

use alacritty_terminal::event::{Event as TermEvent, EventListener};

/// 供 UI 层消费的终端事件。
#[derive(Debug)]
pub enum TermEventMsg {
    /// 需要重绘(EventLoop 解析到新输出)。
    Wakeup,
    /// 子进程退出。
    Exited,
    /// 程序请求写 PTY(如应答 DA/DSR 查询)。
    PtyWrite(String),
    /// 程序请求设置窗口标题。
    Title(String),
    /// 其他未处理事件(记录用)。
    Other,
}

/// 把 TermEvent 投递到 channel 的监听器。
#[derive(Clone)]
pub struct ChannelListener {
    tx: mpsc::Sender<TermEventMsg>,
}

impl ChannelListener {
    pub fn new(tx: mpsc::Sender<TermEventMsg>) -> Self {
        Self { tx }
    }
}

impl EventListener for ChannelListener {
    fn send_event(&self, event: TermEvent) {
        // 向已关闭的 channel 发送失败也无影响:会话已被销毁(CODING_STANDARDS.md 第 3 节)
        let _ = self.tx.send(match event {
            TermEvent::Wakeup => TermEventMsg::Wakeup,
            TermEvent::ChildExit(_) => TermEventMsg::Exited,
            TermEvent::Exit => TermEventMsg::Exited,
            TermEvent::PtyWrite(s) => TermEventMsg::PtyWrite(s),
            TermEvent::Title(s) => TermEventMsg::Title(s),
            _ => TermEventMsg::Other,
        });
    }
}
