//! 终端视图:单窗口单终端(阶段 1)。
//! 输入采集与渲染在这里;快照提取在锁内、绘制在锁外(ARCHITECTURE.md 不变量 2/8)。

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, fill, font, px, rgb, size, App, Bounds, Context, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent,
    SharedString, UTF16Selection, Window,
};

use alacritty_terminal::grid::{Dimensions as _, Scroll};

use term_core::key::{encode_key, KeyState};
use term_core::proxy::TermEventMsg;
use term_core::session::Session;

use crate::grid::{ratio_to_offset, snapshot, thumb_ratio, GridSnapshot};

/// 单元格像素尺寸(阶段 1 固定值,随 HiDPI 缩放由 GPUI 处理)。
pub const CELL_WIDTH: f32 = 8.0;
pub const CELL_HEIGHT: f32 = 16.0;
pub const FONT_SIZE: f32 = 14.0;

/// 滚动条轨道宽度(px)。
pub const SCROLLBAR_WIDTH: f32 = 10.0;

/// thumb 高度占轨道高度的比例(阶段 1 固定值;未按内容比例动态计算)。
pub const THUMB_RATIO: f32 = 0.15;

/// 滚动条拖动状态。grab_y 是按下时相对滚动条轨道顶部的 y(px),
/// grab_offset 是按下时 thumb 对应的 display_offset,保证拖动不跳变。
#[derive(Debug, Clone, Copy)]
struct ScrollDrag {
    grab_y: f32,
    grab_offset: f32,
}

/// 终端视图状态。
pub struct TerminalView {
    session: Session,
    focus: FocusHandle,
    /// 当前生效的字符网格尺寸。
    cols: u16,
    rows: u16,
    /// 实测的单字符 advance(px)。首次绘制时测量,替换粗略的 CELL_WIDTH 常量,
    /// 避免网格列宽与字体实际宽度不一致造成的空格过宽/错位。
    cell_width: f32,
    /// IME 组合中(pinyin 等输入法的预编辑阶段)。组合中的按键不进 PTY,
    /// 由 IME 提交结果后经 replace_text_in_range 写入。
    ime_composing: bool,
    /// 滚动条拖动状态(按下拖动期间);None 表示未拖动。
    scroll_drag: Option<ScrollDrag>,
    /// 最近一次视口高度(px),滚动条几何需要。
    viewport_height: f32,
    /// 滚轮不足一行的余量(f32 行)。触控板的精确增量需要累积,否则小数部分被截断会无响应。
    scroll_remainder: f32,
    /// 滚动条渲染状态(thumb 回看比例),事件与渲染共享。
    scrollbar_state: Arc<std::sync::Mutex<ScrollbarState>>,
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
            cell_width: CELL_WIDTH,
            ime_composing: false,
            scroll_drag: None,
            viewport_height: 0.0,
            scroll_remainder: 0.0,
            scrollbar_state: Arc::new(std::sync::Mutex::new(ScrollbarState {
                offset_ratio: 0.0,
            })),
            snapshot: Arc::new(GridSnapshot {
                rows: rows as usize,
                cols: cols as usize,
                rows_runs: Vec::new(),
                cursor: None,
                display_offset: 0,
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


    /// 处理滚轮:向上滚 → 回看历史(display_offset 增大)。
    ///
    /// 符号依据(锁定版本源码):
    /// - gpui Windows 把 WM_MOUSEWHEEL 的 zDelta 原样换算(events.rs:548-549),Windows 约定上滚为正;
    /// - alacritty 的 Scroll::Delta(正数)使 display_offset 增大,即回看更早内容(grid/mod.rs:166)。
    ///
    /// 两者同号,因此直接把 delta 传入,不再取反。
    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let lines = match event.delta {
            // 精确像素增量(触控板):按单元格高度折算为行,余量累积避免小数丢失
            ScrollDelta::Pixels(delta) => {
                let y_px: f32 = delta.y.into();
                y_px / CELL_HEIGHT
            },
            // 行增量(传统滚轮):直接使用
            ScrollDelta::Lines(delta) => delta.y,
        };

        self.scroll_remainder += lines;
        // 只取整行进位,余量留到下次(向零取整,保证方向单调)
        let whole = self.scroll_remainder.trunc();
        if whole == 0.0 {
            return;
        }
        self.scroll_remainder -= whole;
        self.scroll_lines(whole as i32, cx);
    }

    /// 滚动条按下:命中 thumb 区域时开始拖动,否则跳转到点击位置。
    fn scrollbar_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        // 鼠标事件 position 是视口坐标;滚动条轨道从视口顶部开始,y 直接换算
        self.scrollbar_jump_or_drag(event.position.y.into(), cx);
    }

