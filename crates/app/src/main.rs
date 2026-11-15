//! RustSole 入口:启动 GPUI 应用,打开一个带标签页的窗口(阶段 3)。
//!
//! 窗口使用客户端装饰(自绘标题栏):`titlebar.appears_transparent = true` 让 Windows
//! 隐藏系统标题栏,窗口内容铺满整个窗口,由 `term_ui::tabs` 绘制标签条与窗口按钮(WIN-001)。
//! Linux X11/Wayland 还要同时请求客户端装饰,否则合成器仍会画系统边框(WIN-002)。

use gpui::{
    px, size, App, Application, Bounds, Focusable as _, KeyBinding, TitlebarOptions, WindowBounds,
    WindowDecorations, WindowOptions,
};

use term_ui::actions::{CloseActiveTab, CopySelection, NewTab, NextTab, PasteClipboard, PrevTab};
use term_ui::tabs::TabsView;

const INIT_COLS: u16 = 80;
const INIT_ROWS: u16 = 24;

/// 注册标签页与复制/粘贴快捷键(KEY-001、KEY-002、KEY-006..KEY-009)。
///
/// 为什么用 `Ctrl+Shift` 而不是裸 `Ctrl+T` / `Ctrl+W`:
/// gpui 的绑定在 `on_key_down` **之前**匹配(见 gpui 0.2.2 `window.rs:3790` 的
/// `dispatch_key` 位于 `finish_dispatch_key_event` 之前),绑定命中后事件不再下发给
/// 终端。裸 `Ctrl+T`(0x14)、`Ctrl+W`(0x17)是 shell 与 emacs 的常用控制字符,
/// 抢走它们会违反 KEY-004 的同类原则(应用快捷键不得占用终端控制字符)。
/// `Ctrl+Shift` 组合在 `view.rs::handle_key` 中本就保留给应用快捷键,不写入 PTY。
fn bind_tab_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, Some("TabsRoot")),
        KeyBinding::new("ctrl-shift-w", CloseActiveTab, Some("TabsRoot")),
        KeyBinding::new("ctrl-tab", NextTab, Some("TabsRoot")),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some("TabsRoot")),
        // KEY-001/KEY-002:复制与剪贴板粘贴。与工具栏按钮走同一批 Action。
        KeyBinding::new("ctrl-shift-c", CopySelection, Some("TabsRoot")),
        KeyBinding::new("ctrl-shift-v", PasteClipboard, Some("TabsRoot")),
    ]);
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.0), px(600.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("RustSole".into()),
                // 隐藏系统标题栏,内容区延伸到整个窗口
                appears_transparent: true,
                ..Default::default()
            }),
            // Linux:请求客户端装饰(Windows/macOS 忽略此字段,见 gpui platform.rs:1129-1131)
            window_decorations: Some(WindowDecorations::Client),
            ..Default::default()
        };

        bind_tab_keys(cx);

        // 会话启动失败属于用户可感知的启动错误:打印后以非零码退出,不 panic
        // 视口在窗口创建前已知尺寸常量,先按初始值建会话;open_window 闭包内再用实际视口微调
        let view = match TabsView::new(cx, INIT_COLS, INIT_ROWS) {
            Ok(view) => view,
            Err(e) => {
                eprintln!("RustSole 启动失败: {e}");
                std::process::exit(1);
            }
        };

        let opened = cx.open_window(options, |window, cx| {
            // 键盘焦点移到当前标签的终端
            view.read(cx).focus_handle(cx).focus(window);
            view.clone()
        });
        if let Err(e) = opened {
            eprintln!("打开窗口失败: {e}");
            std::process::exit(1);
        }
    });
}
