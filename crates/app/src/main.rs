//! RustSole 入口:启动 GPUI 应用,打开单窗口单终端(阶段 1)。
//!
//! 窗口使用客户端装饰(自绘标题栏):`titlebar.appears_transparent = true` 让 Windows/macOS
//! 隐藏系统标题栏,窗口内容铺满整个窗口,由 `term_ui::titlebar` 绘制标题与窗口按钮(WIN-001)。
//! Linux X11/Wayland 还要同时请求客户端装饰,否则合成器仍会画系统边框(WIN-002)。

use gpui::{
    px, size, App, Application, Bounds, Focusable as _, TitlebarOptions, WindowBounds,
    WindowDecorations, WindowOptions,
};

const INIT_COLS: u16 = 80;
const INIT_ROWS: u16 = 24;

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

        // 会话启动失败属于用户可感知的启动错误:打印后以非零码退出,不 panic
        // 视口在窗口创建前已知尺寸常量,先按初始值建会话;open_window 闭包内再用实际视口微调
        let view = match term_ui::view::TerminalView::new(cx, INIT_COLS, INIT_ROWS) {
            Ok(view) => view,
            Err(e) => {
                eprintln!("RustSole 启动失败: {e}");
                std::process::exit(1);
            }
        };

        let opened = cx.open_window(options, |window, cx| {
            // 键盘焦点移到终端视图
            view.read(cx).focus_handle(cx).focus(window);
            view.clone()
        });
        if let Err(e) = opened {
            eprintln!("打开窗口失败: {e}");
            std::process::exit(1);
        }
    });
}
