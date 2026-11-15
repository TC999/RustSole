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
    /// 选区高亮底色(以半透明叠加在单元格背景上,见 view::paint_selection)。
    pub const SELECTION_BG: u32 = 0x4a6d8c;
}

/// 标签栏配色。标签栏与窗口按钮共处标题栏一行(Konsole 默认布局),
/// 因此底色与标题栏区分不明显,靠分隔线与文字亮度表达激活状态。
pub mod tabbar {
    /// 标签栏底色(未激活标签与空白处的底色)。
    pub const BG: u32 = 0x1f1f1f;
    /// 激活标签的底色(与终端内容一致,视觉上"连成一片")。
    pub const ACTIVE_BG: u32 = 0x101010;
    /// 未激活标签悬停时的底色。
    pub const HOVER_BG: u32 = 0x2e2e2e;
    /// 激活标签的文字颜色。
    pub const ACTIVE_TEXT: u32 = 0xd0d0d0;
    /// 未激活标签的文字颜色。
    pub const INACTIVE_TEXT: u32 = 0x8a8a8a;
    /// 标签之间的分隔线颜色。
    pub const SEPARATOR: u32 = 0x333333;
    /// 「新建标签」按钮的图标颜色。
    pub const NEW_TAB_ICON: u32 = 0xb0b0b0;
    /// 子进程已退出、但标签尚未关闭时的文字颜色(提示用户该标签已无进程)。
    pub const EXITED_TEXT: u32 = 0x7a7a7a;
    /// 标签上关闭图标的常态颜色。
    pub const CLOSE_ICON: u32 = 0x9a9a9a;
    /// 标签上关闭图标悬停时的颜色(提及可点击,与常态拉开对比)。
    pub const CLOSE_ICON_HOVER: u32 = 0xffffff;
    /// 标签上关闭图标悬停时的圆形底色。
    pub const CLOSE_ICON_HOVER_BG: u32 = 0x4a4a4a;
}

/// 应用工具栏配色。工具栏位于标签条与终端内容之间,底色比终端亮一档,
/// 与标签条同属"窗口装饰"层,视觉上连成一片。
pub mod toolbar {
    /// 工具栏底色。
    pub const BG: u32 = 0x242424;
    /// 工具栏下边线(分隔工具栏与终端内容)。
    pub const BORDER: u32 = 0x333333;
    /// 可用按钮的图标颜色。
    pub const ICON: u32 = 0xc0c0c0;
    /// 可用按钮悬停时的底色。
    pub const HOVER_BG: u32 = 0x3a3a3a;
    /// 尚未实现功能的按钮图标颜色(置灰,提示不可用)。
    pub const ICON_DISABLED: u32 = 0x606060;
    /// 工具栏按钮的提示文字颜色。
    pub const LABEL: u32 = 0xc8c8c8;
}

/// 弹出菜单配色。菜单是浮在窗口内容之上的层,底色比工具栏更亮,
/// 与窗口 chrome 区分开,让用户明确"这是浮层不是窗格"。
pub mod menu {
    /// 菜单面板底色。
    pub const BG: u32 = 0x2b2b2b;
    /// 菜单项文字颜色。
    pub const TEXT: u32 = 0xdcdcdc;
    /// 菜单项不可用时的文字颜色(置灰)。
    pub const TEXT_DISABLED: u32 = 0x707070;
    /// 菜单项悬停时的底色。
    pub const HOVER_BG: u32 = 0x3d3d3d;
    /// 菜单项图标颜色。
    pub const ICON: u32 = 0xc0c0c0;
    /// 菜单项图标不可用时的颜色。
    pub const ICON_DISABLED: u32 = 0x6a6a6a;
    /// 分隔线颜色。
    pub const SEPARATOR: u32 = 0x454545;
    /// 快捷键文字颜色(比正文暗一档,右对齐显示)。
    pub const SHORTCUT: u32 = 0x9a9a9a;
    /// 勾选框边框颜色。
    pub const CHECKBOX_BORDER: u32 = 0x8a8a8a;
    /// 勾选框内勾号颜色。
    pub const CHECKMARK: u32 = 0xdcdcdc;
}

/// 弹出菜单配色。菜单是浮在窗口内容之上的层,底色比工具栏更亮,
/// 与窗口 chrome 区分开,让用户明确"这是浮层不是窗格"。
/// 菜单面板宽度(px)。与 Konsole 的菜单宽度接近,足够放下中文文字与快捷键。
pub const MENU_WIDTH: f32 = 300.0;

/// 菜单项高度(px)。
pub const MENU_ITEM_HEIGHT: f32 = 26.0;

/// 菜单面板的内边距(px)。
pub const MENU_PADDING: f32 = 4.0;

/// 菜单项左右内边距(px)。
pub const MENU_ITEM_PADDING: f32 = 10.0;

/// 菜单图标尺寸(px)。
pub const MENU_ICON_SIZE: f32 = 16.0;

/// 菜单项之间的图标与文字间距(px)。
pub const MENU_ITEM_GAP: f32 = 8.0;

/// 菜单分隔线的高度(px)。
pub const MENU_SEPARATOR_HEIGHT: f32 = 1.0;

/// 分隔线上下的留白(px)。
pub const MENU_SEPARATOR_MARGIN: f32 = 5.0;

/// 菜单弹出时与锚点按钮的垂直间距(px)。
pub const MENU_ANCHOR_OFFSET: f32 = 2.0;

/// 子菜单相对父菜单的横向偏移(px)。
pub const SUBMENU_OFFSET: f32 = 4.0;

/// 应用工具栏高度(px)。不计入终端网格行数,但会占用客户区高度(WIN-007)。
pub const TOOLBAR_HEIGHT: f32 = 34.0;

/// 工具栏按钮高度(px)。
pub const TOOLBAR_BUTTON_HEIGHT: f32 = 26.0;

/// 工具栏按钮图标尺寸(px)。
pub const TOOLBAR_ICON_SIZE: f32 = 16.0;

/// 工具栏按钮的横向内边距(px)。
pub const TOOLBAR_BUTTON_PADDING: f32 = 8.0;

/// 工具栏按钮之间的间隔(px)。
pub const TOOLBAR_BUTTON_GAP: f32 = 4.0;

/// 「新建标签」按钮的宽度(px)。
pub const NEW_TAB_BUTTON_WIDTH: f32 = 30.0;

/// 单个标签的最小宽度(px)。标签多到放不下时保持这个宽度并横向滚动。
pub const TAB_MIN_WIDTH: f32 = 90.0;

/// 单个标签的最大宽度(px)。标签少时不会把标题拉成一整行。
pub const TAB_MAX_WIDTH: f32 = 220.0;

/// 标签内关闭图标的点击区宽度(px)。
pub const TAB_CLOSE_WIDTH: f32 = 20.0;

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
