//! 键盘输入编码:GPUI 按键 → 发给子进程的字节序列。
//! 规格见 docs/ai/BEHAVIOR_SPEC.md KEY-004 / KEY-005。
//! 序列依据 xterm 键盘编码(APP_CURSOR 切换 ESC O / ESC [ 形式)。

/// 编码所需的最小终端状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyState {
    /// 程序开启应用光标键模式(DECOM DECCKM,TermMode::APP_CURSOR)。
    pub app_cursor: bool,
    /// CapsLock 开启。注意:gpui Windows 的 key_char 不反映 CapsLock
    /// (generate_key_char 的 ToUnicode 状态表缺 VK_CAPITAL 位),需要在此补偿。
    pub caps_lock: bool,
}

/// 编码一个按键。
///
/// key:GPUI 的键名(如 "a"、"space"、"up");key_char:平台给出的该次按键产生的字符
/// (gpui Keystroke::key_char;Shift/CapsLock 下字母为大写,Shift 下标点为上档字符,无则 None)。
/// ctrl/alt/shift:修饰键状态。返回 None 表示该按键组合不产生输出(例如纯修饰键按键)。
///
/// KEY-004:Ctrl+C 在此函数层面编码为 0x03(中断字符),复制功能由快捷键绑定走别的 Action,
/// 不会进入本函数。
pub fn encode_key(
    key: &str,
    key_char: Option<&str>,
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
        // 空格:gpui 各平台把该键命名为 "space"(VK_SPACE => "space"),字符 0x20 由这里给出
        ("space", false, _) => Some(vec![0x20]),
        // Ctrl+Space → NUL(xterm 行为,IME 拦截与否由系统决定)
        ("space", true, _) => Some(vec![0x00]),
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
        // 单字符键:优先用 key_char(还原 Shift 产生的真实字符:大写字母、上档标点),
        // 无 key_char 时回退键名(key 名对单字符键就是小写字符)
        (k, false, _) if k.len() == 1 => {
            let ch = key_char
                .and_then(|s| s.chars().next())
                .filter(|c| !c.is_control())
                .unwrap_or_else(|| k.chars().next().expect("len==1 保证非空"));
            // CapsLock 补偿:gpui Windows 的 key_char 只反映 Shift 不反映 CapsLock,
            // 期望输出 = shift XOR caps ? 大写 : 小写(仅 ASCII 字母;标点不受 CapsLock 影响)。
            // 对 key_char 已反映 CapsLock 的平台(macOS/Linux),该变换幂等:大写 upper 后仍大写。
            let ch = if state.caps_lock && ch.is_ascii_alphabetic() {
                if shift {
                    ch.to_ascii_lowercase()
                } else {
                    ch.to_ascii_uppercase()
                }
            } else {
                ch
            };
            Some(ch.to_string().into_bytes())
        },
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
        let st = KeyState {
            app_cursor: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key("up", None, false, false, false, st).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key("down", None, false, false, false, st).unwrap(),
            b"\x1bOB"
        );
        assert_eq!(
            encode_key("right", None, false, false, false, st).unwrap(),
            b"\x1bOC"
        );
        assert_eq!(
            encode_key("left", None, false, false, false, st).unwrap(),
            b"\x1bOD"
        );
    }

    /// KEY-005:非 APP_CURSOR 下发 ESC [ A/B/C/D
    #[test]
    fn key_005_normal_cursor_sequences() {
        let st = KeyState::default();
        assert_eq!(
            encode_key("up", None, false, false, false, st).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key("down", None, false, false, false, st).unwrap(),
            b"\x1b[B"
        );
        assert_eq!(
            encode_key("right", None, false, false, false, st).unwrap(),
            b"\x1b[C"
        );
        assert_eq!(
            encode_key("left", None, false, false, false, st).unwrap(),
            b"\x1b[D"
        );
    }

    /// KEY-004:Ctrl+C 编码为中断字符 0x03
    #[test]
    fn key_004_ctrl_c_is_interrupt() {
        assert_eq!(
            encode_key("c", None, true, false, false, KeyState::default()).unwrap(),
            vec![0x03]
        );
    }

    /// Ctrl+字母 → 控制字符
    #[test]
    fn ctrl_letters_to_control_chars() {
        assert_eq!(
            encode_key("a", None, true, false, false, KeyState::default()).unwrap(),
            vec![0x01]
        );
        assert_eq!(
            encode_key("d", None, true, false, false, KeyState::default()).unwrap(),
            vec![0x04]
        );
    }

    /// Enter / Backspace / Tab
    #[test]
    fn basic_keys() {
        let st = KeyState::default();
        assert_eq!(encode_key("enter", None, false, false, false, st).unwrap(), b"\r");
        assert_eq!(
            encode_key("backspace", None, false, false, false, st).unwrap(),
            vec![0x7f]
        );
        assert_eq!(encode_key("tab", None, false, false, false, st).unwrap(), b"\t");
        // Shift+Tab 反向制表
        assert_eq!(
            encode_key("tab", None, false, false, true, st).unwrap(),
            b"\x1b[Z"
        );
    }

    /// Alt 前缀 ESC
    #[test]
    fn alt_prefixes_esc() {
        let st = KeyState::default();
        assert_eq!(encode_key("x", None, false, true, false, st).unwrap(), b"\x1bx");
    }

    /// 空格键(gpui 命名 "space")→ 0x20
    #[test]
    fn space_key_sends_0x20() {
        let st = KeyState::default();
        assert_eq!(encode_key("space", None, false, false, false, st).unwrap(), vec![0x20]);
        // Alt+Space 前缀 ESC
        assert_eq!(encode_key("space", None, false, true, false, st).unwrap(), b"\x1b ");
    }

    /// Shift/CapsLock 下字母用 key_char 的大写(修复:此前总输出小写)
    #[test]
    fn uppercase_via_key_char() {
        let st = KeyState::default();
        assert_eq!(
            encode_key("a", Some("A"), false, false, true, st).unwrap(),
            b"A"
        );
        // 无 shift 且无 key_char:回退键名小写
        assert_eq!(
            encode_key("a", None, false, false, false, st).unwrap(),
            b"a"
        );
    }

    /// Shift+标点用 key_char 的上档字符
    #[test]
    fn shifted_punct_via_key_char() {
        let st = KeyState::default();
        // Shift+, 在 US 布局产生 "<"
        assert_eq!(
            encode_key(",", Some("<"), false, false, true, st).unwrap(),
            b"<"
        );
    }

    /// CapsLock 开:无 shift 字母变大写(修复:gpui Windows key_char 不反映 CapsLock)
    #[test]
    fn capslock_uppercases_letters() {
        let st = KeyState {
            caps_lock: true,
            ..Default::default()
        };
        // gpui 给的 key_char 是小写(缺 caps 位),编码后应为大写
        assert_eq!(
            encode_key("a", Some("a"), false, false, false, st).unwrap(),
            b"A"
        );
    }

    /// CapsLock 开 + Shift:字母变小写(标准键盘行为)
    #[test]
    fn capslock_with_shift_lowercases() {
        let st = KeyState {
            caps_lock: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key("a", Some("A"), false, false, true, st).unwrap(),
            b"a"
        );
    }

    /// CapsLock 不影响标点与数字
    #[test]
    fn capslock_ignores_non_letters() {
        let st = KeyState {
            caps_lock: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key("1", Some("1"), false, false, false, st).unwrap(),
            b"1"
        );
    }

    /// 不产生输出的组合返回 None
    #[test]
    fn no_output_combinations() {
        let st = KeyState::default();
        assert_eq!(encode_key("shift", None, false, false, true, st), None);
        assert_eq!(
            encode_key("unknown-mediacmd", None, false, false, false, st),
            None
        );
    }
}