    /// 滚动条拖动:y 变化按 thumb 跨度换算 display_offset。
    fn scrollbar_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.scroll_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            // 未收到 mouse_up(极端情况):结束拖动
            self.scroll_drag = None;
            return;
        }
        self.apply_drag(drag, event.position.y.into(), cx);
    }

    /// 按下时的两种行为:点在 thumb 上则原地拖动,否则跳转到该位置。
    fn scrollbar_jump_or_drag(&mut self, y_px: f32, cx: &mut Context<Self>) {
        let history = { self.session.term.lock().grid().history_size() };
        if history == 0 {
            return;
        }
        let (track_height, thumb_top, thumb_height) = self.scrollbar_geometry();
        let relative = y_px.max(0.0).min(track_height);
        let in_thumb = thumb_top <= relative && relative < thumb_top + thumb_height;
        if in_thumb {
            // 抓到 thumb:记录起点,不跳转,避免手抖跳变
            self.scroll_drag = Some(ScrollDrag {
                grab_y: y_px,
                grab_offset: self.display_offset_value(),
            });
        } else {
            // 点在轨道空白:按轨道位置跳转(轨道底 = 最新内容)
            let ratio = if track_height > 0.0 {
                relative / track_height
            } else {
                1.0
            };
            let target = ratio_to_offset(ratio, history);
            self.set_display_offset(target as i32, cx);
        }
    }

    /// 拖动执行:按 y 偏移换算目标 offset。
    fn apply_drag(&mut self, drag: ScrollDrag, y_px: f32, cx: &mut Context<Self>) {
        let history = { self.session.term.lock().grid().history_size() };
        if history == 0 {
            return;
        }
        let (track_height, _thumb_top, thumb_height) = self.scrollbar_geometry();
        let dy = y_px - drag.grab_y;
        // thumb 在轨道上的可移动跨度
        let thumb_span = (track_height - thumb_height).max(1.0);
        // 鼠标向下移动(dy>0)使 thumb 下移 → ratio 增大 → offset 减小(趋向最新内容),
        // 因此 offset 变化量与 dy 反号。
        let delta_offset = -(dy / thumb_span) * history as f32;
        let target = (drag.grab_offset + delta_offset)
            .max(0.0)
            .min(history as f32);
        self.set_display_offset(target.floor() as i32, cx);
    }

    /// 当前 display_offset(锁内直接读)。
    fn display_offset_value(&self) -> f32 {
        let guard = self.session.term.lock();
        guard.grid().display_offset() as f32
    }

    /// 滚动条几何:轨道高度、thumb 顶部偏移、thumb 高度(全部相对轨道顶部)。
    fn scrollbar_geometry(&self) -> (f32, f32, f32) {
        let track_height = self.viewport_height.max(0.0);
        let thumb_height = track_height * THUMB_RATIO;
        let (history, offset) = {
            let term = self.session.term.lock();
            (term.grid().history_size(), term.grid().display_offset())
        };
        let ratio = thumb_ratio(offset, history);
        let thumb_top = ratio * (track_height - thumb_height).max(0.0);
        (track_height, thumb_top, thumb_height)
    }

    /// 实测等宽字体的单字符 advance。首次调用后缓存,字体不变则不再 shape。
    /// 不用硬编码 CELL_WIDTH 的原因:不同平台字体(Consolas/monospace)与字号的实际 advance
    /// 与 8px 有偏差,网格列按 8px 排布会让背景块与文本错位(表现为空格过宽)。
    fn measure_cell_width(&mut self, window: &mut Window) {
        // 只测一次:cell_width 与 CELL_WIDTH 相等说明尚未测量
        if self.cell_width != CELL_WIDTH {
            return;
        }
        let text_run = gpui::TextRun {
            len: 1,
            font: font(term_platform::platform_info().default_font_family),
            color: rgb(0xffffff).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from("M"),
            px(FONT_SIZE),
            &[text_run],
            None,
        );
        // ShapedLine Deref 到 LineLayout,width 为 pub
        let w: f32 = shaped.width.into();
        if w > 0.0 {
            self.cell_width = w;
            // 列宽变了,重新按新宽度计算网格尺寸并同步 PTY
            let width: f32 = window.viewport_size().width.into();
            self.cols = ((width / w).floor() as u16).max(2);
            self.session.resize(self.cols, self.rows);
            {
                let mut term = self.session.term.lock();
                term.resize(alacritty_terminal::term::test::TermSize {
                    columns: self.cols as usize,
                    screen_lines: self.rows as usize,
                });
            }
        }
    }

    /// 滚动 display_offset delta 行。Delta 正值 = 回看更多历史(MOUSE-003 主屏回看行为)。
    /// 在 Term 锁内修改,锁外刷新快照并 notify。
    fn scroll_lines(&mut self, delta: i32, cx: &mut Context<Self>) {
        let history = { self.session.term.lock().grid().history_size() };
        if history == 0 {
            // 没有历史,滚动无意义
            return;
        }
        { let mut term = self.session.term.lock(); term.scroll_display(Scroll::Delta(delta)); }
        self.refresh_scrollbar_state();
        self.refresh_snapshot();
        cx.notify();
    }

    /// 把 display_offset 直接设置到目标值(点击/拖动滚动条用),带钳制。锁内一次完成。
    fn set_display_offset(&mut self, target: i32, cx: &mut Context<Self>) {
        { let mut term = self.session.term.lock();
            let history = term.grid().history_size();
            if history == 0 {
                return;
            }
            let current = term.grid().display_offset() as i32;
            let clamped = target.max(0).min(history as i32);
            term.scroll_display(Scroll::Delta(clamped - current));
        }
        self.refresh_scrollbar_state();
        self.refresh_snapshot();
        cx.notify();
    }

    /// 刷新滚动条 thumb 回看比例(display_offset / history)。
    fn refresh_scrollbar_state(&self) {
        let (history, offset) = {
            let term = self.session.term.lock();
            (term.grid().history_size(), term.grid().display_offset())
        };
        let ratio = thumb_ratio(offset, history);
        if let Ok(mut state) = self.scrollbar_state.lock() {
            state.offset_ratio = ratio;
        }
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
        // 扣掉滚动条轨道宽度(列数按终端画布实际可用宽度算)
        // alacritty 限制:MIN_COLUMNS=2,可见行至少 1
        let new_cols = (((width - SCROLLBAR_WIDTH) / self.cell_width).floor() as u16).max(2);
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
    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // IME 组合中的按键已被输入法拦截(VK_PROCESSKEY),gpui 仍会发原始键的 KeyDown,
        // 这里必须丢弃,否则拼音字母与提交后的汉字会双写进 PTY。
        if self.ime_composing {
            return;
        }
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
            keystroke.key_char.as_deref(),
            m.control,
            m.alt,
            m.shift,
            KeyState {
                app_cursor,
                // gpui Windows 的 key_char 不反映 CapsLock,用窗口缓存的状态补偿
                caps_lock: window.capslock().on,
            },
        ) {
            self.session.write(&bytes);
            // 字符已由键盘编码路径处理:阻止传播,否则 gpui 对未处理事件调用 TranslateMessage
            // 生成 WM_CHAR,handle_char_msg 会经 replace_text_in_range 把同一字符再写一次(双字母)
            cx.stop_propagation();
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
        self.measure_cell_width(window);

        let snapshot = Arc::clone(&self.snapshot);
        let focus = self.focus.clone();
        let cell_width = self.cell_width;
        let this = cx.entity();

        div()
            .id("terminal-root")
            .size_full()
            .bg(rgb(0x101010))
            .track_focus(&focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                view.handle_key(event, window, cx);
            }))
            // 终端画布区:滚轮回看历史
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, cx| {
                view.handle_scroll_wheel(event, cx);
            }))
            .child(
                div()
                    .size_full()
                    .flex()
                    .child(
                        canvas(
                            |_bounds, _window, _cx| {},
                            move |bounds, (), window, cx| {
                                // 注册 IME 输入处理器(paint 阶段必须;焦点在终端时平台才会投递组合文本)
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, this),
                                    cx,
                                );
                                paint_grid(&snapshot, cell_width, bounds, window, cx);
                            },
                        )
                        .size_full(),
                    )
                    .child(scrollbar_element(
                        Arc::clone(&self.scrollbar_state),
                        cx.listener(|view, event: &MouseDownEvent, _window, cx| {
                            view.scrollbar_mouse_down(event, cx);
                        }),
                        cx.listener(|view, event: &MouseMoveEvent, _window, cx| {
                            view.scrollbar_mouse_move(event, cx);
                        }),
                    )),
            )
    }
}

