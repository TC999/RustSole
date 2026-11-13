//! 平台差异层。PTY 已由 alacritty_terminal::tty 内聚(cfg 在其内部),
//! 本 crate 承担剩余的平台差异:默认字体等。
//! 所有 `cfg(target_os)` 集中在本 crate(硬性规则 6),对外只暴露统一接口(ARCHITECTURE.md 不变量 6)。

/// 平台相关的默认值集合。
#[derive(Debug, Clone, Copy)]
pub struct PlatformInfo {
    /// 默认等宽字体族名。来源:各平台系统自带等宽字体。
    pub default_font_family: &'static str,
}

/// 返回当前平台的默认信息。
pub fn platform_info() -> PlatformInfo {
    PlatformInfo {
        default_font_family: default_font_family(),
    }
}

#[cfg(target_os = "windows")]
fn default_font_family() -> &'static str {
    // Windows 系统自带等宽字体
    "Consolas"
}

#[cfg(not(target_os = "windows"))]
fn default_font_family() -> &'static str {
    // Linux/macOS:fontconfig 在多数发行版提供 monospace 别名,交给字体系统解析
    "monospace"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认字体族名非空(两平台编译都需满足)
    #[test]
    fn default_font_family_non_empty() {
        assert!(!platform_info().default_font_family.is_empty());
    }
}
