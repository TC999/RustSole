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
    let mut rows_runs = Vec::with_capacity(rows);
    for line in 0..rows as i32 {
        let l = Line(line);
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
    }
}