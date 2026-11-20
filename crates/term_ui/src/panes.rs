//! 标签内的拆分窗格(pane)树(SPLIT-*)。
//!
//! 一个标签的终端内容可以是单个窗格,也可以是任意二叉拆分树:叶子持有
//! 一个 TerminalView,内部节点记录拆分方向与两个子树。树的结构操作
//! (拆分、关闭收缩、深度优先序)与叶子类型无关,泛型化后可用普通值
//! 做单元测试;焦点相关操作只在 Entity<TerminalView> 实例上提供。
//!
//! 设计取舍:叶子用 Entity<TerminalView> 而不是下标,与 TabsView.tabs 的
//! 持有方式一致 —— Session 独立于 View(ARCHITECTURE.md 不变量 1),pane 的
//! 增删只是视图树重组,不影响 PTY 会话的存活。

use gpui::Focusable as _;
use gpui::{App, Entity, FocusHandle, Window};

use crate::view::TerminalView;

/// 拆分方向(SPLIT-001)。
///
/// 命名与 Konsole 一致:Vertical 指分隔条是竖直线(窗格左右排列),
/// Horizontal 指分隔条是水平线(窗格上下排列)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// 垂直拆分:两个窗格左右排列,分隔条竖直。
    Vertical,
    /// 水平拆分:两个窗格上下排列,分隔条水平。
    Horizontal,
}

/// 拆分窗格树:叶子是终端视图,内部节点是拆分。
///
/// 第一期比例固定 50/50(SPLIT-005,待确认),节点因此不存比例字段。
/// 深度优先序 = 界面上从左到右(垂直拆分)或从上到下(水平拆分)的
/// 阅读顺序,窗格下标一律指这个序。
#[derive(Debug, Clone)]
pub enum PaneNode<L> {
    /// 叶子:一个终端窗格。
    Leaf(L),
    /// 内部节点:沿某方向拆成的两个子树。
    Split {
        /// 拆分方向。
        direction: SplitDirection,
        /// 第一子树(垂直=左侧,水平=上侧)。
        first: Box<PaneNode<L>>,
        /// 第二子树(垂直=右侧,水平=下侧)。
        second: Box<PaneNode<L>>,
    },
}

impl<L: Clone> PaneNode<L> {
    /// 用一个窗格内容构造单窗格树。
    pub fn leaf(view: L) -> Self {
        PaneNode::Leaf(view)
    }

    /// 沿 direction 把本节点拆分为两半:原内容作为第一子树,new_view 成为
    /// 第二子树,返回拆分后的新树(SPLIT-001)。
    ///
    /// 消费 self 而不是就地修改:子树可能多层嵌套,重建比可变借用简单。
    pub fn split(self, direction: SplitDirection, new_view: L) -> Self {
        PaneNode::Split {
            direction,
            first: Box::new(self),
            second: Box::new(PaneNode::Leaf(new_view)),
        }
    }

    /// 收集树中所有窗格,顺序为深度优先序(第一子树优先)。
    pub fn leaves(&self) -> Vec<L> {
        match self {
            PaneNode::Leaf(view) => vec![view.clone()],
            PaneNode::Split { first, second, .. } => {
                let mut out = first.leaves();
                out.extend(second.leaves());
                out
            }
        }
    }

    /// 窗格数量。
    pub fn leaf_count(&self) -> usize {
        self.leaves().len()
    }

    /// 计算每个叶子的内容区面积,顺序同 leaves(深度优先序)。
    ///
    /// 算式必须与 tabs.rs 的渲染布局完全一致:每层 Split 沿拆分方向
    /// 平分剩余空间,分隔条占 divider 宽度(从该层可用总量中扣除一次)。
    /// 50/50 固定比例(SPLIT-005);比例可调时这里与渲染必须同步修改。
    pub fn leaf_areas(&self, area: (f32, f32), divider: f32) -> Vec<(f32, f32)> {
        match self {
            PaneNode::Leaf(_) => vec![area],
            PaneNode::Split {
                direction,
                first,
                second,
            } => {
                let (w, h) = area;
                let (first_area, second_area) = match direction {
                    SplitDirection::Vertical => {
                        let half = ((w - divider) / 2.0).max(0.0);
                        ((half, h), (half, h))
                    }
                    SplitDirection::Horizontal => {
                        let half = ((h - divider) / 2.0).max(0.0);
                        ((w, half), (w, half))
                    }
                };
                let mut areas = first.leaf_areas(first_area, divider);
                areas.extend(second.leaf_areas(second_area, divider));
                areas
            }
        }
    }

