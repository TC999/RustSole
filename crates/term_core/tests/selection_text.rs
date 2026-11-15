//! 选区文本提取的终端状态测试(TESTING.md 第 2 节"终端状态测试")。
//!
//! 这里直接喂字节序列给 `Term`,验证 SEL-001 依赖的核心行为:
//! 选区能提取出正确的文本、空选区不产生文本。
//! UI 层面的鼠标事件与缓冲写入需要图形环境,不在本文件覆盖。

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use term_core::selection::trim_selection_text;

const COLS: usize = 20;
const ROWS: usize = 5;

/// 建一个 Terminal,并把给定字节按 VT 解析喂进去。
fn term_with(input: &str) -> Term<VoidListener> {
    let size = TermSize {
        columns: COLS,
        screen_lines: ROWS,
    };
    let mut term = Term::new(Config::default(), &size, VoidListener);
    let mut processor: Processor = Processor::new();
    processor.advance(&mut term, input.as_bytes());
    term
}

/// 设置普通(线性)选区,起点在左上、终点在右下。
fn select(term: &mut Term<VoidListener>, start: (i32, usize), end: (i32, usize)) {
    let mut selection = Selection::new(
        SelectionType::Simple,
        Point::new(Line(start.0), Column(start.1)),
        Side::Left,
    );
    selection.update(Point::new(Line(end.0), Column(end.1)), Side::Right);
    term.selection = Some(selection);
}

/// 单行选区:提取该行的字符区间
#[test]
fn sel_001_extracts_single_line() {
    let mut term = term_with("hello\r\nworld\r\n");
    select(&mut term, (0, 0), (0, 4));
    assert_eq!(term.selection_to_string().as_deref(), Some("hello"));
}

/// 跨行选区:行间插入换行
#[test]
fn sel_001_extracts_multiple_lines() {
    let mut term = term_with("abc\r\ndef\r\n");
    select(&mut term, (0, 0), (1, 2));
    assert_eq!(term.selection_to_string().as_deref(), Some("abc\ndef"));
}

/// 从行中间开始:起始列之前的字符不属于选区
#[test]
fn sel_001_start_column_respected() {
    let mut term = term_with("abcdef\r\n");
    select(&mut term, (0, 2), (0, 4));
    assert_eq!(term.selection_to_string().as_deref(), Some("cde"));
}

/// SEL-004:单击(未拖动)形成的空选区不产生文本,
/// 视图据此跳过写缓冲,不会覆盖上次选中的内容。
#[test]
fn sel_004_empty_selection_yields_no_text() {
    let mut term = term_with("keep\r\n");
    term.selection = Some(Selection::new(
        SelectionType::Simple,
        Point::new(Line(0), Column(1)),
        Side::Left,
    ));
    assert_eq!(term.selection_to_string(), None);
}

/// 没有选区时不产生文本(未选中任何东西就中键,不应粘贴出历史内容)
#[test]
fn no_selection_yields_no_text() {
    let term = term_with("abc\r\n");
    assert_eq!(term.selection_to_string(), None);
}

/// 选中宽字符所在的两个单元格时,文本里只出现一个字符(占位符不重复)
#[test]
fn wide_char_not_duplicated() {
    let mut term = term_with("中文\r\n");
    select(&mut term, (0, 0), (0, 3));
    let text = term.selection_to_string().unwrap_or_default();
    assert_eq!(text, "中文");
}

/// SEL-012:软换行行不插入换行符(整段为一行),行尾空白经清洗后不残留。
#[test]
fn sel_012_soft_wrap_and_trailing_trim() {
    // 40 个字符在 20 列网格上软换行成两行,末尾带两个空格
    let content = format!("{}  ", "x".repeat(38));
    let mut term = term_with(&content);
    select(&mut term, (0, 0), (1, 19));

    // 软换行:两行之间没有插入换行符
    let raw = term.selection_to_string().unwrap_or_default();
    assert_eq!(raw.matches('\n').count(), 0);
    // 行尾的无样式空格被 alacritty 的 line_length 丢弃,清洗后不残留
    assert_eq!(trim_selection_text(&raw), "x".repeat(38));
}
