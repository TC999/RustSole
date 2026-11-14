//! 选区缓冲的进程级共享实例(SEL-007)。
//!
//! ARCHITECTURE.md §6 明确允许用 GPUI `Global` 保存选择缓冲区:同一进程内的所有窗口与
//! 标签必须共享同一份选区。缓冲区本身是不可变的"最近一次选中",放在全局不违反
//! "不用全局可变静态变量保存终端状态"的约束。

use std::sync::Arc;

use gpui::{App, Global};
use term_core::selection::SelectionBuffer;

/// 全局句柄:只持有 `Arc`,克隆给各个视图使用。
pub struct SelectionBufferGlobal(Arc<SelectionBuffer>);

impl Global for SelectionBufferGlobal {}

/// 初始化全局选区缓冲。应用启动时调用一次;重复调用是安全的(已存在则不动)。
pub fn init(cx: &mut App) {
    if cx.try_global::<SelectionBufferGlobal>().is_none() {
        cx.set_global(SelectionBufferGlobal(Arc::new(SelectionBuffer::new())));
    }
}

/// 取全局缓冲区。未初始化时返回 `None`,调用方回退到视图私有的实例。
pub fn shared(cx: &App) -> Option<Arc<SelectionBuffer>> {
    cx.try_global::<SelectionBufferGlobal>()
        .map(|global| Arc::clone(&global.0))
}