    /// 按深度优先序取第 index 个窗格。
    pub fn leaf_at(&self, index: usize) -> Option<L> {
        self.leaves().into_iter().nth(index)
    }

    /// 第一个窗格(深度优先序)。
    fn first_leaf(&self) -> Option<L> {
        match self {
            PaneNode::Leaf(view) => Some(view.clone()),
            PaneNode::Split { first, .. } => first.first_leaf(),
        }
    }

    /// 最后一个窗格(深度优先序)。
    fn last_leaf(&self) -> Option<L> {
        match self {
            PaneNode::Leaf(view) => Some(view.clone()),
            PaneNode::Split { second, .. } => second.last_leaf(),
        }
    }

    /// 关闭第 index 个窗格:返回其兄弟窗格与收缩后的新树(SPLIT-002)。
    ///
    /// 关闭后兄弟窗格占据被释放父节点的整个空间(树收缩,不出现空洞)。
    /// 两种情况返回 None,由调用方决定上层动作:
    /// - 树只有一个窗格:关闭它等价于关闭标签(SPLIT-002);
    /// - index 越界:视为防御性输入,忽略。
    pub fn close_leaf(self, index: usize) -> Option<(L, Self)> {
        match self {
            // 单窗格树:没有"兄弟",关闭语义上交(关标签/关窗口)
            PaneNode::Leaf(_) => None,
            PaneNode::Split {
                direction,
                first,
                second,
            } => {
                let first_count = first.leaf_count();
                if index >= first_count + second.leaf_count() {
                    return None;
                }
                if index < first_count {
                    if first_count == 1 {
                        // 关闭的是第一子树的唯一窗格:兄弟在第二子树,
                        // 第二子树接管整个节点
                        let sibling = second.first_leaf()?;
                        return Some((sibling, *second));
                    }
                    let (sibling, new_first) = first.close_leaf(index)?;
                    return Some((
                        sibling,
                        PaneNode::Split {
                            direction,
                            first: Box::new(new_first),
                            second,
                        },
                    ));
                }
                // index 命中第二子树
                if second.leaf_count() == 1 {
                    // 关闭的是第二子树的唯一窗格:兄弟是第一子树的最后
                    // 一个窗格,第一子树接管整个节点
                    let sibling = first.last_leaf()?;
                    return Some((sibling, *first));
                }
                let (sibling, new_second) = second.close_leaf(index - first_count)?;
                Some((
                    sibling,
                    PaneNode::Split {
                        direction,
                        first,
                        second: Box::new(new_second),
                    },
                ))
            }
        }
    }
}

/// 实际使用的窗格树:叶子是终端视图实体。
pub type TerminalPaneTree = PaneNode<Entity<TerminalView>>;

/// 终端窗格树的焦点操作:只有 Entity 形态的树才需要。
impl PaneNode<Entity<TerminalView>> {
    /// 收集所有窗格的焦点句柄(顺序同 leaves)。
    pub fn focus_handles(&self, cx: &App) -> Vec<FocusHandle> {
        self.leaves()
            .into_iter()
            .map(|view| view.read(cx).focus_handle(cx))
            .collect()
    }

    /// 判断哪个窗格持有键盘焦点,返回其深度优先序下标(SPLIT-006)。
    ///
    /// 焦点不在本树的任何终端上时返回 None(调用方退回 0 号窗格)。
    pub fn focused_leaf(&self, window: &Window, cx: &App) -> Option<usize> {
        for (index, view) in self.leaves().into_iter().enumerate() {
            let handle = view.read(cx).focus_handle(cx);
            if handle.is_focused(window) || handle.contains_focused(window, cx) {
                return Some(index);
            }
        }
        None
    }

    /// 返回持有焦点的窗格实体;焦点不在本树时回退到 0 号窗格。
    ///
    /// 复制/粘贴/清除历史这类"对当前终端的操作"都走这里:用户看到键盘
    /// 输入落在哪个窗格,操作就应该作用于哪个窗格(SPLIT-006 的推论)。
    pub fn focused_or_first_leaf(&self, window: &Window, cx: &App) -> Option<Entity<TerminalView>> {
        let leaves = self.leaves();
        self.focused_leaf(window, cx)
            .and_then(|index| leaves.get(index).cloned())
            .or_else(|| leaves.into_iter().next())
    }

