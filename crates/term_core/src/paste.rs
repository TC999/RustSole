//! 粘贴文本处理。规格见 docs/ai/BEHAVIOR_SPEC.md PASTE-001 ~ PASTE-006。
//! 剪贴板与选区缓冲共用同一个入口(规格:两条通道共用一个处理函数)。

/// 按粘贴规则清洗一段文本。
///
/// 顺序依据规格:先做换行规范化,再过滤控制字符;
/// ESC 必须在判断 bracketed paste 之前移除,否则内容中的 ESC[200~ 会伪造包裹序列(PASTE-004)。
pub fn sanitize_paste(text: &str, bracketed_paste: bool) -> String {
    // PASTE-004:ESC 一律移除(0x1b),防止提前结束/伪造括号粘贴序列
    // PASTE-005:移除其余 C0 控制字符,仅保留 \t 与换行(换行下一步统一)
    let cleaned: String = text
        .chars()
        .filter(|&c| c == '\t' || c == '\n' || c == '\r' || !c.is_control())
        .collect();

    // PASTE-001:\r\n 与 \n 统一转换为 \r(终端换行语义)
    let normalized = cleaned
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r");

    if bracketed_paste {
        // PASTE-002:程序开启 bracketed paste 时用 200/201 包裹
        format!("\x1b[200~{}\x1b[201~", normalized)
    } else {
        normalized
    }
}

/// 判断一次粘贴是否需要多行确认对话框(PASTE-003)。
///
/// 仅当程序未开启 bracketed paste、文本含换行且确认开关开启时需要确认。
/// 抽成纯函数以便脱离图形环境测试。
pub fn needs_multiline_confirm(bracketed: bool, text: &str, confirm_enabled: bool) -> bool {
    !bracketed && confirm_enabled && text.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PASTE-001:\r\n 与 \n 统一转 \r
    #[test]
    fn paste_001_newline_normalization() {
        assert_eq!(sanitize_paste("a\r\nb\nc\rd", false), "a\rb\rc\rd");
    }

    /// PASTE-002:bracketed paste 包裹
    #[test]
    fn paste_002_bracketed_wrapping() {
        assert_eq!(sanitize_paste("hi", true), "\x1b[200~hi\x1b[201~");
    }

    /// PASTE-004:ESC 一律移除,包括试图伪造包裹序列的内容
    #[test]
    fn paste_004_esc_stripped() {
        // 内容自带结束序列,若不滤掉 ESC 会提前结束括号粘贴
        let evil = "a\x1b[201~b";
        assert_eq!(sanitize_paste(evil, true), "\x1b[200~a[201~b\x1b[201~");
    }

    /// PASTE-005:仅保留 \t 与换行,其余 C0 控制(如 \0 \x07 \x1b)移除
    #[test]
    fn paste_005_other_c0_removed() {
        assert_eq!(sanitize_paste("a\x00b\x07c\x1bd\te", false), "abcd\te");
    }

    /// 中文等多字节字符不受影响
    #[test]
    fn utf8_preserved() {
        assert_eq!(sanitize_paste("中文\r\n测试", false), "中文\r测试");
    }

    /// PASTE-003:仅在未开启 bracketed paste、含换行且开关开启时需要确认
    #[test]
    fn paste_003_needs_confirm_only_when_required() {
        assert!(needs_multiline_confirm(false, "a\nb", true));
        // bracketed paste 已开启:不需要
        assert!(!needs_multiline_confirm(true, "a\nb", true));
        // 不含换行:不需要
        assert!(!needs_multiline_confirm(false, "abc", true));
        // 确认开关关闭:不需要
        assert!(!needs_multiline_confirm(false, "a\nb", false));
    }
}
