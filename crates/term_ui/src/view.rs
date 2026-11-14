//! 终端视图:单窗口单终端(阶段 1)。
//! 输入采集与渲染在这里;快照提取在锁内、绘制在锁外(ARCHITECTURE.md 不变量 2/8)。

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, fill, font, px, rgb, size, App, Bounds, Context, Entity, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, Pixels, Point, Render, SharedString, Window,
};

use term_core::key::{encode_key, KeyState};
use term_core::proxy::TermEventMsg;
use term_core::session::Session;

use crate::grid::{snapshot, GridSnapshot};

/// 单元格像素尺寸(阶段 1 固定值,随 HiDPI 缩放由 GPUI 处理)。
pub const CELL_WIDTH: f32 = 8.0;
pub const CELL_HEIGHT: f32 = 16.0;
pub const FONT_SIZE: f32 = 14.0;

/// 终端视图状态。
pub struct TerminalView {
    session: Session,
    focus: FocusHandle,
    /// 当前生效的字符网格尺寸。
    cols: u16,
    rows: u16,
    /// 待渲染快照(Wakeup 时刷新)。
    snapshot: Arc<GridSnapshot>,
}

impl TerminalView {
    /// 创建视图并启动 PTY 会话。
    ///
    /// cols/rows 必须与创建时的视口一致:Term 与 PTY 从一开始就同尺寸,
    /// 避免启动窗口期 EventLoop 向小网格写入大量输出导致越界。
    ///
    /// # Errors
    /// PTY/shell 启动失败时返回,由入口显示给用户(CODING_STANDARDS.md 第 3 节)。
    pub fn new(cx: &mut App, cols: u16, rows: u16) -> gpui::Result<Entity<Self>> {
        let session = Session::new(cols, rows)
            .map_err(|e| gpui::private::anyhow::anyhow!("启动终端会话失败: {e}"))?;

        let focus = cx.focus_handle();
        let view = cx.new(|_| Self {
            session,
            focus,
            cols,
            rows,
            snapshot: Arc::new(GridSnapshot {
                rows: rows as usize,
                cols: cols as usize,
                rows_runs: Vec::new(),
                cursor: None,
            }),
        });

        // 事件驱动重绘:后台任务把 PTY 事件合并后唤醒主线程(不变量 4:约 8ms 合并)
        view.update(cx, |view, cx| view.spawn_event_loop(cx));
        Ok(view)
    }

    /// 常驻任务:等待 PTY 事件 → 批量合并 → notify 主线程重绘(不变量 4:合并唤醒)。
    /// 不持 Term 锁、不在锁内做长工作(不变量 2)。
    fn spawn_event_loop(&mut self, cx: &mut Context<Self>) {
        let events = self.session.events_handle();
        cx.spawn(async move |weak, cx| {
            loop {
                // std 通道式阻塞在 async 里不可用;用轻量轮询等待事件到来(空转仅原子读,4ms 粒度)
                while !events.has_pending() {
                    gpui::Timer::after(std::time::Duration::from_millis(4)).await;
                }

                // 排空当前积压,合并为一次主线程更新
                let taken = events.drain();
                let mut had_wakeup = false;
                let mut pty_writes: Vec<String> = Vec::new();
                for msg in taken {
                    match msg {
                        TermEventMsg::Wakeup | TermEventMsg::Exited => had_wakeup = true,
                        TermEventMsg::PtyWrite(s) => pty_writes.push(s),
                        TermEventMsg::Title(_) | TermEventMsg::Other => {},
                    }
                }

                let ok = weak
                    .update(cx, |view, cx| {
                        // 程序应答序列写回 PTY
                        for s in &pty_writes {
                            view.session.write(s.as_bytes());
                        }
                        if had_wakeup {
                            view.refresh_snapshot();
                            cx.notify();
                        }
                    })
                    .is_ok();
                // 视图已释放:会话结束,退出任务
                if !ok {
                    return;
                }
            }
        })
        .detach();
    }

    /// 不变量 2:锁内只取快照,立即释放。
    fn refresh_snapshot(&mut self) {
        let guard = self.session.term.lock();
        let snap = snapshot(&guard);
        drop(guard);
        self.snapshot = Arc::new(snap);
    }

