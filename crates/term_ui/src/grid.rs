//! 从 Term 提取可绘制快照(锁内),并做同 run 合并。
//! 快照结构独立于 alacritty 类型,锁外绘制。

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use term_core::proxy::ChannelListener;

/// 单元格样式(简化子集,渲染用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStyle {
    /// 前景色 RGB。
    pub fg: [u8; 3],
    /// 背景色 RGB。
    pub bg: [u8; 3],
    /// 粗体。
    pub bold: bool,
    /// 反显。
    pub inverse: bool,
}

/// 一行中一段同样式文本。
#[derive(Debug, Clone)]
pub struct TextRun {
    /// 起始列。
    pub start_col: usize,
    /// 文本内容(宽字符占位符已剔除)。
    pub text: String,
    /// 样式。
    pub style: CellStyle,
}

/// 一屏快照。
#[derive(Debug, Clone)]
pub struct GridSnapshot {
    /// 行数。
    pub rows: usize,
    /// 列数。
    pub cols: usize,
    /// 每行文本 run。
    pub rows_runs: Vec<Vec<TextRun>>,
    /// 光标位置(行、列)。
    pub cursor: Option<(usize, usize)>,
    /// 当前显示偏移(回看的历史行数;0 = 显示最新内容)。
    pub display_offset: usize,
}

/// 调色板:16 色 + 前景/背景。默认值取自常见 xterm 配色(阶段 1 无配置系统)。
pub const PALETTE: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], // black
    [0xcd, 0x31, 0x31], // red
    [0x00, 0xcd, 0x00], // green
    [0xcd, 0xcd, 0x00], // yellow
    [0x00, 0x00, 0xee], // blue
    [0xcd, 0x00, 0xcd], // magenta
    [0x00, 0xcd, 0xcd], // cyan
    [0xe5, 0xe5, 0xe5], // white
    [0x7f, 0x7f, 0x7f], // bright black
    [0xff, 0x45, 0x00], // bright red
    [0x00, 0xff, 0x00], // bright green
    [0xff, 0xd7, 0x00], // bright yellow
    [0x00, 0x00, 0xff], // bright blue
    [0xff, 0x00, 0xff], // bright magenta
    [0x00, 0xff, 0xff], // bright cyan
    [0xff, 0xff, 0xff], // bright white
];

pub const DEFAULT_FG: [u8; 3] = [0xe5, 0xe5, 0xe5];
pub const DEFAULT_BG: [u8; 3] = [0x10, 0x10, 0x10];

/// 解析 alacritty 颜色到 RGB(粗略:阶段 1 不支持真彩索引外的 dim 混合细节)。
fn resolve_color(color: Color, bold: bool, for_fg: bool) -> [u8; 3] {
    match color {
        Color::Named(named) => {
            let base = match named {
                NamedColor::Foreground => DEFAULT_FG,
                NamedColor::Background => DEFAULT_BG,
                NamedColor::BrightForeground => PALETTE[15],
                n if (n as usize) < 8 => PALETTE[n as usize],
                n => PALETTE[(n as usize) - 8],
            };
            // 粗体前景提亮到 bright 变体(xterm 常见行为)
            if bold && for_fg {
                match named as usize {
                    0..=7 => PALETTE[(named as usize) + 8],
                    _ => base,
                }
            } else {
                base
            }
        }
        Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        Color::Indexed(i) => {
            // 0..16 直接映射调色板;16..232 色阶与 232..256 灰阶在此简化(阶段 1),
            // 只处理 <16 与灰阶,色阶回退为白/黑(记录在汇报中)。
            let idx = i as usize;
            if idx < 16 {
                PALETTE[idx]
            } else if (232..256).contains(&idx) {
                let v = ((idx - 232) * 10 + 8) as u8;
                [v, v, v]
            } else {
                // 6x6x6 色阶:component = level*51
                let n = idx - 16;
                let r = (n / 36) as u8;
                let g = ((n % 36) / 6) as u8;
                let b = (n % 6) as u8;
                [r * 51, g * 51, b * 51]
            }
        }
    }
}

