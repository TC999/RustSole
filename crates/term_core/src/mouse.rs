//! 鼠标上报编码(MOUSE-001)与备用屏滚轮决策(MOUSE-003)。
//!
//! 程序开启鼠标上报模式(vim mouse=a、tmux、htop)后,鼠标事件不再用于选区,
//! 而是编码成转义序列发给程序。编码格式以 xterm 协议为准,与 alacritty 主仓
//! alacritty/src/input/mod.rs 的实现一致;TermMode 位已在锁定版本
//! alacritty_terminal 0.26.0 的 src/term/mod.rs:60-80 查证。
//!
//! 纯函数、不依赖 gpui:决策与编码可脱离图形环境单测(TESTING.md 第 1 节原则 2)。

use alacritty_terminal::term::TermMode;

/// 终端内容区里的一次鼠标动作(已从像素换算成单元格坐标)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseReport {
    /// grid 行号:0 是视口顶行,负数为历史行。上报只取可见区,调用方先过滤。
    pub line: i32,
    /// 列号(0 起)。
    pub col: usize,
    /// 落点在单元格左半侧(与选区端点语义一致,决定 SGR 上报的列偏移)。
    pub side_left: bool,
    /// 修饰键状态。
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// 鼠标动作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// 按下(不含滚轮,滚轮单独建模)。
    Press(MouseButton),
    /// 松开。
    Release(MouseButton),
    /// 按住移动(拖动)。button 表示当前按住的键。
    Drag(MouseButton),
    /// 无按键时的移动(悬停)。
    Motion,
}

/// 上报支持的按键。gpui 的 Navigate(前进/后退)无法映射到协议,不在此列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// 滚轮方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelDirection {
    Up,
    Down,
}

/// 按键 → 协议按钮码(xterm:左 0、中 1、右 2)。
fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// 动作 → 基础按钮码。释放一律用码 3(xterm 约定:普通协议的释放不带键号),
/// 拖动在按下码上 +32。悬停移动在非 SGR 模式下不可表达,返回 None。
fn base_code(action: MouseAction) -> Option<u8> {
    match action {
        MouseAction::Press(b) => Some(button_code(b)),
        MouseAction::Release(_) => Some(3),
        MouseAction::Drag(b) => Some(button_code(b) + 32),
        MouseAction::Motion => None,
    }
}

/// 修饰键位:xterm 协议 Shift=4、Alt=8、Ctrl=16,可叠加。
fn modifier_bits(shift: bool, alt: bool, control: bool) -> u8 {
    (shift as u8) * 4 + (alt as u8) * 8 + (control as u8) * 16
}

/// 决策:这次动作是否要上报给程序。
///
/// MOUSE-001:程序开启鼠标上报时,按下/释放/移动归程序;
/// MOUSE-002(Shift 强制终端处理)由调用方先行判断,本函数不含该分支。
/// 拖动上报需要 MOUSE_DRAG 位(tmux 拖选),悬停上报需要 MOUSE_MOTION 位。
pub fn should_report(mode: TermMode, action: MouseAction) -> bool {
    let reporting = mode.intersects(TermMode::MOUSE_MODE);
    match action {
        MouseAction::Press(_) | MouseAction::Release(_) => reporting,
        MouseAction::Drag(_) => reporting && mode.contains(TermMode::MOUSE_DRAG),
        MouseAction::Motion => reporting && mode.contains(TermMode::MOUSE_MOTION),
    }
}