/// 滚动条状态(跨事件与渲染共享):thumb 的回看比例 0..1。
#[derive(Debug)]
struct ScrollbarState {
    offset_ratio: f32,
}

/// 滚动条元素:div 容器包 canvas(canvas 不实现 InteractiveElement,事件注册在 div 上)。
fn scrollbar_element(
    state: Arc<std::sync::Mutex<ScrollbarState>>,
    mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    mouse_move: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    div()
        .w(px(SCROLLBAR_WIDTH))
        .bg(rgb(0x181818))
        .child(
            canvas(
                // prepaint:thumb 位置由状态决定
                move |_bounds, _window, _cx| (),
                // paint:从共享状态读取 offset_ratio 绘制 thumb
                move |bounds, (), window, _cx| {
                    let ratio = state.lock().map(|s| s.offset_ratio).unwrap_or(0.0);
                    paint_scrollbar(bounds, ratio, window);
                },
            )
            .size_full(),
        )
        .on_mouse_down(MouseButton::Left, mouse_down)
        .on_mouse_move(mouse_move)
        .into_any_element()
}

/// 绘制滚动条轨道与 thumb。thumb 高度 = 轨道高 ×(可见行/全部内容行),
/// 位置 = ratio ×(轨道高 - thumb 高)。
fn paint_scrollbar(bounds: Bounds<Pixels>, thumb_ratio_value: f32, window: &mut Window) {
    let track_h: f32 = bounds.size.height.into();
    let thumb_h = track_h * THUMB_RATIO;
    // thumb_ratio_value:1.0 = 轨道底(最新内容),0.0 = 轨道顶(最旧内容)
    let thumb_top = thumb_ratio_value * (track_h - thumb_h).max(0.0);
    let thumb_bounds = Bounds::new(
        Point::new(bounds.origin.x, bounds.origin.y + px(thumb_top)),
        size(px(SCROLLBAR_WIDTH), px(thumb_h)),
    );
    window.paint_quad(fill(thumb_bounds, rgb(0x555555)));
}

