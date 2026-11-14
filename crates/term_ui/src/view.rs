//! 终端视图:单窗口单终端(阶段 1)。
//! 输入采集与渲染在这里;快照提取在锁内、绘制在锁外(ARCHITECTURE.md 不变量 2/8)。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, fill, font, px, relative, rgb, size, App, Bounds, ClickEvent, Context,
    DispatchPhase, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, UTF16Selection, Window,
};

use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;

use term_core::key::{encode_key, KeyState};
use term_core::paste::sanitize_paste;
use term_core::proxy::TermEventMsg;
use term_core::selection::SelectionBuffer;
use term_core::session::Session;

use crate::buffer;
use crate::grid::{ratio_to_offset, snapshot, thumb_ratio, GridSnapshot};
use crate::selection::{terminal_handles_mouse, DragAnchor, GridGeometry};
use crate::theme::{terminal, window as window_theme, WindowControl, WindowControlsShown};
use crate::titlebar::{render_titlebar, TitlebarHandlers, TitlebarModel};

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
    /// 滚动条 thumb 的回看比例(1.0 = 贴轨道底/最新内容,0.0 = 贴轨道顶/最旧内容)。
    scrollbar_ratio: f32,
    /// 待渲染快照(Wakeup 时刷新)。
    snapshot: Arc<GridSnapshot>,
    /// 窗口标题(标题栏文字;程序通过 OSC 0/2 设置标题时更新)。
    title: SharedString,
    /// 标题栏上当前悬停的控制按钮(None = 不在任何一个按钮上)。
    hovered_control: Option<WindowControl>,
    /// 窗口激活状态观察者(首次 render 时注册,见 render 中的说明)。
    activation_subscription: Option<gpui::Subscription>,
    /// 选区缓冲。首次 render 时从全局取共享实例(SEL-007),之后复用。
    selection_buffer: Option<Arc<SelectionBuffer>>,
    /// 左键拖选的锚点;None 表示当前没有拖选进行中。
    drag_anchor: Option<DragAnchor>,
    /// 终端画布在视口中的左上角(逻辑像素),每次绘制时由 canvas 回填。
    /// 鼠标事件给的是视口坐标,换算单元格必须减去这个偏移。
    /// 用 `Cell` 而不是字段直接赋值:paint 闭包只拿到共享引用,不能改视图字段。
    canvas_origin: Rc<Cell<(f32, f32)>>,
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

        // SEL-007:选区缓冲全局共享。保证全局实例存在(应用入口也会调用,这里是兜底)。
        buffer::init(cx);

        let focus = cx.focus_handle();
        let view = cx.new(|cx| Self {
            session,
            focus,
            cols,
            rows,
            cell_width: CELL_WIDTH,
            ime_composing: false,
            scroll_drag: None,
            viewport_height: 0.0,
            scroll_remainder: 0.0,
            scrollbar_ratio: 1.0,
            snapshot: Arc::new(GridSnapshot {
                rows: rows as usize,
                cols: cols as usize,
                rows_runs: Vec::new(),
                cursor: None,
                display_offset: 0,
                selection: Vec::new(),
            }),
            title: SharedString::from("RustSole"),
            hovered_control: None,
            activation_subscription: None,
            selection_buffer: buffer::shared(cx),
            drag_anchor: None,
            canvas_origin: Rc::new(Cell::new((0.0, 0.0))),
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
                let mut new_title: Option<String> = None;
                for msg in taken {
                    match msg {
                        TermEventMsg::Wakeup | TermEventMsg::Exited => had_wakeup = true,
                        TermEventMsg::PtyWrite(s) => pty_writes.push(s),
                        // OSC 0/2 设置的窗口标题:只保留最后一个,由主线程应用到标题栏
                        TermEventMsg::Title(t) => new_title = Some(t),
                        TermEventMsg::Other => {}
                    }
                }

                let ok = weak
                    .update(cx, |view, cx| {
                        // 程序应答序列写回 PTY
                        for s in &pty_writes {
                            view.session.write(s.as_bytes());
                        }
                        if let Some(t) = new_title {
                            // 标题是用户可见状态:只在真的变化时刷新,避免每帧重绘标题栏
                            let title = SharedString::from(t);
                            if view.title != title {
                                view.title = title;
                                had_wakeup = true;
                            }
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
            }
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
            // 列宽变了,重新按新宽度计算网格尺寸并同步 PTY。
            // 算法必须与 sync_size 保持一致(同样扣掉滚动条轨道宽度),否则首次测量后
            // 会先按整幅宽度算出偏多的列数,下一帧再被 sync_size 改回来,窗口一开就抖动。
            let width: f32 = window.viewport_size().width.into();
            self.cols = (((width - SCROLLBAR_WIDTH) / w).floor() as u16).max(2);
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
        {
            let mut term = self.session.term.lock();
            term.scroll_display(Scroll::Delta(delta));
        }
        self.refresh_scrollbar_state();
        self.refresh_snapshot();
        cx.notify();
    }

    /// 把 display_offset 直接设置到目标值(点击/拖动滚动条用),带钳制。锁内一次完成。
    fn set_display_offset(&mut self, target: i32, cx: &mut Context<Self>) {
        {
            let mut term = self.session.term.lock();
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
    fn refresh_scrollbar_state(&mut self) {
        let (history, offset) = {
            let term = self.session.term.lock();
            (term.grid().history_size(), term.grid().display_offset())
        };
        self.scrollbar_ratio = thumb_ratio(offset, history);
    }

    /// 本帧实际可用的客户区尺寸(逻辑像素)。
    ///
    /// 优先用 Win32 读到的真实客户区:窗口"最大化 -> 最小化 -> 还原为普通尺寸"后,
    /// gpui 的 `viewport_size()` 可能仍停留在最大化时的大小,而按它布局的自绘元素
    /// (标题栏按钮、滚动条)会被画到窗口外。两者相差超过 1px 时以系统值为准
    /// (1px 以内视为取整误差,避免每帧来回切换导致布局抖动)。
    fn layout_size(window: &Window) -> (f32, f32) {
        let vp = window.viewport_size();
        let viewport = (f32::from(vp.width), f32::from(vp.height));
        match term_platform::client_size_logical(window.scale_factor()) {
            Some((w, h)) if w > 1.0 && h > 1.0 => {
                let differs = (w - viewport.0).abs() > 1.0 || (h - viewport.1).abs() > 1.0;
                if differs {
                    (w, h)
                } else {
                    viewport
                }
            }
            _ => viewport,
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
    ///
    /// `size` 是本帧实际可用的客户区尺寸(逻辑像素),由 [`Self::layout_size`] 校正后传入,
    /// 不一定等于 `window.viewport_size()`。
    fn sync_size(&mut self, size: (f32, f32), cx: &mut Context<Self>) {
        let (width, height) = size;
        // 视口高度里扣掉自绘标题栏:终端网格只占标题栏以下的区域(WIN-001)。
        // 若不扣,最后一行会被标题栏遮住,且 `stty size` 比可见行数多 2 行左右。
        let height: f32 = (height - crate::theme::TITLEBAR_HEIGHT).max(0.0);
        // 扣掉滚动条轨道宽度(列数按终端画布实际可用宽度算)
        // alacritty 限制:MIN_COLUMNS=2,可见行至少 1
        let new_cols = (((width - SCROLLBAR_WIDTH) / self.cell_width).floor() as u16).max(2);
        let new_rows = ((height / CELL_HEIGHT).floor() as u16).max(1);
        // 滚动条几何(点击跳转/拖动)要用终端内容区的实际高度;这里就是它的来源。
        // 此前该字段只初始化、从未赋值,导致轨道高度恒为 0,滚动条点击位置判断失效。
        self.viewport_height = height;
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

    /// 选区缓冲(全局共享,SEL-007)。全局实例缺失时退回视图私有实例,
    /// 保证功能不因初始化顺序而失效(此时跨窗口共享不成立,属于降级)。
    fn selection_buffer(&mut self) -> Arc<SelectionBuffer> {
        if let Some(buffer) = &self.selection_buffer {
            return Arc::clone(buffer);
        }
        let buffer = Arc::new(SelectionBuffer::new());
        self.selection_buffer = Some(Arc::clone(&buffer));
        buffer
    }

    /// 视口坐标 → 单元格(栅格坐标)。
    ///
    /// 画布左上角由 canvas prepaint 回填;行列数用视图当前网格尺寸。
    /// 返回值的行号已换算成 grid 行(减去回看偏移)。
    fn cell_from_position(&self, position: Point<Pixels>) -> Option<DragAnchor> {
        let (origin_x, origin_y) = self.canvas_origin.get();
        let geometry = GridGeometry {
            origin_x,
            origin_y,
            cell_width: self.cell_width,
            cell_height: CELL_HEIGHT,
            rows: self.rows as usize,
            cols: self.cols as usize,
        };
        let point = geometry.cell_at(f32::from(position.x), f32::from(position.y))?;
        // 回看历史时,视口顶行对应 grid 的 Line(-display_offset)
        let offset = { self.session.term.lock().grid().display_offset() as i32 };
        Some(DragAnchor {
            line: point.row as i32 - offset,
            col: point.col,
            side_left: point.side_left,
        })
    }

    /// 鼠标按下:左键开始拖选(SEL-001),中键粘贴选区缓冲(SEL-002)。
    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 点击终端要收回键盘焦点,否则选区后键盘输入落到别处
        self.focus.focus(window);

        // MOUSE-001/002:程序接管鼠标时终端不抢事件;Shift 强制由终端处理
        let mouse_reporting = {
            let term = self.session.term.lock();
            term.mode().intersects(TermMode::MOUSE_MODE)
        };
        if !terminal_handles_mouse(mouse_reporting, event.modifiers.shift) {
            return;
        }

        match event.button {
            MouseButton::Left => {
                let Some(anchor) = self.cell_from_position(event.position) else {
                    return;
                };
                self.drag_anchor = Some(anchor);
                {
                    let mut term = self.session.term.lock();
                    // 开始新的选择:旧高亮立即消失(缓冲内容保持不变)
                    term.selection = Some(Selection::new(
                        SelectionType::Simple,
                        TermPoint::new(Line(anchor.line), Column(anchor.col)),
                        side_of(anchor.side_left),
                    ));
                }
                self.refresh_snapshot();
                cx.notify();
            }
            MouseButton::Middle => self.paste_selection_buffer(cx),
            _ => {}
        }
    }

    /// 鼠标移动:拖动中把选区末端移到当前单元格。
    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag_anchor.is_none() {
            return;
        }
        // 左键已松开但没收到 mouse_up(例如在窗口外松开):结束拖选
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag_anchor = None;
            return;
        }
        let Some(end) = self.cell_from_position(event.position) else {
            return;
        };
        {
            let mut term = self.session.term.lock();
            if let Some(selection) = term.selection.as_mut() {
                selection.update(
                    TermPoint::new(Line(end.line), Column(end.col)),
                    side_of(end.side_left),
                );
            }
        }
        self.refresh_snapshot();
        cx.notify();
    }

    /// 鼠标抬起:左键抬起时把选中文本写入选区缓冲(SEL-001)。
    ///
    /// 空选区不覆盖已有内容(SEL-004)。
    fn handle_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left || self.drag_anchor.take().is_none() {
            return;
        }
        let text = {
            let term = self.session.term.lock();
            term.selection_to_string()
        };
        // 锁已释放:写缓冲不属于终端状态
        if let Some(text) = text {
            if !text.is_empty() {
                self.selection_buffer().set(text);
            }
        }
        cx.notify();
    }

    /// 中键粘贴选区缓冲(SEL-002):与剪贴板粘贴共用同一个清洗函数(PASTE-*)。
    fn paste_selection_buffer(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.selection_buffer().get() else {
            // 缓冲为空:不发送任何字节(发送空输入会让终端挂起)
            return;
        };
        let bracketed = {
            let term = self.session.term.lock();
            term.mode().contains(TermMode::BRACKETED_PASTE)
        };
        let payload = sanitize_paste(&text, bracketed);
        self.session.write(payload.as_bytes());
        cx.notify();
    }
}

/// 注册窗口级鼠标监听(选区与中键粘贴)。
///
/// 不用元素级 `on_mouse_down/on_mouse_move`:元素级监听带 `hitbox.is_hovered` 过滤,
/// 拖到画布外就收不到移动事件,选区无法延伸到边缘。窗口级监听只按
/// `bounds` 与当前拖选状态自行过滤,语义更贴近终端。
fn register_mouse_handlers(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    entity: Entity<TerminalView>,
) {
    // 按下:仅限画布区域(避免点滚动条/标题栏时误粘贴或误开选区)
    let down_bounds = bounds;
    let down_entity = entity.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !down_bounds.contains(&event.position) {
            return;
        }
        down_entity.update(cx, |view, cx| {
            view.handle_mouse_down(event, window, cx);
        });
    });

    // 移动:拖选进行中才处理;指针离开画布时由 gpui 的 SetCapture 继续投递
    let move_bounds = bounds;
    let move_entity = entity.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        move_entity.update(cx, |view, cx| {
            if view.drag_anchor.is_none() {
                return;
            }
            // 拖出画布时把位置钳制到画布边缘(选区延伸到边界,不越界)
            let clamped = Point::new(
                event.position.x.max(move_bounds.origin.x),
                event.position.y.max(move_bounds.origin.y),
            );
            let clamped = Point::new(
                clamped.x.min(move_bounds.origin.x + move_bounds.size.width),
                clamped
                    .y
                    .min(move_bounds.origin.y + move_bounds.size.height),
            );
            let synthetic = MouseMoveEvent {
                position: clamped,
                pressed_button: event.pressed_button,
                modifiers: event.modifiers,
            };
            view.handle_mouse_move(&synthetic, cx);
        });
    });

    // 抬起:拖选完成时写选区缓冲
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        entity.update(cx, |view, cx| {
            view.handle_mouse_up(event, cx);
        });
    });
}