    /// 焦点从第 current 个窗格沿方向移到相邻窗格,返回目标下标(SPLIT-006)。
    ///
    /// 相邻 = 两窗格在树上共享一个方向匹配的分割节点且分居两侧;同一方向
    /// 上更深的分割边界优先(先把本子树内走完,走不出去才跨界)。
    /// 两端不环绕;没有相邻窗格时返回 None,调用方保持焦点不变。
    pub fn neighbor(
        &self,
        current: usize,
        direction: SplitDirection,
        forward: bool,
        cx: &App,
    ) -> Option<usize> {
        let leaves = self.leaves();
        let current_view = leaves.get(current)?.clone();
        let current_handle = current_view.read(cx).focus_handle(cx);
        let target = self.neighbor_from(&current_handle, direction, forward, cx)?;
        leaves.iter().position(|view| *view == target)
    }

    /// 树内递归:返回与 handle 所在窗格按方向相邻的窗格实体。
    ///
    /// 只在方向匹配的分割节点上产生跨边界目标;方向不匹配的节点只向
    /// 包含焦点的子树继续递归(焦点不可能穿越不匹配的边界)。
    fn neighbor_from(
        &self,
        handle: &FocusHandle,
        direction: SplitDirection,
        forward: bool,
        cx: &App,
    ) -> Option<Entity<TerminalView>> {
        match self {
            // 走到叶子还没遇到方向匹配的边界:该方向上没有相邻窗格
            PaneNode::Leaf(_) => None,
            PaneNode::Split {
                direction: dir,
                first,
                second,
            } => {
                if first.contains_handle(handle, cx) {
                    let deeper = first.neighbor_from(handle, direction, forward, cx);
                    if *dir == direction && forward {
                        // 子树内没有更近的边界时,跨过本节点进入第二子树
                        return deeper.or_else(|| second.first_leaf());
                    }
                    return deeper;
                }
                if second.contains_handle(handle, cx) {
                    let deeper = second.neighbor_from(handle, direction, forward, cx);
                    if *dir == direction && !forward {
                        return deeper.or_else(|| first.last_leaf());
                    }
                    return deeper;
                }
                None
            }
        }
    }

    /// 对每个叶子执行一次更新(顺序:深度优先,带叶子下标)。
    ///
    /// chrome 高度、窗格内容区尺寸这类"容器层决定的属性"由 TabsView 统一
    /// 回填,叶子自己不知道自己在树里的位置。下标与 leaf_areas 的返回
    /// 顺序一致,调用方据此取每个窗格的面积。
    pub fn update_leaves(
        &self,
        cx: &mut gpui::App,
        f: impl Fn(usize, &mut TerminalView, &mut gpui::Context<TerminalView>),
    ) {
        for (index, view) in self.leaves().into_iter().enumerate() {
            view.update(cx, |view, cx| f(index, view, cx));
        }
    }

