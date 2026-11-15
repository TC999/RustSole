//! 弹出菜单:「更多」按钮展开的下拉菜单(阶段 4 的右键菜单与菜单栏也复用本模块)。
//!
//! gpui 0.2.2 没有内置菜单组件(源码里没有 menu 模块),因此菜单完全自绘:
//! 菜单项由普通元素拼成,弹出层用 `anchored()` 定位到锚点按钮下方。
//!
//! 图标用 `div` 的边框与文字自绘:项目未注册 `AssetSource`,`svg()` 取不到资源。
//!
//! 与 titlebar.rs / tabbar.rs / toolbar.rs 一致:本模块只负责绘制与回调转发,
//! 不持有业务状态,菜单项点了做什么由 `TabsView` 决定(ARCHITECTURE.md 不变量 5)。

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, rgb, App, Hsla, IntoElement, Pixels, Point, SharedString, Window};

use crate::theme::{
    menu, MENU_ANCHOR_OFFSET, MENU_ICON_SIZE, MENU_ITEM_GAP, MENU_ITEM_HEIGHT, MENU_ITEM_PADDING,
    MENU_PADDING, MENU_SEPARATOR_HEIGHT, MENU_SEPARATOR_MARGIN, MENU_WIDTH,
};

/// 菜单项的种类。
///
/// 与 Konsole 的「更多」菜单一一对应。多数项在本阶段没有对应功能,
/// 显示为置灰(不可用),等各自功能落地后再启用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItemId {
    /// 新建窗口:打开一个新的终端窗口。
    NewWindow,
    /// 输出保存为:把当前终端的输出存成文件。
    SaveOutputAs,
    /// 全屏模式:隐藏标题栏与工具栏,终端占满窗口。
    FullScreen,
    /// 清除回滚历史:丢弃已回看过的历史行。
    ClearHistory,
    /// 放大字体。
    ZoomIn,
    /// 重置字体大小。
    ResetZoom,
    /// 缩小字体。
    ZoomOut,
    /// 新建配置方案(Profile)。
    NewProfile,
    /// 切换配置方案(Profile)。
    SwitchProfile,
    /// 视图相关设置。
    View,
    /// 书签。
    Bookmarks,
    /// 插件。
    Plugins,
    /// 设置。
    Settings,
    /// 帮助。
    Help,
    /// 更多。
    More,
}

/// 菜单条目的种类:普通动作、分隔线还是子菜单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    /// 普通动作:点击后执行。
    Action(MenuItemId),
    /// 分隔线。
    Separator,
    /// 子菜单:右侧有箭头,悬停展开。
    Submenu(MenuItemId),
}

/// 一个菜单项的呈现数据(全部来自视图状态,不含 gpui 句柄)。
#[derive(Debug, Clone)]
pub struct MenuItem {
    /// 菜单项种类。
    pub id: MenuItemId,
    /// 显示文字。
    pub label: SharedString,
    /// 右侧显示的快捷键文字(如 `Ctrl+Shift+N`);None 表示不显示。
    pub shortcut: Option<SharedString>,
    /// 是否带勾选框。
    pub checkable: bool,
    /// 勾选框当前是否勾选。
    pub checked: bool,
    /// 是否可用。不可用项置灰且不响应点击。
    pub enabled: bool,
    /// 是否是子菜单(右侧显示箭头)。
    pub submenu: bool,
}

/// 菜单项点击回调。
///
/// 签名与 `Context::listener` 的产物一致(第一个参数是视图实体的引用),
/// 因此视图可以直接用 `Rc::new(cx.listener(...))` 构造。
pub type MenuClickHandler = Rc<dyn Fn(&MenuItemId, &mut Window, &mut App)>;
/// 菜单项悬停回调(Some = 某个子菜单项,None = 离开所有子菜单项)。
pub type MenuHoverHandler = Rc<dyn Fn(&Option<MenuItemId>, &mut Window, &mut App)>;
/// 菜单面板外按下鼠标的回调。
pub type MenuOutsideHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// 菜单回调。
pub struct MenuHandlers {
    /// 某个菜单项被点击(只有可用项才会触发)。
    pub click: MenuClickHandler,
    /// 悬停到某个子菜单项上(用于展开二级菜单)。
    pub hover: MenuHoverHandler,
    /// 在菜单面板外按下鼠标(用于关闭菜单)。
    pub outside_press: MenuOutsideHandler,
}

