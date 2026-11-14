//! 界面主题:窗口装饰与终端区域的统一配色。
//!
//! 现状:阶段 1 没有配置系统(WORKFLOW.md 阶段 5 才引入配色方案),这里的常量是
//! 单一暗色主题的**唯一来源**。组件内不得再硬编码颜色(CODING_STANDARDS.md 第 6 节);
//! 后续接入配置时只需把本模块换成 `Arc<Theme>` 快照,读取点不用动。

/// 标题栏与窗口控制按钮配色。
pub mod window {
    /// 标题栏底色(比终端底色明显亮一档,形成可辨认的分界)。
    pub const TITLEBAR_BG: u32 = 0x1f1f1f;
    /// 标题栏下边线(分隔标题栏与终端内容)。
    pub const TITLEBAR_BORDER: u32 = 0x333333;
    /// 窗口激活时的标题文字颜色。
    pub const TITLE_TEXT: u32 = 0xd0d0d0;
    /// 窗口未激活时的标题文字颜色。
    pub const TITLE_TEXT_INACTIVE: u32 = 0x8a8a8a;
    /// 窗口控制按钮的图标颜色。
    pub const CONTROL_ICON: u32 = 0xb0b0b0;
    /// 窗口控制按钮的悬停底色。
    pub const CONTROL_HOVER_BG: u32 = 0x2e2e2e;
    /// 关闭按钮的悬停底色(危险动作与其余按钮区分)。
    pub const CONTROL_CLOSE_HOVER_BG: u32 = 0xc42b1c;
    /// 关闭按钮悬停时的图标颜色。
    pub const CONTROL_CLOSE_HOVER_ICON: u32 = 0xffffff;
    /// 窗口整体底色(内容未覆盖处,避免闪白)。
    pub const WINDOW_BG: u32 = 0x101010;
}

/// 终端内容区配色。
pub mod terminal {
    /// 终端画布底色。
    pub const BG: u32 = 0x101010;
    /// 滚动条轨道底色。
    pub const SCROLLBAR_TRACK: u32 = 0x181818;
    /// 滚动条 thumb 常态色。
    pub const SCROLLBAR_THUMB: u32 = 0x555555;
}

/// 标题栏高度(px)。与窗口控制按钮宽度一致时按钮是正方形热区。
pub const TITLEBAR_HEIGHT: f32 = 32.0;

/// 窗口控制按钮宽度(px)。
pub const WINDOW_CONTROL_WIDTH: f32 = 46.0;

/// 标题栏上显示哪些窗口控制按钮(由平台能力决定)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowControlsShown {
    /// 显示最小化按钮。
    pub minimize: bool,
    /// 显示最大化/还原按钮。
    pub maximize: bool,
}

/// 窗口控制按钮种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControl {
    /// 最小化。
    Minimize,
    /// 最大化 / 还原。
    Maximize,
    /// 关闭。
    Close,
}