/// 画选区高亮:按快照里的每行列区间画半透明矩形。
///
/// 画在背景块之后、文本之前:覆盖背景但不影响文字可读性,也不需要把文本 run
/// 再按选中状态切分(宽字符列与文本列的对齐问题保持与既有渲染一致)。
fn paint_selection(
    snapshot: &GridSnapshot,
    cell_width: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    for (row, span) in snapshot.selection.iter().enumerate() {
        let Some((start, end)) = span else {
            continue;
        };
        if end < start {
            continue;
        }
        let x = bounds.origin.x + px(*start as f32 * cell_width);
        let y = bounds.origin.y + px(row as f32 * CELL_HEIGHT);
        let width = px((end - start + 1) as f32 * cell_width);
        let quad = Bounds::new(Point::new(x, y), size(width, px(CELL_HEIGHT)));
        let color: gpui::Hsla = rgb(terminal::SELECTION_BG).into();
        window.paint_quad(fill(quad, color.opacity(0.35)));
    }
}

/// 单元格半侧 → alacritty 的选择端点方向。
fn side_of(side_left: bool) -> Side {
    if side_left {
        Side::Left
    } else {
        Side::Right
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 先用校正后的客户区尺寸(而不是可能滞后的 viewport_size)同步终端网格,
        // 再用同一个尺寸给根节点显式宽高,保证自绘元素不会被画到窗口外。
        let layout = Self::layout_size(window);
        // PTY 事件由 spawn_event_loop 的常驻任务驱动重绘;render 只处理视口尺寸变化
        self.sync_size(layout, cx);
        self.measure_cell_width(window);

        // 窗口从最小化恢复时,GPUI 那一帧的呈现可能不包含自绘内容,
        // 表现为标题栏按钮与滚动条消失、点击其他窗口才恢复(用户实测)。
        // 这里在窗口重新激活后延迟一小段再刷新一次,确保有一帧完整的重绘被提交。
        if self.activation_subscription.is_none() {
            let subscription = cx.observe_window_activation(window, |_view, window, cx| {
                if window.is_window_active() {
                    cx.spawn(async move |this, cx| {
                        gpui::Timer::after(std::time::Duration::from_millis(120)).await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                }
            });
            self.activation_subscription = Some(subscription);
        }

        let snapshot = Arc::clone(&self.snapshot);
        let focus = self.focus.clone();
        let cell_width = self.cell_width;
        let this = cx.entity();
        // 鼠标事件监听注册在绘制阶段(Window::on_mouse_event 的约束),
        // 闭包只能拿共享引用,因此画布原点通过 Rc<Cell<_>> 共享给 paint 与鼠标回调。
        let canvas_origin = Rc::clone(&self.canvas_origin);

        let titlebar = render_titlebar(
            &TitlebarModel {
                title: self.title.clone(),
                maximized: window.is_maximized(),
                focused: window.is_window_active(),
                controls: WindowControlsShown {
                    minimize: true,
                    maximize: true,
                },
                hovered: self.hovered_control,
            },
            TitlebarHandlers {
                drag: Rc::new(cx.listener(
                    |_view, _event: &MouseDownEvent, window: &mut Window, _cx| {
                        start_window_drag(window);
                    },
                )),
                double_click: Rc::new(cx.listener(
                    |_view, _event: &ClickEvent, window: &mut Window, _cx| {
                        toggle_maximize(window);
                    },
                )),
                control: Rc::new(cx.listener(
                    |_view, kind: &WindowControl, window: &mut Window, _cx| {
                        apply_window_control(*kind, window);
                    },
                )),
                hover: Rc::new(cx.listener(
                    |view, hovered: &Option<WindowControl>, _window, cx| {
                        if view.hovered_control != *hovered {
                            view.hovered_control = *hovered;
                            cx.notify();
                        }
                    },
                )),
            },
        );

        div()
            .id("terminal-root")
            // 显式宽高而不是 size_full:窗口状态变化后 gpui 的 viewport 可能滞后,
            // 用实测客户区尺寸可以让标题栏按钮与滚动条始终落在窗口内。
            .w(px(layout.0))
            .h(px(layout.1))
            .flex()
            .flex_col()
            .bg(rgb(window_theme::WINDOW_BG))
            .track_focus(&focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                view.handle_key(event, window, cx);
            }))
            .child(titlebar)
            .child(
                // 终端内容区:滚轮回看历史
                div()
                    .id("terminal-body")
                    .flex_1()
                    .w_full()
                    .flex()
                    .overflow_hidden()
                    .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, cx| {
                        view.handle_scroll_wheel(event, cx);
                    }))
                    .child(
                        // 画布占据滚动条以外的宽度:`flex_1` 而不是 `size_full`,
                        // 否则 canvas 会按整行宽度布局,把滚动条挤出可视区。
                        div().flex_1().h_full().child(
                            canvas(
                                move |bounds, _window, _cx| {
                                    // 画布原点供鼠标坐标换算:视图字段在绘制闭包里拿不到
                                    // 可变引用,走共享 Cell 回填。
                                    (f32::from(bounds.origin.x), f32::from(bounds.origin.y))
                                },
                                move |bounds, origin, window, cx| {
                                    canvas_origin.set(origin);
                                    // 注册 IME 输入处理器(paint 阶段必须;焦点在终端时平台才会投递组合文本)
                                    window.handle_input(
                                        &focus,
                                        ElementInputHandler::new(bounds, this.clone()),
                                        cx,
                                    );
                                    paint_grid(&snapshot, cell_width, bounds, window, cx);
                                    // 选区与中键粘贴用窗口级监听:按下后指针离开画布
                                    // (gpui 在按下时已 SetCapture)仍要收到移动与抬起,
                                    // 元素级 on_mouse_move 的 is_hovered 过滤会漏掉这些事件。
                                    register_mouse_handlers(window, bounds, this.clone());
                                },
                            )
                            .size_full(),
                        ),
                    )
                    .child(scrollbar_element(
                        self.scrollbar_ratio,
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

/// 标题栏拖动区按下。
///
/// - Windows:gpui 的 `WindowControlArea` NC 命中路径实测未生效,`start_window_move`
///   在该平台又是空实现,因此交给 [`term_platform::begin_window_drag`] 直连 Win32
///   (`ReleaseCapture` + `WM_NCLBUTTONDOWN`/`HTCAPTION`),由系统接管拖动。
/// - Linux(X11/Wayland):gpui 已实现 `start_window_move`,直接调用。
fn start_window_drag(window: &mut Window) {
    match term_platform::window_chrome() {
        term_platform::WindowChrome::SystemHandled => {
            term_platform::begin_window_drag();
        }
        term_platform::WindowChrome::AppHandled => window.start_window_move(),
    }
}

/// 最大化/还原切换(双击标题栏与最大化按钮共用)。
///
/// - Windows:`zoom_window` 只会 `SW_MAXIMIZE`,还原必须走
///   [`term_platform::restore_window`] 的 `ShowWindow(SW_RESTORE)`。
/// - X11/Wayland:`zoom()` 本身就是切换语义,直接调用即可。
fn toggle_maximize(window: &mut Window) {
    match term_platform::window_chrome() {
        term_platform::WindowChrome::SystemHandled => {
            if window.is_maximized() {
                term_platform::restore_window();
            } else {
                window.zoom_window();
            }
        }
        term_platform::WindowChrome::AppHandled => window.zoom_window(),
    }
}

/// 执行标题栏按钮动作。
///
/// 注意:即便按钮上设置了 `WindowControlArea`,Windows 的 NC 命中测试路径实测未生效,
/// 因此这里统一由应用侧调用 `Window` 方法,保证按钮在两平台都可用。
fn apply_window_control(kind: WindowControl, window: &mut Window) {
    match kind {
        WindowControl::Minimize => window.minimize_window(),
        WindowControl::Maximize => toggle_maximize(window),
        WindowControl::Close => window.remove_window(),
    }
}

/// 滚动条元素:轨道与 thumb 都用普通 `div`(相对定位),不经过 canvas。
///
/// 用 div 而不是 canvas 的原因与窗口按钮相同:窗口最小化后恢复时,canvas 自绘内容
/// 可能不会重新提交给合成器,表现为滚动条与按钮一起消失、点其他窗口才恢复。
///
/// `ratio` 语义:1.0 = 贴轨道底(最新内容),0.0 = 贴轨道顶(最旧内容)。
fn scrollbar_element(
    ratio: f32,
    mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    mouse_move: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let ratio = ratio.clamp(0.0, 1.0);
    // thumb 在轨道内的可移动行程是 (1 - THUMB_RATIO);用百分比定位,无需知道轨道像素高度。
    let thumb_top = ratio * (1.0 - THUMB_RATIO);

    div()
        .w(px(SCROLLBAR_WIDTH))
        .h_full()
        .bg(rgb(terminal::SCROLLBAR_TRACK))
        .relative()
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(relative(thumb_top))
                .w_full()
                .h(relative(THUMB_RATIO))
                .bg(rgb(terminal::SCROLLBAR_THUMB)),
        )
        .on_mouse_down(MouseButton::Left, mouse_down)
        .on_mouse_move(mouse_move)
        .into_any_element()
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
    window.paint_quad(fill(bounds, rgb(terminal::BG)));

    let font_family = term_platform::platform_info().default_font_family;

    // 第一遍:所有非默认背景块。必须排在选区之前,否则程序设置的背景色会盖住高亮;
    // 文本必须排在选区之后,否则高亮会压暗文字。
    for (row_idx, runs) in snapshot.rows_runs.iter().enumerate() {
        let y = bounds.origin.y + px(row_idx as f32 * CELL_HEIGHT);
        for run in runs {
            if run.style.bg == crate::grid::DEFAULT_BG {
                continue;
            }
            let x = bounds.origin.x + px(run.start_col as f32 * cell_width);
            let width = px(run.text.chars().count() as f32 * cell_width);
            let bg_bounds = Bounds::new(Point::new(x, y), size(width, px(CELL_HEIGHT)));
            window.paint_quad(fill(
                bg_bounds,
                rgb((run.style.bg[0] as u32) << 16
                    | (run.style.bg[1] as u32) << 8
                    | (run.style.bg[2] as u32)),
            ));
        }
    }

    paint_selection(snapshot, cell_width, bounds, window);

    for (row_idx, runs) in snapshot.rows_runs.iter().enumerate() {
        let y = bounds.origin.y + px(row_idx as f32 * CELL_HEIGHT);
        for run in runs {
            let x = bounds.origin.x + px(run.start_col as f32 * cell_width);

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
