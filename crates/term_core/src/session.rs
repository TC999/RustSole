//! Session:一个 PTY + 一个 Term + 事件流(ARCHITECTURE.md 分层定义)。
//! 独立于 View,View 只持有引用并负责显示(不变量 1)。

use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier, State as LoopState};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{test::TermSize, Term};
use alacritty_terminal::tty;
use alacritty_terminal::tty::Options as TtyOptions;

use crate::proxy::{ChannelListener, EventQueue, TermEventMsg};

/// 共享的 Term 句柄:渲染与 EventLoop 线程共用,锁的临界区必须短(不变量 2)。
pub type SharedTerm = Arc<FairMutex<Term<ChannelListener>>>;

/// 一个活动会话。
pub struct Session {
    /// 共享终端状态(加锁访问)。
    pub term: SharedTerm,
    /// 向 PTY 写入的句柄(键盘输入、resize 指令)。
    notifier: Notifier,
    /// 共享事件队列(EventLoop 线程生产,UI 消费)。
    events: Arc<EventQueue>,
    /// 事件循环线程句柄(spawn 返回 (EventLoop, State),退出时可恢复,这里直接持有)。
    loop_thread: Option<std::thread::JoinHandle<(EventLoop<tty::Pty, ChannelListener>, LoopState)>>,
    /// 会话是否已退出(子进程结束)。
    exited: bool,
}

impl Session {
    /// 创建会话:分配 PTY、启动 shell、启动 EventLoop 线程。
    ///
    /// # Errors
    /// PTY 分配或 shell 启动失败时返回 io 错误。
    pub fn new(cols: u16, rows: u16) -> std::io::Result<Self> {
        let events = Arc::new(EventQueue::default());
        let listener = ChannelListener::new(Arc::clone(&events));

        let term = Arc::new(FairMutex::new(Term::new(
            alacritty_terminal::term::Config::default(),
            &TermSize {
                columns: cols as usize,
                screen_lines: rows as usize,
            },
            listener.clone(),
        )));

        let pty = tty::new(
            &TtyOptions::default(),
            WindowSize {
                num_lines: rows,
                num_cols: cols,
                cell_width: 8,
                cell_height: 16,
            },
            0,
        )?;

        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, false, false)?;
        let notifier = Notifier(event_loop.channel());
        let thread_handle = event_loop.spawn();

        Ok(Self {
            term,
            notifier,
            events,
            loop_thread: Some(thread_handle),
            exited: false,
        })
    }

    /// 把字节写入 PTY(键盘输入、粘贴)。
    pub fn write(&self, bytes: &[u8]) {
        // 空输入会让终端挂起(对齐 alacritty Notifier 的防护),直接跳过
        if bytes.is_empty() {
            return;
        }
        let _ = self.notifier.0.send(Msg::Input(Cow::Owned(bytes.to_vec())));
    }

    /// 通知 PTY 新的字符尺寸。
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.notifier.0.send(Msg::Resize(WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width: 8,
            cell_height: 16,
        }));
    }

    /// 关闭会话:通知事件循环退出并等待线程结束。
    pub fn shutdown(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
        if let Some(handle) = self.loop_thread.take() {
            let _ = handle.join();
        }
    }

    /// 非阻塞排空事件队列并更新退出状态。
    pub fn poll_events(&mut self) -> Vec<TermEventMsg> {
        let taken = self.events.drain();
        if taken.iter().any(|m| matches!(m, TermEventMsg::Exited)) {
            self.exited = true;
        }
        taken
    }

    /// 供异步事件任务的共享队列句柄(与 poll_events 消费同一队列,不会重复消费)。
    pub fn events_handle(&self) -> Arc<EventQueue> {
        Arc::clone(&self.events)
    }

    /// 子进程是否已退出。
    pub fn exited(&self) -> bool {
        self.exited
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}