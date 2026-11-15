//! 标签页相关的 Action(ARCHITECTURE.md 不变量 5:所有用户操作走 Action)。
//!
//! 键位在应用入口用 `App::bind_keys` 注册,`key_context("TabsRoot")` 让它们只在
//! 标签容器内生效;Action 的处理函数在 `tabs.rs` 的根节点上。
//!
//! 为什么用 Action 而不是在 `on_key_down` 里判断字符串:菜单栏、右键菜单、命令面板
//! (后续阶段)都要触发同一批操作,重复写判断会让键位与行为两处漂移。

gpui::actions!(
    tabs,
    [
        /// 新建标签页并切换到它。
        NewTab,
        /// 关闭当前标签页(最后一个标签关闭时关闭窗口)。
        CloseActiveTab,
        /// 切换到下一个标签页,末尾环绕到第一个。
        NextTab,
        /// 切换到上一个标签页,首个环绕到最后一个。
        PrevTab,
    ]
);
