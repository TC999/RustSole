//! 选区几何:像素坐标 → 网格单元格,以及选区在某一行的列区间。
//!
//! 纯计算,不依赖 gpui 的绘制/事件类型,可在无图形环境下单测
//! (TESTING.md:选区文本裁剪与坐标映射应有纯单元测试)。

/// 终端画布的几何参数,鼠标坐标换算单元格需要。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridGeometry {
    /// 画布左上角 x(视口逻辑像素)。
    pub origin_x: f32,
    /// 画布左上角 y。
    pub origin_y: f32,
    /// 单元格宽度。
    pub cell_width: f32,
    /// 单元格高度。
    pub cell_height: f32,
    /// 视口可见行数。
    pub rows: usize,
    /// 可见列数。
    pub cols: usize,
}

/// 视口内的一个单元格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPoint {
    /// 视口行号,0 = 视口顶行(不是 grid 里的 Line,后者含负数的历史行)。
    pub row: usize,
    /// 列号。
    pub col: usize,
    /// 落点在单元格左半侧。
    ///
    /// 换算成 alacritty 的 `Side` 时:左侧 = `Side::Left`(端点在该列左边界,
    /// 该列本身不算进选区),右侧 = `Side::Right`(端点在右边界,下一列开始)。
    /// 依据 `selection.rs` 的 `range_simple`:end 为 Left 时末列减一、start 为 Right 时首列加一。
    pub side_left: bool,
}

impl GridGeometry {
    /// 像素位置换算成视口单元格。
    ///
    /// 位置落在画布外时钳制到最近的边缘单元格,而不是返回 `None`:
    /// 拖拽时鼠标离开内容区是常态,钳制能让选区延伸到边界
    /// (SEL-013 的自动滚动尚未实现,这里只做不越界的静态延伸)。
    ///
    /// `None`:画布/网格尺寸无效(尚未测量出列宽、窗口过小、坐标非有限值)。
    pub fn cell_at(&self, x: f32, y: f32) -> Option<CellPoint> {
        if !self.is_valid() {
            return None;
        }
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }

        let col_f = (x - self.origin_x) / self.cell_width;
        let row_f = (y - self.origin_y) / self.cell_height;
        // 半格判断用原始小数部分:钳制只影响越界坐标,不应翻转边界语义
        let side_left = (col_f - col_f.floor()) < 0.5;

        let row = row_f.floor().clamp(0.0, (self.rows - 1) as f32) as usize;
        let col = col_f.floor().clamp(0.0, (self.cols - 1) as f32) as usize;

        Some(CellPoint {
            row,
            col,
            side_left,
        })
    }

    /// 尺寸是否可用于换算(行列为 0 或单元格尺寸非正数时不可用)。
    fn is_valid(&self) -> bool {
        self.cell_width.is_finite()
            && self.cell_width > 0.0
            && self.cell_height.is_finite()
            && self.cell_height > 0.0
            && self.rows > 0
            && self.cols > 0
    }
}

/// 终端是否自行处理这次鼠标操作。
///
/// MOUSE-001:程序开启鼠标上报(vim `mouse=a`、tmux、htop)时鼠标事件属于程序,
/// 终端不做选区与粘贴;MOUSE-002:按住 Shift 时忽略上报状态,由终端自己处理。
///
/// 本次未实现鼠标事件向程序的上报编码,所以"交给程序"表现为终端不响应;
/// 这不会比现状更差(现状同样没有上报),但避免与程序抢鼠标。
pub fn terminal_handles_mouse(mouse_reporting: bool, shift: bool) -> bool {
    !mouse_reporting || shift
}

/// 拖选锚点:按下的位置,用 grid 坐标(行号含历史区,可能为负)。
///
/// 记 grid 坐标而不是视口行:回看历史或内容滚动后,同一点击位置对应的 grid 行会变,
/// 锚点必须固定在按下时的内容上(alacritty 的 selection.rotate 会在滚动时同步移动端点)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragAnchor {
    /// grid 行号(0 是视口顶行,负数为历史行)。
    pub line: i32,
    /// 列号。
    pub col: usize,
    /// 落点在该单元格左半侧(含义见 [`CellPoint::side_left`])。
    pub side_left: bool,
}

/// 选区范围(行号用 grid 的 Line 值,可能是负数的历史行)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionSpan {
    /// 起始行(较小的一行)。
    pub start_line: i32,
    /// 起始列。
    pub start_col: usize,
    /// 结束行(较大的一行)。
    pub end_line: i32,
    /// 结束列(含)。
    pub end_col: usize,
    /// 矩形(块)选择。
    pub is_block: bool,
}