    /// 视口尺寸变化时,把字符网格调整为新的 cols/rows,并通知 PTY 与 Term。
    fn sync_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vp = window.viewport_size();
        let width: f32 = vp.width.into();
        let height: f32 = vp.height.into();
        // alacritty 限制:MIN_COLUMNS=2,可见行至少 1
        let new_cols = ((width / CELL_WIDTH).floor() as u16).max(2);
        let new_rows = ((height / CELL_HEIGHT).floor() as u16).max(1);
        if new_cols != self.cols || new_rows != self.rows {
            self.cols = new_cols;
            self.rows = new_rows;
            self.session.resize(new_cols, new_rows);
            // Term 网格同步 resize(EventLoop 的 Msg::Resize 只改 PTY)
            {
                let mut term = self.session.term.lock();
                term.resize(alacritty_terminal::term::test::TermSize {
                    columns: new_cols as usize,
                    screen_lines: new_rows as usize,
                });
            }
            self.refresh_snapshot();
            cx.notify();
        }
    }

    /// 处理键盘输入:编码并发送(KEY-004/KEY-005)。
    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;

        // Ctrl+Shift 组合保留给应用快捷键(Ctrl+Shift+C/V 等),不进编码器
        if m.control && m.shift {
            return;
        }

        let app_cursor = {
            let term = self.session.term.lock();
            term.mode().contains(term_core::mode_flag_app_cursor())
        };

        if let Some(bytes) = encode_key(
            &keystroke.key,
            m.control,
            m.alt,
            m.shift,
            KeyState { app_cursor },
        ) {
            self.session.write(&bytes);
            cx.notify();
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // PTY 事件由 spawn_event_loop 的常驻任务驱动重绘;render 只处理视口尺寸变化
        self.sync_size(window, cx);

        let snapshot = Arc::clone(&self.snapshot);
        let focus = self.focus.clone();

        div()
            .id("terminal-root")
            .size_full()
            .bg(rgb(0x101010))
            .track_focus(&focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                view.handle_key(event, cx);
            }))
            .child(
                canvas(
                    // prepaint:此元素不需要额外状态
                    |_bounds, _window, _cx| {},
                    // paint:逐行绘制文本 run(相邻同样式单元格已合并)
                    move |bounds, (), window, cx| {
                        paint_grid(&snapshot, bounds, window, cx);
                    },
                )
                .size_full(),
            )
    }
}

/// 绘制网格:先背景块,再文本,最后光标。所有数据来自快照,无锁。
fn paint_grid(snapshot: &GridSnapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    window.paint_quad(fill(bounds, rgb(0x101010)));

    let font_family = term_platform::platform_info().default_font_family;

    for (row_idx, runs) in snapshot.rows_runs.iter().enumerate() {
        let y = bounds.origin.y + px(row_idx as f32 * CELL_HEIGHT);
        for run in runs {
            let x = bounds.origin.x + px(run.start_col as f32 * CELL_WIDTH);
            let width = px(run.text.chars().count() as f32 * CELL_WIDTH);

            // 背景色块(仅当非默认背景时画)
            if run.style.bg != crate::grid::DEFAULT_BG {
                let bg_bounds = Bounds::new(Point::new(x, y), size(width, px(CELL_HEIGHT)));
                window.paint_quad(fill(
                    bg_bounds,
                    rgb((run.style.bg[0] as u32) << 16
                        | (run.style.bg[1] as u32) << 8
                        | run.style.bg[2] as u32),
                ));
            }

            // 文本 run(TextRun.font 是 gpui::Font,颜色需要 Hsla)
            let text_run = gpui::TextRun {
                len: run.text.len(),
                font: font(font_family),
                color: rgb((run.style.fg[0] as u32) << 16
                    | (run.style.fg[1] as u32) << 8
                    | run.style.fg[2] as u32)
                .into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(run.text.clone()),
                px(FONT_SIZE),
                &[text_run],
                None,
            );
            let _ = shaped.paint(Point::new(x, y), px(CELL_HEIGHT), window, cx);
        }
    }

    // 光标:半透明方块叠加
    if let Some((row, col)) = snapshot.cursor {
        let cursor_bounds = Bounds::new(
            Point::new(
                bounds.origin.x + px(col as f32 * CELL_WIDTH),
                bounds.origin.y + px(row as f32 * CELL_HEIGHT),
            ),
            size(px(CELL_WIDTH), px(CELL_HEIGHT)),
        );
        window.paint_quad(fill(cursor_bounds, gpui::hsla(0.0, 0.0, 0.9, 0.35)));
    }
}