/// 渲染弹出菜单面板。
///
/// `anchor` 是锚点按钮的左上角(窗口坐标),菜单贴着它的下边缘展开。
/// `window_width` 是窗口宽度:菜单靠右放不下时改为右对齐,避免被窗口裁掉。
pub fn render_menu(
    entries: &[MenuEntry],
    items: &[MenuItem],
    anchor: Point<Pixels>,
    window_width: f32,
    handlers: MenuHandlers,
) -> impl IntoElement {
    // 菜单右边缘超出窗口时改为右对齐(右边缘贴着窗口右缘)。
    let menu_left = if f32::from(anchor.x) + MENU_WIDTH > window_width {
        px(window_width - MENU_WIDTH)
    } else {
        anchor.x
    };
    // 面板外的按下事件:用于关闭菜单。
    //
    // 用 `on_mouse_down_out` 而不是窗口级监听:窗口级监听分不清点击落在
    // 菜单面板内还是外,会把菜单项的点击也当成外部点击(菜单项就永远点不到)。
    // `on_mouse_down_out` 只在鼠标不在面板内时触发,语义正好。
    let outside = Rc::clone(&handlers.outside_press);
    let mut panel = div()
        .id("menu-panel")
        .flex()
        .flex_col()
        .w(px(MENU_WIDTH))
        .p(px(MENU_PADDING))
        .bg(rgb(menu::BG))
        .border_1()
        .border_color(rgb(menu::SEPARATOR))
        .rounded(px(4.0))
        .on_mouse_down_out(move |_event, window, cx| (outside)(window, cx));

    for entry in entries {
        match entry {
            MenuEntry::Separator => panel = panel.child(separator()),
            MenuEntry::Action(id) | MenuEntry::Submenu(id) => {
                let Some(item) = items.iter().find(|item| item.id == *id) else {
                    // 条目与呈现数据对不上:跳过而不是 panic(数据不一致不该拖垮界面)。
                    continue;
                };
                panel = panel.child(menu_row(item, &handlers));
            }
        }
    }

    // 菜单面板用绝对定位落在锚点下方。
    //
    // 为什么不用 gpui 的 `anchored()`:它在窗口边缘会翻转锚点、并按自身布局 bounds
    // 做偏移换算,实测弹出位置与锚点有偏差(菜单跑到窗口中部)。这里直接给明确的
    // left/top,坐标就是点击位置(窗口坐标),行为可预期。
    //
    // 坐标基准是根节点(tabs-root)的左上角,根节点加了 `.relative()` 后与窗口原点一致。
    div()
        .id("menu-layer")
        .absolute()
        .left(menu_left)
        .top(anchor.y + px(MENU_ANCHOR_OFFSET))
        .w(px(MENU_WIDTH))
        .child(panel)
}

/// 单个菜单项。
///
/// 布局:图标 | 文字 …… 快捷键 | 子菜单箭头
/// 不可用项置灰且不注册点击。
fn menu_row(item: &MenuItem, handlers: &MenuHandlers) -> gpui::AnyElement {
    let id = item.id;
    let enabled = item.enabled;

    let (text_color, icon_color): (Hsla, Hsla) = if enabled {
        (rgb(menu::TEXT).into(), rgb(menu::ICON).into())
    } else {
        (
            rgb(menu::TEXT_DISABLED).into(),
            rgb(menu::ICON_DISABLED).into(),
        )
    };

    let mut row = div()
        .id(("menu-item", id as usize))
        .flex()
        .flex_row()
        .items_center()
        .h(px(MENU_ITEM_HEIGHT))
        .px(px(MENU_ITEM_PADDING))
        .gap(px(MENU_ITEM_GAP))
        .rounded(px(3.0));

    row = row.child(item_icon(item, icon_color));

    row = row.child(
        div()
            .flex_1()
            .min_w_0()
            .text_sm()
            .whitespace_nowrap()
            .text_color(text_color)
            .child(item.label.clone()),
    );

    if let Some(shortcut) = item.shortcut.clone() {
        row = row.child(
            div()
                .text_xs()
                .whitespace_nowrap()
                .text_color(rgb(menu::SHORTCUT))
                .child(shortcut),
        );
    }

    if item.submenu {
        row = row.child(submenu_arrow(icon_color));
    }

    if enabled {
        let click = Rc::clone(&handlers.click);
        row = row
            .cursor_pointer()
            .on_click(move |_event, window, cx| (click)(&id, window, cx));

        if item.submenu {
            let hover = Rc::clone(&handlers.hover);
            row = row.on_hover(move |&is_hovered, window, cx| {
                let hovered = if is_hovered { Some(id) } else { None };
                (hover)(&hovered, window, cx);
            });
        }
    }

    row.into_any_element()
}