/// 绘制网格:先背景块,再文本,最后光标。所有数据来自快照,无锁。
/// cell_width 为实测单字符 advance,保证背景块列宽与文本 advance 一致。
fn paint_grid(
    snapshot: &GridSnapshot,
    cell_width: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, rgb(0x101010)));

    let font_family = term_platform::platform_info().default_font_family;

    for (row_idx, runs) in snapshot.rows_runs.iter().enumerate() {
        let y = bounds.origin.y + px(row_idx as f32 * CELL_HEIGHT);
        for run in runs {
            let x = bounds.origin.x + px(run.start_col as f32 * cell_width);
            let width = px(run.text.chars().count() as f32 * cell_width);

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
                bounds.origin.x + px(col as f32 * cell_width),
                bounds.origin.y + px(row as f32 * CELL_HEIGHT),
            ),
            size(px(cell_width), px(CELL_HEIGHT)),
        );
        window.paint_quad(fill(cursor_bounds, gpui::hsla(0.0, 0.0, 0.9, 0.35)));
    }
}

/// IME 集成:组合中的拼音不显示(候选窗由系统绘制,位置跟随终端光标),
/// 提交的汉字串直接写 PTY。对齐 Windows Terminal 的行为。
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // 终端无文档模型
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // 空选区:表示插入点存在,IME 候选窗经 bounds_for_range 跟随光标
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        // 组合中返回非空 range:gpui 据此在组合期间吞掉原始按键的 KeyDown
        // (events.rs handle_keydown_msg 的 is_composing 分支),防止拼音字母进入 PTY。
        // 终端不显示预编辑,这里只做状态声明。
        if self.ime_composing {
            Some(0..1)
        } else {
            None
        }
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.ime_composing = false;
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_composing = false;
        // IME 提交的文本是用户意图输入的内容,不走粘贴过滤(PASTE 规则只约束粘贴通道)
        self.session.write(text.as_bytes());
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // 组合中:仅记录状态,不写入 PTY,也不在终端显示预编辑
        self.ime_composing = !text.is_empty();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 候选窗锚点:终端光标所在单元格(行 y 向下正,平台层会自行换算物理坐标)
        let (row, col) = self.snapshot.cursor.unwrap_or((0, 0));
        Some(Bounds::new(
            Point::new(
                element_bounds.origin.x + px(col as f32 * self.cell_width),
                element_bounds.origin.y + px(row as f32 * CELL_HEIGHT),
            ),
            size(px(self.cell_width), px(CELL_HEIGHT)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}