/// 编码一次鼠标上报。
///
/// 返回 None:不该上报、悬停移动在非 SGR 模式下不可表达,或坐标落在回看
/// 历史区(line < 0)—— 协议只报可见屏幕。
///
/// 三种协议的取舍(与 alacritty 一致):
/// - SGR(ESC[<b;x;yM/m):现代程序默认,坐标不受 223 限制;
/// - X10 普通(ESC[M+3c):坐标 +33 编码为单字节,超过 223 丢弃;
/// - UTF-8 扩展:坐标字节超过 127 时用两字节续编,上限 2015。
pub fn encode_mouse_report(
    mode: TermMode,
    action: MouseAction,
    report: MouseReport,
) -> Option<Vec<u8>> {
    if !should_report(mode, action) {
        return None;
    }
    // 回看历史时的行在可见屏幕之外,不上报(alacritty: point.line < 0 直接返回)
    if report.line < 0 {
        return None;
    }

    // SGR 下悬停移动用码 35;SGR 的释放帧保留原按键码(协议 1006 用 M/m 区分按下与释放),
    // 码 3 只在普通协议里表示"释放";非 SGR 悬停不可表达
    let base = match action {
        MouseAction::Motion if mode.contains(TermMode::SGR_MOUSE) => 35,
        MouseAction::Release(b) if mode.contains(TermMode::SGR_MOUSE) => button_code(b),
        other => base_code(other)?,
    };
    let button = base + modifier_bits(report.shift, report.alt, report.control);

    // SGR 用 1 起始坐标;右半侧落点算下一列,与选区端点的 Side 语义一致
    let col_offset = if report.side_left { 0 } else { 1 };
    let column = report.col + 1 + col_offset;
    let line = report.line as usize + 1;

    if mode.contains(TermMode::SGR_MOUSE) {
        // 按下/移动 'M',释放 'm'
        let c = match action {
            MouseAction::Release(_) => 'm',
            _ => 'M',
        };
        Some(format!("\x1b[<{button};{column};{line}{c}").into_bytes())
    } else {
        // 普通/UTF-8 协议:ESC [ M <cb> <cx> <cy>,坐标字节从 33 起
        let utf8 = mode.contains(TermMode::UTF8_MOUSE);
        let max = if utf8 { 2015 } else { 223 };
        if column > max || line > max {
            return None;
        }
        let mut msg = vec![0x1b, b'[', b'M', 32 + button];
        push_coord(32 + column as u16, utf8, &mut msg);
        push_coord(32 + line as u16, utf8, &mut msg);
        Some(msg)
    }
}

/// 编码滚轮上报(MOUSE-001:程序接管鼠标时滚轮也归程序)。
///
/// 滚轮在协议里是"按下即松开"的按钮 64(上)/ 65(下);普通协议只发按下帧
/// (释放帧码 3 无法区分滚轮,与 alacritty 一致)。
pub fn encode_wheel_report(
    mode: TermMode,
    direction: WheelDirection,
    report: MouseReport,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) || report.line < 0 {
        return None;
    }
    let code = match direction {
        WheelDirection::Up => 64,
        WheelDirection::Down => 65,
    } + modifier_bits(report.shift, report.alt, report.control);
    let col_offset = if report.side_left { 0 } else { 1 };
    let column = report.col + 1 + col_offset;
    let line = report.line as usize + 1;
    if mode.contains(TermMode::SGR_MOUSE) {
        Some(format!("\x1b[<{code};{column};{line}M").into_bytes())
    } else {
        let utf8 = mode.contains(TermMode::UTF8_MOUSE);
        let max = if utf8 { 2015 } else { 223 };
        if column > max || line > max {
            return None;
        }
        let mut msg = vec![0x1b, b'[', b'M', 32 + code];
        push_coord(32 + column as u16, utf8, &mut msg);
        push_coord(32 + line as u16, utf8, &mut msg);
        Some(msg)
    }
}

/// 鼠标坐标字节:值 < 128 直接单字节;UTF-8 扩展协议下再用两字节续编。
fn push_coord(value: u16, utf8: bool, out: &mut Vec<u8>) {
    if value < 128 || !utf8 {
        out.push(value as u8);
    } else {
        out.push((0xC0 + (value / 64)) as u8);
        out.push((0x80 + (value % 64)) as u8);
    }
}