impl SelectionSpan {
    /// 某一行被选中的列区间(含首尾)。
    ///
    /// 普通选区首行从起始列开始、末行到结束列为止,中间行整行;
    /// 块选区每行都用相同的列区间。
    ///
    /// `None`:该行不在选区内。
    pub fn cols_on_line(&self, line: i32, cols: usize) -> Option<(usize, usize)> {
        if cols == 0 {
            return None;
        }
        let in_range = line >= self.start_line && line <= self.end_line;
        if !in_range {
            return None;
        }

        if self.is_block {
            let (a, b) = if self.start_col <= self.end_col {
                (self.start_col, self.end_col)
            } else {
                (self.end_col, self.start_col)
            };
            return Some((a.min(cols - 1), b.min(cols - 1)));
        }

        let start = if line == self.start_line {
            self.start_col.min(cols - 1)
        } else {
            0
        };
        let end = if line == self.end_line {
            self.end_col.min(cols - 1)
        } else {
            cols - 1
        };
        if start > end {
            return None;
        }
        Some((start, end))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用几何:画布左上角 (0, 32),单元格 8x16,10 行 80 列。
    fn geometry() -> GridGeometry {
        GridGeometry {
            origin_x: 0.0,
            origin_y: 32.0,
            cell_width: 8.0,
            cell_height: 16.0,
            rows: 10,
            cols: 80,
        }
    }

    /// 列/行映射:单元格内部任意位置都应落到该单元格
    #[test]
    fn maps_pixels_to_cell() {
        // 画布左上角 (0, 32),单元格 8x16
        let p = geometry().cell_at(3.0, 40.0).unwrap();
        assert_eq!((p.row, p.col), (0, 0));
        assert!(p.side_left);

        let p = geometry().cell_at(20.0, 72.0).unwrap();
        assert_eq!((p.row, p.col), (2, 2));
    }

    /// 落点靠右半侧时 side_left 为 false(决定选区端点是否包含该列)
    #[test]
    fn side_depends_on_half_cell() {
        let left_half = geometry().cell_at(1.0, 32.0).unwrap();
        assert!(left_half.side_left);
        let right_half = geometry().cell_at(7.0, 32.0).unwrap();
        assert!(!right_half.side_left);
    }

    /// 画布外钳制到边缘单元格(拖出内容区仍扩展选区)
    #[test]
    fn clamps_outside_area() {
        let top_left = geometry().cell_at(-100.0, -100.0).unwrap();
        assert_eq!((top_left.row, top_left.col), (0, 0));
        let bottom_right = geometry().cell_at(9999.0, 9999.0).unwrap();
        assert_eq!((bottom_right.row, bottom_right.col), (9, 79));
    }

    /// 无效尺寸(尚未测量列宽、空网格)返回 None
    #[test]
    fn rejects_invalid_geometry() {
        let zero_width = GridGeometry {
            cell_width: 0.0,
            ..geometry()
        };
        let zero_rows = GridGeometry {
            rows: 0,
            ..geometry()
        };
        assert_eq!(zero_width.cell_at(1.0, 1.0), None);
        assert_eq!(zero_rows.cell_at(1.0, 1.0), None);
        assert_eq!(geometry().cell_at(f32::NAN, 1.0), None);
        assert_eq!(geometry().cell_at(1.0, f32::INFINITY), None);
    }

    /// 普通选区:首行从起始列开始,中间整行,末行到结束列
    #[test]
    fn simple_span_per_line() {
        let span = SelectionSpan {
            start_line: 0,
            start_col: 5,
            end_line: 2,
            end_col: 3,
            is_block: false,
        };
        assert_eq!(span.cols_on_line(0, 10), Some((5, 9)));
        assert_eq!(span.cols_on_line(1, 10), Some((0, 9)));
        assert_eq!(span.cols_on_line(2, 10), Some((0, 3)));
        assert_eq!(span.cols_on_line(3, 10), None);
        assert_eq!(span.cols_on_line(-1, 10), None);
    }

    /// 块选区:每行相同的列区间
    #[test]
    fn block_span_uses_same_columns() {
        let span = SelectionSpan {
            start_line: 1,
            start_col: 7,
            end_line: 3,
            end_col: 2,
            is_block: true,
        };
        for line in 1..=3 {
            assert_eq!(span.cols_on_line(line, 10), Some((2, 7)));
        }
        assert_eq!(span.cols_on_line(0, 10), None);
    }

    /// MOUSE-001:程序开启鼠标上报且无 Shift 时,由程序处理
    #[test]
    fn mouse_001_program_owns_mouse_when_reporting() {
        assert!(!terminal_handles_mouse(true, false));
    }

    /// MOUSE-002:按住 Shift 时忽略上报状态,终端自行处理
    #[test]
    fn mouse_002_shift_forces_terminal_handling() {
        assert!(terminal_handles_mouse(true, true));
    }

    /// 程序未上报时终端总是自己处理
    #[test]
    fn terminal_handles_mouse_without_reporting() {
        assert!(terminal_handles_mouse(false, false));
        assert!(terminal_handles_mouse(false, true));
    }

    /// 单行单列选区只高亮一个单元格
    #[test]
    fn single_cell_span() {
        let span = SelectionSpan {
            start_line: 4,
            start_col: 6,
            end_line: 4,
            end_col: 6,
            is_block: false,
        };
        assert_eq!(span.cols_on_line(4, 10), Some((6, 6)));
    }
}