/// 菜单项左侧的图标。
fn item_icon(item: &MenuItem, color: Hsla) -> gpui::AnyElement {
    if item.checkable {
        return checkbox(item.checked);
    }
    action_icon(item.id, color)
}

/// 勾选框:一个方框,勾选时在框内画两条短线拼成对勾。
fn checkbox(checked: bool) -> gpui::AnyElement {
    let size = px(13.0);
    let mut box_ = div()
        .w(size)
        .h(size)
        .border_1()
        .border_color(rgb(menu::CHECKBOX_BORDER))
        .rounded(px(2.0));
    if checked {
        box_ = box_.child(
            div()
                .absolute()
                .top(px(4.0))
                .left(px(3.0))
                .w(px(3.5))
                .h(px(1.5))
                .bg(rgb(menu::CHECKMARK))
                .rounded(px(1.0)),
        );
        box_ = box_.child(
            div()
                .absolute()
                .top(px(3.0))
                .left(px(5.0))
                .w(px(6.0))
                .h(px(1.5))
                .bg(rgb(menu::CHECKMARK))
                .rounded(px(1.0)),
        );
    }
    box_.into_any_element()
}

/// 子菜单箭头:一个朝右的尖角。
fn submenu_arrow(color: Hsla) -> gpui::AnyElement {
    div()
        .w(px(7.0))
        .h(px(9.0))
        .relative()
        .child(
            div()
                .absolute()
                .top(px(1.5))
                .left(px(1.0))
                .w(px(5.0))
                .h(px(1.5))
                .bg(color)
                .rounded(px(1.0)),
        )
        .child(
            div()
                .absolute()
                .top(px(6.0))
                .left(px(1.0))
                .w(px(5.0))
                .h(px(1.5))
                .bg(color)
                .rounded(px(1.0)),
        )
        .into_any_element()
}

/// 分隔线:上下留白 + 一条细线。
fn separator() -> gpui::AnyElement {
    div()
        .h(px(MENU_SEPARATOR_HEIGHT + MENU_SEPARATOR_MARGIN * 2.0))
        .flex()
        .items_center()
        .child(
            div()
                .w_full()
                .h(px(MENU_SEPARATOR_HEIGHT))
                .bg(rgb(menu::SEPARATOR)),
        )
        .into_any_element()
}

