//! 选区缓冲(SEL-*)。
//!
//! 与系统剪贴板是两个独立通道(ARCHITECTURE.md 不变量 7),两者不得互相覆盖。
//! 本阶段两个平台都用进程内缓冲:Windows 符合 SEL-007;Linux 的系统 PRIMARY
//! 接入(SEL-006)尚未实现,由上层汇报说明。
//!
//! 放在 term_core 而不是 term_ui:缓冲不依赖 gpui,可以在无图形环境做单元测试。

use std::sync::Mutex;

/// 进程内选区缓冲。
///
/// 进程级共享由 UI 层用 GPUI `Global` 包装实现(SEL-007:同一进程内所有窗口共享),
/// 本类型只负责"存/取"语义。
#[derive(Default)]
pub struct SelectionBuffer {
    /// 当前选区文本。用 `Mutex` 而不是 `RefCell`:`Global` 要求 `'static`,
    /// 且将来多窗口在多个回调里访问同一实例。
    text: Mutex<String>,
}

impl SelectionBuffer {
    /// 新建空缓冲。
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入选区文本。
    ///
    /// SEL-004:选中结果为空字符串时不覆盖已有内容(空选区不能把上次选中的文本抹掉)。
    pub fn set(&self, text: String) {
        if text.is_empty() {
            return;
        }
        // 锁中毒只可能是其他线程持锁时 panic;此时保留旧的缓冲区内容,
        // 粘贴到空文本比 panic 更可接受,属于"失败也无影响"的场景。
        if let Ok(mut slot) = self.text.lock() {
            *slot = text;
        }
    }

    /// 读取选区文本;缓冲为空时返回 `None`(调用方据此不发送任何字节)。
    pub fn get(&self) -> Option<String> {
        // 同上:锁中毒时按"无内容"处理,不做 panic 传播。
        let slot = self.text.lock().ok()?;
        if slot.is_empty() {
            None
        } else {
            Some(slot.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEL-004:空选区不覆盖已有内容
    #[test]
    fn sel_004_empty_selection_does_not_overwrite() {
        let buffer = SelectionBuffer::new();
        buffer.set("first".to_string());
        buffer.set(String::new());
        assert_eq!(buffer.get().as_deref(), Some("first"));
    }

    /// 空缓冲的初始状态:没有可粘贴内容
    #[test]
    fn empty_buffer_has_no_content() {
        let buffer = SelectionBuffer::new();
        assert_eq!(buffer.get(), None);
    }

    /// SEL-003:取消选区高亮不清空缓冲。视图侧取消高亮只是丢弃 Term 的 selection,
    /// 不调用 `set`,所以缓冲必须原样保留 —— 这里用"只读不写"模拟该路径。
    #[test]
    fn sel_003_clearing_highlight_keeps_buffer() {
        let buffer = SelectionBuffer::new();
        buffer.set("kept".to_string());
        // 取消高亮:视图不写缓冲,只继续读
        assert_eq!(buffer.get().as_deref(), Some("kept"));
        assert_eq!(buffer.get().as_deref(), Some("kept"));
    }

    /// 非空选区覆盖旧内容(新的一次选中应替换上次结果)
    #[test]
    fn non_empty_selection_replaces_previous() {
        let buffer = SelectionBuffer::new();
        buffer.set("first".to_string());
        buffer.set("second".to_string());
        assert_eq!(buffer.get().as_deref(), Some("second"));
    }
}
