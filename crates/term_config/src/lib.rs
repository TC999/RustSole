//! 配置层:结构、解析、分层合并、校验。
//!
//! 当前仅承载阶段 2 选择/粘贴所需的开关;文件解析与热重载在后续阶段接入。
//! 视图持有 `Arc` 快照,热重载时整体替换(ARCHITECTURE.md 不变量 9)。

/// 终端配置快照。
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub selection: SelectionConfig,
    pub paste: PasteConfig,
}

/// 选择相关配置。
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionConfig {
    /// SEL-008:选中时额外写入系统剪贴板(默认关闭)。选区缓冲行为不变。
    pub copy_on_select: bool,
}

/// 粘贴相关配置。
#[derive(Debug, Clone, PartialEq)]
pub struct PasteConfig {
    /// PASTE-003:未开启 bracketed paste 且文本含换行时弹出确认对话框(默认开启)。
    pub confirm_multiline: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            selection: SelectionConfig {
                copy_on_select: false,
            },
            paste: PasteConfig {
                confirm_multiline: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEL-008:copy_on_select 默认关闭;PASTE-003:多行确认默认开启
    #[test]
    fn defaults_match_spec() {
        let config = Config::default();
        assert!(!config.selection.copy_on_select);
        assert!(config.paste.confirm_multiline);
    }
}