    /// 树内是否包含持有 handle 的窗格。
    fn contains_handle(&self, handle: &FocusHandle, cx: &App) -> bool {
        match self {
            PaneNode::Leaf(view) => view.read(cx).focus_handle(cx) == *handle,
            PaneNode::Split { first, second, .. } => {
                first.contains_handle(handle, cx) || second.contains_handle(handle, cx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestNode = PaneNode<u8>;

    /// V(a,b):垂直拆分,a 左 b 右;H(a,b):水平拆分,a 上 b 下。
    fn v(first: TestNode, second: TestNode) -> TestNode {
        TestNode::Split {
            direction: SplitDirection::Vertical,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn h(first: TestNode, second: TestNode) -> TestNode {
        TestNode::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn leaf(n: u8) -> TestNode {
        TestNode::leaf(n)
    }

    /// SPLIT-001:两种拆分方向是不同的值,防止 V/H 在 UI 分发时混用。
    #[test]
    fn split_001_directions_are_distinct() {
        assert_ne!(SplitDirection::Vertical, SplitDirection::Horizontal);
    }

    /// SPLIT-001:深度优先序 = 从左到右 / 从上到下;新窗格排在末尾。
    #[test]
    fn split_001_leaf_order_is_reading_order() {
        let tree = v(h(leaf(0), leaf(1)), leaf(2));
        assert_eq!(tree.leaves(), vec![0, 1, 2]);
        assert_eq!(tree.leaf_at(0), Some(0));
        assert_eq!(tree.leaf_at(2), Some(2));
        assert_eq!(tree.leaf_at(3), None);
    }

    /// SPLIT-002:关闭叶子后树收缩,兄弟接管被释放的空间。
    #[test]
    fn split_002_close_shrinks_tree_with_sibling_taking_over() {
        // V(a,b) 关 a:兄弟 b(=1),树收缩为单叶 b
        let (sibling, tree) = v(leaf(0), leaf(1)).close_leaf(0).unwrap();
        assert_eq!(sibling, 1, "兄弟是右半窗格");
        assert_eq!(tree.leaves(), vec![1]);
        // V(a,b) 关 b:兄弟 a(=0),树收缩为单叶 a
        let (sibling, tree) = v(leaf(0), leaf(1)).close_leaf(1).unwrap();
        assert_eq!(sibling, 0, "兄弟是左半窗格");
        assert_eq!(tree.leaves(), vec![0]);
    }

    /// SPLIT-002:嵌套树关闭内部窗格,只收缩命中的一层。
    #[test]
    fn split_002_close_inner_leaf_keeps_other_branch() {
        // V(V(a,b),c) 关 b:a 与 b 互为兄弟(兄弟=a=0),c 不受影响 → V(a,c)
        let (sibling, tree) = v(v(leaf(0), leaf(1)), leaf(2)).close_leaf(1).unwrap();
        assert_eq!(sibling, 0);
        assert_eq!(tree.leaves(), vec![0, 2]);
        // H(V(a,b), H(c,d)) 关 c:d 接管右半 → H(V(a,b), d)
        let (sibling, tree) = h(v(leaf(0), leaf(1)), h(leaf(2), leaf(3)))
            .close_leaf(2)
            .unwrap();
        assert_eq!(sibling, 3);
        assert_eq!(tree.leaves(), vec![0, 1, 3]);
    }

    /// SPLIT-002:关闭跨方向边界上的窗格,对侧整个子树接管。
    #[test]
    fn split_002_close_boundary_leaf_collapses_to_side_subtree() {
        // V(V(a,b),c) 关 c(右半):左半 V(a,b) 接管,兄弟是它最后一个窗格 b
        let (sibling, tree) = v(v(leaf(0), leaf(1)), leaf(2)).close_leaf(2).unwrap();
        assert_eq!(sibling, 1);
        assert_eq!(tree.leaves(), vec![0, 1]);
        // 对称:关左半唯一窗格 V(a,c) 的 a → 右半 c 接管(兄弟=c=1)
        let (sibling, tree) = v(leaf(0), leaf(1)).close_leaf(0).unwrap();
        assert_eq!(sibling, 1);
        assert_eq!(tree.leaves(), vec![1]);
    }

    /// SPLIT-002:关闭最后一个窗格返回 None(上层等价关闭标签);越界也返回 None。
    #[test]
    fn split_002_close_last_leaf_and_out_of_range_return_none() {
        // close_leaf 返回含 PaneNode 的元组,未派生 PartialEq,用 is_none 断言
        assert!(leaf(0).close_leaf(0).is_none(), "单窗格关闭应上交标签层");
        assert!(v(leaf(0), leaf(1)).close_leaf(2).is_none(), "越界应被忽略");
        assert!(v(leaf(0), leaf(1)).close_leaf(9).is_none(), "越界应被忽略");
    }

    /// SPLIT-004:面积按拆分方向平分,分隔条宽度从总量中扣除。
    #[test]
    fn split_004_areas_split_evenly_with_divider() {
        const D: f32 = 2.0;
        // 单窗格:占满全部面积
        assert_eq!(leaf(0).leaf_areas((800.0, 600.0), D), vec![(800.0, 600.0)]);
        // 垂直拆分:左右各半,宽度和 = 总宽 - 分隔条
        let areas = v(leaf(0), leaf(1)).leaf_areas((800.0, 600.0), D);
        assert_eq!(areas, vec![(399.0, 600.0), (399.0, 600.0)]);
        assert!((areas[0].0 + areas[1].0 + D - 800.0).abs() < 1e-6);
        // 水平拆分:上下各半
        let areas = h(leaf(0), leaf(1)).leaf_areas((800.0, 600.0), D);
        assert_eq!(areas, vec![(800.0, 299.0), (800.0, 299.0)]);
    }

    /// SPLIT-004:嵌套树按层级递归平分,面积顺序与深度优先序一致。
    #[test]
    fn split_004_nested_areas_follow_dfs_order() {
        const D: f32 = 2.0;
        // V(V(a,b),c):左半再左右分,右半整块
        let areas = v(v(leaf(0), leaf(1)), leaf(2)).leaf_areas((802.0, 600.0), D);
        let half = (802.0 - D) / 2.0;
        let quarter = (half - D) / 2.0;
        assert_eq!(
            areas,
            vec![(quarter, 600.0), (quarter, 600.0), (half, 600.0),]
        );
    }

    /// SPLIT-004:面积极小(窗格被压到 0)不产生负数。
    #[test]
    fn split_004_areas_never_negative() {
        const D: f32 = 2.0;
        let areas = v(leaf(0), leaf(1)).leaf_areas((1.0, 10.0), D);
        assert!(areas.iter().all(|(w, h)| *w >= 0.0 && *h >= 0.0));
    }
}