/// 各菜单项的功能图标。
///
/// 全部用边框与文字拼出线性图标,风格与工具栏图标一致。
/// 勾选项(全屏模式)不走这里,由 [`checkbox`] 画方框。
fn action_icon(id: MenuItemId, color: Hsla) -> gpui::AnyElement {
    let size = px(MENU_ICON_SIZE);
    match id {
        // 新建窗口:一个窗口轮廓加一个小窗口。
        MenuItemId::NewWindow => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(0.0), px(0.0), px(15.0), px(12.0)))
            .child(frame(color, px(5.0), px(6.0), px(10.0), px(9.0)))
            .into_any_element(),
        // 输出保存为:一个软盘形方块。
        MenuItemId::SaveOutputAs => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(0.0), px(0.0), px(16.0), px(16.0)))
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(4.0))
                    .w(px(7.0))
                    .h(px(4.0))
                    .bg(rgb(menu::BG)),
            )
            .into_any_element(),
        // 全屏模式:窗口轮廓(实际显示为勾选框)。
        MenuItemId::FullScreen => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(0.0), px(0.0), px(16.0), px(16.0)))
            .into_any_element(),
        // 清除回滚历史:一个桶(桶身 + 桶盖)。
        MenuItemId::ClearHistory => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(3.0), px(5.0), px(10.0), px(11.0)))
            .child(
                div()
                    .absolute()
                    .top(px(3.0))
                    .left(px(1.0))
                    .w(px(14.0))
                    .h(px(1.5))
                    .bg(color),
            )
            .into_any_element(),
        // 放大字体:一个大写 A。
        MenuItemId::ZoomIn => div()
            .flex()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .text_lg()
            .text_color(color)
            .child("A")
            .into_any_element(),
        // 重置字体大小:一个小写 A。
        MenuItemId::ResetZoom => div()
            .flex()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .text_sm()
            .text_color(color)
            .child("A")
            .into_any_element(),
        // 缩小字体:更小的小写 A。
        MenuItemId::ZoomOut => div()
            .flex()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .text_xs()
            .text_color(color)
            .child("A")
            .into_any_element(),
        // 新建配置方案:一张纸加一个加号。
        MenuItemId::NewProfile => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(2.0), px(0.0), px(12.0), px(16.0)))
            .child(plus(color))
            .into_any_element(),
        // 切换配置方案:两张纸错开。
        MenuItemId::SwitchProfile => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(0.0), px(0.0), px(11.0), px(13.0)))
            .child(frame(color, px(5.0), px(3.0), px(11.0), px(13.0)))
            .into_any_element(),
        // 视图:一只眼睛。
        MenuItemId::View => div()
            .relative()
            .w(size)
            .h(size)
            .child(
                div()
                    .absolute()
                    .top(px(5.0))
                    .left(px(0.0))
                    .w(px(16.0))
                    .h(px(6.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(3.0)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .left(px(6.0))
                    .w(px(4.0))
                    .h(px(4.0))
                    .bg(color)
                    .rounded(px(2.0)),
            )
            .into_any_element(),
        // 书签:一个书签形状。
        MenuItemId::Bookmarks => div()
            .relative()
            .w(size)
            .h(size)
            .child(
                div()
                    .absolute()
                    .top(px(1.0))
                    .left(px(4.0))
                    .w(px(8.0))
                    .h(px(14.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(1.0)),
            )
            .into_any_element(),
        // 插件:一个方块加一个缺口。
        MenuItemId::Plugins => div()
            .relative()
            .w(size)
            .h(size)
            .child(frame(color, px(1.0), px(1.0), px(14.0), px(14.0)))
            .child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .left(px(12.0))
                    .w(px(4.0))
                    .h(px(4.0))
                    .bg(rgb(menu::BG)),
            )
            .into_any_element(),
        // 设置:一个齿轮。
        MenuItemId::Settings => div()
            .relative()
            .w(size)
            .h(size)
            .child(
                div()
                    .absolute()
                    .top(px(5.0))
                    .left(px(5.0))
                    .w(px(6.0))
                    .h(px(6.0))
                    .border_1()
                    .border_color(color)
                    .rounded(px(1.0)),
            )
            .child(gear_tooth(color, px(7.0), px(0.0), px(2.0), px(5.0)))
            .child(gear_tooth(color, px(7.0), px(11.0), px(2.0), px(5.0)))
            .child(gear_tooth(color, px(0.0), px(7.0), px(5.0), px(2.0)))
            .child(gear_tooth(color, px(11.0), px(7.0), px(5.0), px(2.0)))
            .into_any_element(),
        // 帮助:一个问号。
        MenuItemId::Help => div()
            .flex()
            .items_center()
            .justify_center()
            .w(size)
            .h(size)
            .text_lg()
            .text_color(color)
            .child("?")
            .into_any_element(),
        // 更多:三个竖点。
        MenuItemId::More => div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.0))
            .w(size)
            .h(size)
            .child(dot(color))
            .child(dot(color))
            .child(dot(color))
            .into_any_element(),
    }
}

/// 空心矩形边框(菜单图标的基本构件)。
fn frame(color: Hsla, left: Pixels, top: Pixels, w: Pixels, h: Pixels) -> gpui::AnyElement {
    div()
        .absolute()
        .top(top)
        .left(left)
        .w(w)
        .h(h)
        .border_1()
        .border_color(color)
        .rounded(px(1.0))
        .into_any_element()
}

/// 一个加号(新建配置方案图标用)。
fn plus(color: Hsla) -> gpui::AnyElement {
    div()
        .absolute()
        .top(px(5.0))
        .left(px(3.0))
        .w(px(10.0))
        .h(px(10.0))
        .child(
            div()
                .absolute()
                .top(px(4.0))
                .left(px(0.0))
                .w(px(10.0))
                .h(px(1.5))
                .bg(color),
        )
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(4.0))
                .w(px(1.5))
                .h(px(10.0))
                .bg(color),
        )
        .into_any_element()
}

/// 齿轮的一个齿。
fn gear_tooth(color: Hsla, left: Pixels, top: Pixels, w: Pixels, h: Pixels) -> gpui::AnyElement {
    div()
        .absolute()
        .top(top)
        .left(left)
        .w(w)
        .h(h)
        .bg(color)
        .into_any_element()
}

/// 一个圆点(更多菜单图标用)。
fn dot(color: Hsla) -> gpui::AnyElement {
    div()
        .w(px(3.0))
        .h(px(3.0))
        .bg(color)
        .rounded(px(2.0))
        .into_any_element()
}