/// 提取快照:必须在锁内调用完毕后立即释放锁(不变量 2)。
pub fn snapshot(term: &Term<ChannelListener>) -> GridSnapshot {
    let grid = term.grid();
    let rows = grid.screen_lines();
    let cols = grid.columns();

    // alacritty 0.26 索引:Line(0) 是视口顶行,Line(rows-1) 是视口底行,负行是 scrollback
    // (依据:grid/mod.rs display_iter 起点 Line(-offset-1) 与 Dimensions::bottommost_line = screen_lines-1)
    // display_offset>0 时视口顶行下移到 Line(-offset),快照必须按偏移遍历,否则回看内容不对。
    let top_line = -(grid.display_offset() as i32);
    let mut rows_runs = Vec::with_capacity(rows);
    for i in 0..rows as i32 {
        let l = Line(top_line + i);
        let row = &grid[l];
        let mut runs: Vec<TextRun> = Vec::new();
        let mut cur_text = String::new();
        let mut cur_style: Option<CellStyle> = None;
        let mut cur_start = 0usize;

        for c in 0..cols {
            let cell: &Cell = &row[Column(c)];
            // 宽字符的右侧占位与行首宽字符占位不渲染
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let inverse = cell.flags.contains(Flags::INVERSE);
            let bold = cell.flags.contains(Flags::BOLD);
            let mut fg = resolve_color(cell.fg, bold, true);
            let mut bg = resolve_color(cell.bg, false, false);
            if inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            let style = CellStyle {
                fg,
                bg,
                bold,
                inverse,
            };

            if cell.c == ' ' && !inverse && cell.bg == Color::Named(NamedColor::Background) {
                // 纯空白且无背景变化:结束当前 run
                if let Some(s) = cur_style.take() {
                    runs.push(TextRun {
                        start_col: cur_start,
                        text: std::mem::take(&mut cur_text),
                        style: s,
                    });
                }
                continue;
            }

            let append = match &cur_style {
                Some(s) if *s == style => true,
                Some(s) => {
                    runs.push(TextRun {
                        start_col: cur_start,
                        text: std::mem::take(&mut cur_text),
                        style: s.clone(),
                    });
                    false
                }
                None => false,
            };
            if !append {
                cur_start = c;
            }
            cur_style = Some(style);
            cur_text.push(cell.c);
        }
        if let Some(s) = cur_style.take() {
            runs.push(TextRun {
                start_col: cur_start,
                text: cur_text,
                style: s,
            });
        }
        rows_runs.push(runs);
    }

    let cursor_point = term.grid().cursor.point;
    let cursor = if term.mode().contains(TermMode::SHOW_CURSOR) {
        let line = cursor_point.line.0;
        if (0..rows as i32).contains(&line) {
            Some((line as usize, cursor_point.column.0))
        } else {
            None
        }
    } else {
        None
    };

    GridSnapshot {
        rows,
        cols,
        rows_runs,
        cursor,
        display_offset: grid.display_offset(),
    }
}

/// 滚动条 thumb 位置(0..1)与显示偏移的映射。
///
/// 语义基准(实测确认):
/// - display_offset = 0 表示显示**最新**内容 → thumb 应贴在轨道**底部**;
/// - display_offset = history 表示回看到**最旧**内容 → thumb 贴在轨道**顶部**。
///
/// 因此 thumb 比例 = 1 - offset / history,与 offset 成**反比**。
pub fn thumb_ratio(display_offset: usize, history: usize) -> f32 {
    if history == 0 {
        return 1.0;
    }
    let offset = display_offset.min(history) as f32;
    1.0 - offset / history as f32
}

/// 滚动条轨道位移(0..1,0 = 轨道顶)换算回 display_offset。
///
/// 与 [thumb_ratio] 互逆:轨道顶(0.0)→ offset = history(最旧);
/// 轨道底(1.0)→ offset = 0(最新)。
pub fn ratio_to_offset(ratio: f32, history: usize) -> usize {
    if history == 0 {
        return 0;
    }
    let r = ratio.clamp(0.0, 1.0);
    ((1.0 - r) * history as f32).round() as usize
}

#[cfg(test)]
mod scrollbar_tests {
    use super::*;

    /// 无历史:thumb 贴底(最新内容)
    #[test]
    fn no_history_thumb_at_bottom() {
        assert_eq!(thumb_ratio(0, 0), 1.0);
        assert_eq!(ratio_to_offset(1.0, 0), 0);
    }

    /// offset=0(最新)→ thumb 在底部;offset=history(最旧)→ thumb 在顶部
    #[test]
    fn thumb_position_is_inverse_of_offset() {
        assert_eq!(thumb_ratio(0, 100), 1.0);
        assert_eq!(thumb_ratio(100, 100), 0.0);
        assert_eq!(thumb_ratio(50, 100), 0.5);
    }

    /// 轨道顶对应最旧内容,轨道底对应最新内容
    #[test]
    fn ratio_maps_back_to_offset() {
        assert_eq!(ratio_to_offset(0.0, 100), 100);
        assert_eq!(ratio_to_offset(1.0, 100), 0);
        assert_eq!(ratio_to_offset(0.5, 100), 50);
    }

    /// 两个方向互逆
    #[test]
    fn roundtrip_is_identity() {
        for offset in [0, 1, 37, 99, 100] {
            let r = thumb_ratio(offset, 100);
            assert_eq!(ratio_to_offset(r, 100), offset);
        }
    }

    /// 向下拖动 thumb(比例增大)→ offset 变小 → 趋向最新内容(与鼠标移动方向一致)
    #[test]
    fn dragging_down_moves_toward_latest() {
        // 起始在中间
        let start = 0.5_f32;
        assert_eq!(ratio_to_offset(start, 100), 50);
        // 鼠标向下拖 → 比例变大
        assert_eq!(ratio_to_offset(0.6, 100), 40);
        assert!(ratio_to_offset(0.6, 100) < ratio_to_offset(start, 100));
        // 鼠标向上拖 → 比例变小 → offset 变大(更旧)
        assert_eq!(ratio_to_offset(0.4, 100), 60);
    }

    /// 越界输入被钳制
    #[test]
    fn clamps_out_of_range() {
        assert_eq!(ratio_to_offset(-5.0, 100), 100);
        assert_eq!(ratio_to_offset(5.0, 100), 0);
        assert_eq!(thumb_ratio(200, 100), 0.0);
    }
}