/// 备用屏滚轮决策(MOUSE-003):备用屏(less/man)且程序开启 ALTERNATE_SCROLL
/// 时,滚轮转为方向键序列;否则返回 None 由终端回看历史。
///
/// Shift 强制终端回看(MOUSE-002 的滚轮形式,与 alacritty 一致)由调用方判断。
/// 方向键固定用应用光标形式 ESC O A/B:该模式只在备用屏生效,而备用屏全屏程序
/// (less/most)期望应用光标序列;主屏滚轮不走这里,与 KEY-005 的 DECCKM 分流不冲突。
pub fn alternate_scroll_sequence(
    mode: TermMode,
    direction: WheelDirection,
    lines: u32,
) -> Option<Vec<u8>> {
    let on_alt_screen =
        mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL);
    if !on_alt_screen || lines == 0 {
        return None;
    }
    let cmd = match direction {
        WheelDirection::Up => b'A',
        WheelDirection::Down => b'B',
    };
    // 钳到 64:正常滚轮一次折算不过几十行,防御异常大增量把 PTY 撑爆
    let lines = lines.min(64);
    let mut out = Vec::with_capacity(3 * lines as usize);
    for _ in 0..lines {
        out.extend_from_slice(&[0x1b, b'O', cmd]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLICK: TermMode = TermMode::MOUSE_REPORT_CLICK;
    const SGR: TermMode = TermMode::MOUSE_REPORT_CLICK.union(TermMode::SGR_MOUSE);
    const DRAG: TermMode = TermMode::MOUSE_REPORT_CLICK.union(TermMode::MOUSE_DRAG);
    const MOTION: TermMode = TermMode::MOUSE_MOTION.union(TermMode::SGR_MOUSE);

    fn report(line: i32, col: usize) -> MouseReport {
        MouseReport {
            line,
            col,
            side_left: true,
            shift: false,
            alt: false,
            control: false,
        }
    }

    /// MOUSE-001:开启点击上报时按下被编码为 SGR 序列
    #[test]
    fn mouse_001_sgr_press_sequence() {
        let msg =
            encode_mouse_report(SGR, MouseAction::Press(MouseButton::Left), report(0, 4)).unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<0;5;1M");
    }

    /// MOUSE-001:释放用 'm' 结尾
    #[test]
    fn mouse_001_sgr_release_sequence() {
        let msg = encode_mouse_report(SGR, MouseAction::Release(MouseButton::Left), report(2, 9))
            .unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<0;10;3m");
    }

    /// 右半侧落点算下一列(与选区端点的 Side 语义一致)
    #[test]
    fn sgr_right_half_counts_next_column() {
        let r = MouseReport {
            side_left: false,
            ..report(0, 4)
        };
        let msg = encode_mouse_report(SGR, MouseAction::Press(MouseButton::Left), r).unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<0;6;1M");
    }

    /// 修饰键位:Shift=4、Alt=8、Ctrl=16,可叠加(xterm 协议)
    #[test]
    fn sgr_modifier_bits() {
        let r = MouseReport {
            control: true,
            alt: true,
            ..report(0, 0)
        };
        let msg = encode_mouse_report(SGR, MouseAction::Press(MouseButton::Middle), r).unwrap();
        // 中键 1 + Ctrl 16 + Alt 8 = 25
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<25;1;1M");
    }

    /// 中键码 1、右键码 2
    #[test]
    fn sgr_button_codes() {
        let msg = encode_mouse_report(SGR, MouseAction::Press(MouseButton::Middle), report(0, 0))
            .unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<1;1;1M");
        let msg =
            encode_mouse_report(SGR, MouseAction::Press(MouseButton::Right), report(0, 0)).unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<2;1;1M");
    }

    /// 普通(X10)协议:ESC [ M cb cx cy,坐标字节 +33;释放按钮码为 3
    #[test]
    fn x10_press_and_release() {
        let msg = encode_mouse_report(CLICK, MouseAction::Press(MouseButton::Left), report(0, 4))
            .unwrap();
        assert_eq!(msg, vec![0x1b, b'[', b'M', 32, 32 + 5, 32 + 1]);
        let msg = encode_mouse_report(CLICK, MouseAction::Release(MouseButton::Left), report(0, 4))
            .unwrap();
        assert_eq!(msg, vec![0x1b, b'[', b'M', 32 + 3, 32 + 5, 32 + 1]);
    }

    /// 拖动在按下码上 +32;仅 MOUSE_DRAG 模式下才上报
    #[test]
    fn drag_code_32_and_mode_gate() {
        // 只开 CLICK,不上报拖动
        assert_eq!(
            encode_mouse_report(CLICK, MouseAction::Drag(MouseButton::Left), report(0, 0)),
            None
        );
        // 开了 MOUSE_DRAG(非 SGR):X10 格式,拖动码 = 32 + 0 + 32 = '@'
        let msg =
            encode_mouse_report(DRAG, MouseAction::Drag(MouseButton::Left), report(1, 2)).unwrap();
        assert_eq!(msg, vec![0x1b, b'[', b'M', 32 + 32, 32 + 3, 32 + 2]);
        // SGR + MOUSE_DRAG:拖动帧 <32
        let sgr_drag = DRAG.union(TermMode::SGR_MOUSE);
        let msg = encode_mouse_report(sgr_drag, MouseAction::Drag(MouseButton::Left), report(1, 2))
            .unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<32;3;2M");
    }

    /// 悬停移动:仅 SGR + MOUSE_MOTION 下编码为码 35;其他情况 None
    #[test]
    fn motion_only_in_sgr_motion_mode() {
        let msg = encode_mouse_report(MOTION, MouseAction::Motion, report(0, 0)).unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<35;1;1M");
        assert_eq!(
            encode_mouse_report(SGR, MouseAction::Motion, report(0, 0)),
            None
        );
        assert_eq!(
            encode_mouse_report(CLICK, MouseAction::Motion, report(0, 0)),
            None
        );
    }

    /// 回看历史区的行不上报(协议只报可见屏幕)
    #[test]
    fn scrollback_lines_not_reported() {
        let r = report(-3, 0);
        assert_eq!(
            encode_mouse_report(SGR, MouseAction::Press(MouseButton::Left), r),
            None
        );
        assert_eq!(encode_wheel_report(SGR, WheelDirection::Up, r), None);
    }

    /// 普通(非 SGR)协议的悬停移动不可表达
    #[test]
    fn motion_not_expressible_without_sgr() {
        let motion_only_plain = TermMode::MOUSE_MOTION;
        assert_eq!(
            encode_mouse_report(motion_only_plain, MouseAction::Motion, report(0, 0)),
            None
        );
    }

    /// UTF-8 扩展协议:坐标字节 >= 128 用两字节续编
    #[test]
    fn utf8_mouse_encodes_wide_coordinates() {
        let mode = CLICK.union(TermMode::UTF8_MOUSE);
        let r = MouseReport {
            col: 100,
            ..report(0, 0)
        };
        let msg = encode_mouse_report(mode, MouseAction::Press(MouseButton::Left), r).unwrap();
        // 列 100 → 字节值 32+1+100=133 → 0xC0+2, 0x80+5
        assert_eq!(msg, vec![0x1b, b'[', b'M', 32, 0xC2, 0x85, 32 + 1]);
    }

    /// X10 协议坐标超过 223 时丢弃(避免字节回绕)
    #[test]
    fn x10_clamps_beyond_223() {
        let r = MouseReport {
            col: 300,
            ..report(0, 0)
        };
        assert_eq!(
            encode_mouse_report(CLICK, MouseAction::Press(MouseButton::Left), r),
            None
        );
    }

    /// 滚轮:SGR 模式编码为按钮 64/65
    #[test]
    fn mouse_001_wheel_sgr() {
        let up = encode_wheel_report(SGR, WheelDirection::Up, report(0, 0)).unwrap();
        assert_eq!(String::from_utf8(up).unwrap(), "\x1b[<64;1;1M");
        let down = encode_wheel_report(SGR, WheelDirection::Down, report(0, 0)).unwrap();
        assert_eq!(String::from_utf8(down).unwrap(), "\x1b[<65;1;1M");
    }

    /// 滚轮:X10 协议只发按下帧
    #[test]
    fn mouse_001_wheel_x10() {
        let msg = encode_wheel_report(CLICK, WheelDirection::Down, report(1, 2)).unwrap();
        assert_eq!(msg, vec![0x1b, b'[', b'M', 32 + 65, 32 + 3, 32 + 2]);
    }

    /// 滚轮:修饰键进入按钮码(Shift+滚轮 = 64+4)
    #[test]
    fn mouse_001_wheel_with_shift() {
        let r = MouseReport {
            shift: true,
            ..report(0, 0)
        };
        let msg = encode_wheel_report(SGR, WheelDirection::Up, r).unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "\x1b[<68;1;1M");
    }

    /// 程序未开启鼠标上报时,滚轮不归程序
    #[test]
    fn wheel_ignored_without_mouse_mode() {
        assert_eq!(
            encode_wheel_report(TermMode::empty(), WheelDirection::Up, report(0, 0)),
            None
        );
    }

    /// MOUSE-003:备用屏 + ALTERNATE_SCROLL 时滚轮转方向键(ESC O A/B),按行数重复
    #[test]
    fn mouse_003_alternate_scroll_to_arrows() {
        let mode = TermMode::ALT_SCREEN.union(TermMode::ALTERNATE_SCROLL);
        let up = alternate_scroll_sequence(mode, WheelDirection::Up, 3).unwrap();
        assert_eq!(up, b"\x1bOA\x1bOA\x1bOA".to_vec());
        let down = alternate_scroll_sequence(mode, WheelDirection::Down, 1).unwrap();
        assert_eq!(down, b"\x1bOB".to_vec());
    }

    /// MOUSE-003:缺 ALT_SCREEN 或 ALTERNATE_SCROLL 任一位则不转换
    #[test]
    fn mouse_003_requires_alt_screen_and_alternate_scroll() {
        assert_eq!(
            alternate_scroll_sequence(TermMode::ALT_SCREEN, WheelDirection::Up, 1),
            None
        );
        assert_eq!(
            alternate_scroll_sequence(TermMode::ALTERNATE_SCROLL, WheelDirection::Up, 1),
            None
        );
    }

    /// MOUSE-003:0 行(增量不足一行)不发送
    #[test]
    fn mouse_003_zero_lines_sends_nothing() {
        let mode = TermMode::ALT_SCREEN.union(TermMode::ALTERNATE_SCROLL);
        assert_eq!(alternate_scroll_sequence(mode, WheelDirection::Up, 0), None);
    }
}
