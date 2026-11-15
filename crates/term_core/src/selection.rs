//! 选区缓冲(SEL-*)与选区几何(SEL-009/011/012)。
//!
//! 与系统剪贴板是两个独立通道(ARCHITECTURE.md 不变量 7),两者不得互相覆盖。
//! 进程内缓冲在 Windows 直接作为选区缓冲(SEL-007);Linux 的系统 PRIMARY 由
//! term_ui 的剪贴板适配层在提交选区时镜像(SEL-006),本层只负责存/取语义。
//!
//! 放在 term_core 而不是 term_ui:缓冲与几何不依赖 gpui,可以在无图形环境做单元测试。

use std::sync::Mutex;

use alacritty_terminal::grid::Row;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::cell::{Cell, Flags, LineLength};

/// 双击选词时视为词内字符的额外集合(SEL-011)。
///
/// 这些符号让 URL、路径等能被一次双击完整选中,而不是在点号/斜杠处被截断。
pub const WORD_EXTRA_CHARS: &str = "@-./_~?&=%+#";

/// 字符是否属于词内字符(SEL-011):字母/数字,或 WORD_EXTRA_CHARS 中的符号。
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || WORD_EXTRA_CHARS.contains(c)
}

/// 计算以 col 列所在单元格为中心的词区间(含首尾列),用于双击选词(SEL-009/SEL-011)。
///
/// 宽字符的尾部占位格(Flags::WIDE_CHAR_SPACER)不参与词边界判断,避免把一个宽字符
/// 从中间拆成两段。返回 None:该列超出行内容长度,或所在单元格不是词内字符。
pub fn word_span(row: &Row<Cell>, col: usize) -> Option<(usize, usize)> {
    let content_len = row.line_length().0;
    if col >= content_len {
        return None;
    }
    let is_word = |c: usize| -> bool {
        let cell = &row[Column(c)];
        // 占位格本身没有字符,跳过它继续判断,使宽字符整体算作一个词内单元
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            return true;
        }
        is_word_char(cell.c)
    };
    if !is_word(col) {
        return None;
    }

    let mut start = col;
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < content_len && is_word(end + 1) {
        end += 1;
    }
    Some((start, end))
}

/// 按 SEL-012 清洗从选区提取出的文本:去除每行行尾空白。
///
/// 软换行(被换行折断的同一逻辑行)不插入换行符的行为,由 alacritty 的
/// Term::selection_to_string 在提取阶段完成(其 line_to_string 依据
/// Flags::WRAPLINE 判断是否补换行),这里只负责行尾空白裁剪。
pub fn trim_selection_text(text: &str) -> String {
    text.split('\n')
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

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

    /// SEL-012:每行行尾空白被裁剪,行内空白保留
    #[test]
    fn sel_012_trims_trailing_whitespace() {
        assert_eq!(trim_selection_text("a  \nb \t \n  d"), "a\nb\n  d");
    }

    /// SEL-009/SEL-011:双击选词按词内字符扩展,点号/斜杠等额外符号属于词内
    #[test]
    fn sel_011_word_span_includes_extra_chars() {
        let mut row = Row::new(16);
        for (i, c) in "a.b/c".chars().enumerate() {
            row[Column(i)] = Cell {
                c,
                ..Default::default()
            };
        }
        // 点在 b(第 2 列):向左吃到 a.,向右吃到 /c
        assert_eq!(word_span(&row, 2), Some((0, 4)));
    }

    /// 分隔符(空格)处不构成词
    #[test]
    fn word_span_returns_none_on_separator() {
        let mut row = Row::new(8);
        for (i, c) in "a b".chars().enumerate() {
            row[Column(i)] = Cell {
                c,
                ..Default::default()
            };
        }
        assert_eq!(word_span(&row, 1), None);
    }

    /// 超出行内容长度返回 None
    #[test]
    fn word_span_out_of_content() {
        let row: Row<Cell> = Row::new(8);
        assert_eq!(word_span(&row, 3), None);
    }
}
