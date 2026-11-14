//! alacritty EventListener 实现:把终端事件写入共享队列,由 UI 侧任务消费。
//! PTY 读写与 Term 更新都在 EventLoop 线程(ARCHITECTURE.md 不变量 3)。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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

/// EventLoop 线程与 UI 任务之间的共享事件队列。
///
/// 生产者:alacritty EventLoop 线程。消费者:UI 主线程 / 异步事件任务(两者只应有一个在消费,
/// 见 `Session::poll_events` 与 `SharedEventQueue`)。锁临界区只有入队/出队,满足不变量 2。
#[derive(Default)]
pub struct EventQueue {
    queue: Mutex<VecDeque<TermEventMsg>>,
    /// 当前积压事件数;0 表示无需唤醒(消费者据此决定是否休眠,避免无谓加锁)。
    pending: AtomicUsize,
}

impl EventQueue {
    /// 投递一个事件(EventLoop 线程调用)。
    fn push(&self, msg: TermEventMsg) {
        // 队列丢失仅在持有方全部释放后不可能发生;锁中毒说明已有 panic,传播即可
        if let Ok(mut q) = self.queue.lock() {
            q.push_back(msg);
            self.pending.fetch_add(1, Ordering::Release);
        }
    }

    /// 非阻塞取走全部积压事件。
    pub fn drain(&self) -> Vec<TermEventMsg> {
        let taken = match self.queue.lock() {
            Ok(mut q) => {
                let drained: Vec<TermEventMsg> = q.drain(..).collect();
                drained
            },
            Err(_) => Vec::new(),
        };
        if !taken.is_empty() {
            self.pending.fetch_sub(taken.len(), Ordering::Release);
        }
        taken
    }

    /// 是否可能有积压事件(廉价检查,无锁)。
    pub fn has_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire) > 0
    }
}

/// 把 TermEvent 投递到共享队列的监听器。
#[derive(Clone)]
pub struct ChannelListener {
    queue: Arc<EventQueue>,
}

impl ChannelListener {
    pub fn new(queue: Arc<EventQueue>) -> Self {
        Self { queue }
    }
}

impl EventListener for ChannelListener {
    fn send_event(&self, event: TermEvent) {
        self.queue.push(match event {
            TermEvent::Wakeup => TermEventMsg::Wakeup,
            TermEvent::ChildExit(_) => TermEventMsg::Exited,
            TermEvent::Exit => TermEventMsg::Exited,
            TermEvent::PtyWrite(s) => TermEventMsg::PtyWrite(s),
            TermEvent::Title(s) => TermEventMsg::Title(s),
            _ => TermEventMsg::Other,
        });
    }
}
