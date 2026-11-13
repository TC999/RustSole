//! 键盘输入编码:GPUI 按键 → 发给子进程的字节序列。
//! 规格见 docs/ai/BEHAVIOR_SPEC.md KEY-004 / KEY-005。
//! 序列依据 xterm 键盘编码(APP_CURSOR 切换 ESC O / ESC [ 形式)。

/// 编码所需的最小终端状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyState {
    /// 程序开启应用光标键模式(DECOM DECCKM,TermMode::APP_CURSOR)。
    pub app_cursor: bool,
}

/// 编码一个按键。
///
/// key:GPUI 的键名(如 "a"、"enter"、"up");ctrl/alt/shift:修饰键状态。
/// 返回 None 表示该按键组合不产生输出(例如纯 Shift 按键)。
///
/// KEY-004:Ctrl+C 在此函数层面编码为 0x03(中断字符),复制功能由快捷键绑定走别的 Action,
/// 不会进入本函数。
pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    state: KeyState,
) -> Option<Vec<u8>> {
    // xterm 规则:Alt 前缀 ESC
    let prefix: &[u8] = if alt { &[0x1b] } else { &[] };

    let seq: Option<Vec<u8>> = match (key, ctrl, shift) {
        // 控制字符:ctrl+字母 → 0x01..0x1a
        (k, true, _) if k.len() == 1 => {
            let c = k.as_bytes()[0];
            let base = c.to_ascii_lowercase();
            if base.is_ascii_lowercase() {
                Some(vec![base - b'a' + 1])
            } else {
                // ctrl+其他单字符不编码
                None
            }
        }
        ("enter", ..) => Some(vec![b'\r']),
        ("backspace", ..) => Some(vec![0x7f]),
        ("tab", false, false) => Some(vec![b'\t']),
        ("tab", false, true) => Some(b"\x1b[Z".to_vec()),
        ("escape", ..) => Some(vec![0x1b]),
        // 方向键:KEY-005,APP_CURSOR 用 ESC O,否则 ESC [
        ("up", false, _) if state.app_cursor => Some(b"\x1bOA".to_vec()),
        ("down", false, _) if state.app_cursor => Some(b"\x1bOB".to_vec()),
        ("right", false, _) if state.app_cursor => Some(b"\x1bOC".to_vec()),
        ("left", false, _) if state.app_cursor => Some(b"\x1bOD".to_vec()),
        ("up", false, _) => Some(b"\x1b[A".to_vec()),
        ("down", false, _) => Some(b"\x1b[B".to_vec()),
        ("right", false, _) => Some(b"\x1b[C".to_vec()),
        ("left", false, _) => Some(b"\x1b[D".to_vec()),
        ("home", false, _) => Some(b"\x1b[H".to_vec()),
        ("end", false, _) => Some(b"\x1b[F".to_vec()),
        ("pageup", false, _) => Some(b"\x1b[5~".to_vec()),
        ("pagedown", false, _) => Some(b"\x1b[6~".to_vec()),
        ("delete", false, _) => Some(b"\x1b[3~".to_vec()),
        ("insert", false, _) => Some(b"\x1b[2~".to_vec()),
        // 功能键 F1-F4(xterm:ss 约定用 ESC O P..S)
        ("f1", false, _) => Some(b"\x1bOP".to_vec()),
        ("f2", false, _) => Some(b"\x1bOQ".to_vec()),
        ("f3", false, _) => Some(b"\x1bOR".to_vec()),
        ("f4", false, _) => Some(b"\x1bOS".to_vec()),
        // 单个可打印字符(修饰键不含 ctrl/alt 时由 key_char 路径处理,这里兜底)
        (k, false, _) if k.len() == 1 => Some(k.as_bytes().to_vec()),
        _ => None,
    };

    seq.map(|mut s| {
        s.splice(0..0, prefix.iter().copied());
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KEY-005:方向键在 APP_CURSOR 下发 ESC O A/B/C/D
    #[test]
    fn key_005_app_cursor_sequences() {
        let st = KeyState { app_cursor: true };
        assert_eq!(
            encode_key("up", false, false, false, st).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key("down", false, false, false, st).unwrap(),
            b"\x1bOB"
        );
        assert_eq!(
            encode_key("right", false, false, false, st).unwrap(),
            b"\x1bOC"
        );
        assert_eq!(
            encode_key("left", false, false, false, st).unwrap(),
            b"\x1bOD"
        );
    }

    /// KEY-005:非 APP_CURSOR 下发 ESC [ A/B/C/D
    #[test]
    fn key_005_normal_cursor_sequences() {
        let st = KeyState::default();
        assert_eq!(
            encode_key("up", false, false, false, st).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key("down", false, false, false, st).unwrap(),
            b"\x1b[B"
        );
        assert_eq!(
            encode_key("right", false, false, false, st).unwrap(),
            b"\x1b[C"
        );
        assert_eq!(
            encode_key("left", false, false, false, st).unwrap(),
            b"\x1b[D"
        );
    }

    /// KEY-004:Ctrl+C 编码为中断字符 0x03
    #[test]
    fn key_004_ctrl_c_is_interrupt() {
        assert_eq!(
            encode_key("c", true, false, false, KeyState::default()).unwrap(),
            vec![0x03]
        );
    }

    /// Ctrl+字母 → 控制字符
    #[test]
    fn ctrl_letters_to_control_chars() {
        assert_eq!(
            encode_key("a", true, false, false, KeyState::default()).unwrap(),
            vec![0x01]
        );
        assert_eq!(
            encode_key("d", true, false, false, KeyState::default()).unwrap(),
            vec![0x04]
        );
    }

    /// Enter / Backspace / Tab
    #[test]
    fn basic_keys() {
        let st = KeyState::default();
        assert_eq!(encode_key("enter", false, false, false, st).unwrap(), b"\r");
        assert_eq!(
            encode_key("backspace", false, false, false, st).unwrap(),
            vec![0x7f]
        );
        assert_eq!(encode_key("tab", false, false, false, st).unwrap(), b"\t");
        // Shift+Tab 反向制表
        assert_eq!(
            encode_key("tab", false, false, true, st).unwrap(),
            b"\x1b[Z"
        );
    }

    /// Alt 前缀 ESC
    #[test]
    fn alt_prefixes_esc() {
        let st = KeyState::default();
        assert_eq!(encode_key("x", false, true, false, st).unwrap(), b"\x1bx");
    }

    /// 不产生输出的组合返回 None
    #[test]
    fn no_output_combinations() {
        let st = KeyState::default();
        assert_eq!(encode_key("shift", false, false, true, st), None);
        assert_eq!(
            encode_key("unknown-mediacmd", false, false, false, st),
            None
        );
    }
}
