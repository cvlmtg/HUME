use hume_grid::Rect;
use hume_grid::box_glyphs::{
    BOTTOM_LEFT, BOTTOM_RIGHT, CROSS, HORIZONTAL, HORIZONTAL_DOWN, HORIZONTAL_UP, TOP_LEFT,
    TOP_RIGHT, VERTICAL, VERTICAL_LEFT, VERTICAL_RIGHT,
};
use rustc_hash::FxHashMap;

use super::{PaneId, UnattachedPane};

// ---------------------------------------------------------------------------
// Layout tree
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

/// Which way [`EngineView::rotate_focused`](super::EngineView::rotate_focused)
/// moves the members of a row or column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// Each member moves right or down; the last wraps to the first.
    Forward,
    /// Each member moves left or up; the first wraps to the last.
    Backward,
}

/// Which way a step through an ordered list goes, wrapping at either end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cycle {
    Next,
    Prev,
}

impl Cycle {
    /// The index one step from `idx` in a list of `len` items, wrapping at
    /// either end. `idx` must be a position in the list, so `len > 0`.
    pub fn step(self, idx: usize, len: usize) -> usize {
        debug_assert!(idx < len, "step from {idx} in a list of {len}");
        match self {
            Cycle::Next => (idx + 1) % len,
            Cycle::Prev => (idx + len - 1) % len,
        }
    }
}

/// Refusal of [`EngineView::close_focused`](super::EngineView::close_focused)
/// and [`EngineView::rotate_focused`](super::EngineView::rotate_focused): the
/// focused pane is the layout's only pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SinglePane;

/// A side of the focused pane, for
/// [`ScreenLayout::focused_neighbor`](super::ScreenLayout::focused_neighbor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// A single 1-cell divider segment drawn between two sibling panes at one
/// split node.
///
/// Emitted by `LayoutTree::collect_seams_into`, which walks the same
/// recursion as `collect_rects_into` off the same `split_rect` math, so seam
/// and leaf geometry can never drift apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seam {
    pub rect: Rect,
    /// The split that produced this seam. `Horizontal` (a width split)
    /// carves a 1-column-wide, full-height seam, drawn as a vertical line
    /// (`│`). `Vertical` (a height split) carves a 1-row-tall, full-width
    /// seam, drawn as a horizontal line (`─`). Same axis inversion as
    /// `LayoutTree::Split` (see `split_focused_pane`'s doc comment).
    pub direction: Direction,
}

/// Proof that a `PaneId` has been detached from every `LayoutTree` that
/// could reach it. Minted only by `FocusedLayout::close_focused` and
/// [`FocusedLayout::into_detached`], both in this module, and consumed by
/// `hume-editor`'s `drop_pane_state` to free the pane's state. Not
/// `Clone`/`Copy`: a duplicable token would let the same pane be freed
/// twice. Together with [`super::UnattachedPane`] (the opposite
/// transition, minted on insert) and `EngineView::layout`'s privacy, this
/// is the compiler-checked pane/layout sync invariant: a pane in the pool
/// with no tree leaf naming it is a leak, and a leaf naming a pane the
/// pool no longer holds is a dangling reference.
///
/// A tree outside this crate cannot mint one for a pane it names:
///
/// ```compile_fail,E0624
/// use hume_engine::pipeline::{LayoutTree, PaneId};
///
/// let _ = LayoutTree::Leaf(PaneId::default()).into_detached();
/// ```
#[must_use = "a detached pane's per-pane state must be dropped"]
#[derive(Debug)]
pub struct DetachedPane(PaneId);

impl DetachedPane {
    pub fn pane_id(&self) -> PaneId {
        self.0
    }
}

/// Recursive layout tree. Leaves reference panes; splits partition space.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutTree {
    Leaf(PaneId),
    Split {
        direction: Direction,
        /// Fraction (0.0–1.0) allocated to the first child. Derived by
        /// `equalize` from the two children's `shares_along` counts,
        /// not chosen by whatever split produced this node, so every pane
        /// sharing a split axis stays equal-sized regardless of split order.
        /// Kept as stored (not derived-on-read) state because `LayoutTree`
        /// is `pub` and constructible directly with an arbitrary ratio (see
        /// `focused_neighbor_tie_breaks_on_center_distance`); a
        /// derived-on-read field couldn't hold a ratio nothing equalized.
        /// `equalize` currently rewrites every ratio in the whole tree on
        /// every split/close; a future manual pane resize will need to keep
        /// its own ratio scoped out of that pass rather than being
        /// overwritten by an unrelated split elsewhere.
        ratio: f32,
        children: Box<(LayoutTree, LayoutTree)>,
    },
}

impl LayoutTree {
    /// Compute (PaneId, Rect) pairs for the leaf panes given the total area.
    /// Results are appended to `out` (which is not cleared; caller must clear first).
    ///
    /// `reserve_seam` mirrors the `pane-dividers` setting: `true` carves a
    /// 1-cell seam out of each split axis (see `split_rect`); `false` tiles
    /// children edge-to-edge with no reserved gap.
    pub(crate) fn collect_rects_into(
        &self,
        area: Rect,
        reserve_seam: bool,
        out: &mut Vec<(PaneId, Rect)>,
    ) {
        match self {
            LayoutTree::Leaf(id) => out.push((*id, area)),
            LayoutTree::Split {
                direction,
                ratio,
                children,
            } => {
                let (r1, _seam, r2) = split_rect(
                    area,
                    *direction == Direction::Vertical,
                    *ratio,
                    reserve_seam,
                );
                children.0.collect_rects_into(r1, reserve_seam, out);
                children.1.collect_rects_into(r2, reserve_seam, out);
            }
        }
    }

    /// `pid`'s own rect, without collecting every other leaf's: the single-
    /// target sibling of [`Self::collect_rects_into`], for callers that
    /// don't need the whole partition and would otherwise allocate one just
    /// to search it.
    pub(crate) fn find_rect(&self, pid: PaneId, area: Rect, reserve_seam: bool) -> Option<Rect> {
        match self {
            LayoutTree::Leaf(id) => (*id == pid).then_some(area),
            LayoutTree::Split {
                direction,
                ratio,
                children,
            } => {
                let (r1, _seam, r2) = split_rect(
                    area,
                    *direction == Direction::Vertical,
                    *ratio,
                    reserve_seam,
                );
                children
                    .0
                    .find_rect(pid, r1, reserve_seam)
                    .or_else(|| children.1.find_rect(pid, r2, reserve_seam))
            }
        }
    }

    /// Compute the 1-cell seam divider drawn between sibling panes at every
    /// split node, given the total area. Walks the same recursion as
    /// `collect_rects_into`, off the same `split_rect` math, so seams always
    /// align with the leaf rects computed for the same `area` this frame.
    /// Results are appended to `out` (not cleared; caller must clear first).
    ///
    /// Always reserves the seam: callers only invoke this when dividers are
    /// being drawn, which is also when `collect_rects_into` is called with
    /// `reserve_seam: true`, so the two stay aligned.
    pub(crate) fn collect_seams_into(&self, area: Rect, out: &mut Vec<Seam>) {
        if let LayoutTree::Split {
            direction,
            ratio,
            children,
        } = self
        {
            let (r1, seam, r2) = split_rect(area, *direction == Direction::Vertical, *ratio, true);
            if seam.width > 0 && seam.height > 0 {
                out.push(Seam {
                    rect: seam,
                    direction: *direction,
                });
            }
            children.0.collect_seams_into(r1, out);
            children.1.collect_seams_into(r2, out);
        }
    }

    /// Replace `Leaf(target)` with a `Split` of `(Leaf(target),
    /// Leaf(new_pane))`: `false` when `target` is not a leaf below `self`.
    /// Leaves the ratios to the caller's single `equalize` at the root, so
    /// equalization doesn't run once per level of recursion.
    fn insert_split(&mut self, target: PaneId, new_pane: PaneId, direction: Direction) -> bool {
        match self {
            LayoutTree::Leaf(id) if *id == target => {
                *self = LayoutTree::Split {
                    direction,
                    // Overwritten by the caller's `equalize()`.
                    ratio: 0.5,
                    children: Box::new((LayoutTree::Leaf(target), LayoutTree::Leaf(new_pane))),
                };
                true
            }
            LayoutTree::Leaf(_) => false,
            LayoutTree::Split { children, .. } => {
                children.0.insert_split(target, new_pane, direction)
                    || children.1.insert_split(target, new_pane, direction)
            }
        }
    }

    /// Whether `target` is reachable as a leaf in this subtree.
    pub fn contains_leaf(&self, target: PaneId) -> bool {
        match self {
            LayoutTree::Leaf(id) => *id == target,
            LayoutTree::Split { children, .. } => {
                children.0.contains_leaf(target) || children.1.contains_leaf(target)
            }
        }
    }

    /// Consume the whole tree, detaching every pane it reaches (one token
    /// per leaf). Discarding a tab's tree and freeing all of its panes
    /// become the same operation, so there is no window where the tree is
    /// gone but a pane it referenced still lives in the pool (or vice
    /// versa).
    pub(crate) fn into_detached(self) -> Vec<DetachedPane> {
        let mut out = Vec::new();
        self.collect_detached(&mut out);
        out
    }

    fn collect_detached(self, out: &mut Vec<DetachedPane>) {
        match self {
            LayoutTree::Leaf(id) => out.push(DetachedPane(id)),
            LayoutTree::Split { children, .. } => {
                let (a, b) = *children;
                a.collect_detached(out);
                b.collect_detached(out);
            }
        }
    }

    /// Leftmost leaf, found by descending into the first child at each split.
    fn first_leaf(&self) -> PaneId {
        match self {
            LayoutTree::Leaf(id) => *id,
            LayoutTree::Split { children, .. } => children.0.first_leaf(),
        }
    }

    /// Every pane id reachable in this subtree. No defined order beyond a
    /// depth-first walk. Callers that care about a specific pane (e.g. the
    /// first one) should use `Self::first_leaf` instead of reading the
    /// first element.
    ///
    /// Panes are never shared across tabs (each tab's `LayoutTree`
    /// references a disjoint set of leaves) but all live in one global pool
    /// regardless of which tab is active. See
    /// [`EngineView::active_pane_ids`](super::EngineView::active_pane_ids)
    /// for the same walk over the *live* tab's tree.
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            LayoutTree::Leaf(id) => out.push(*id),
            LayoutTree::Split { children, .. } => {
                children.0.collect_leaves(out);
                children.1.collect_leaves(out);
            }
        }
    }

    /// How many side-by-side shares this subtree occupies along `direction`'s
    /// axis: the unit [`Self::equalize`] balances a split's two children in.
    /// Named `share` because `slot` is this crate's display-line vocabulary.
    /// A leaf is always one share. A split *on* `direction`'s axis is the sum
    /// of its children's shares, since each becomes its own share of that
    /// axis. A split on the *other* axis (a stacked or side-by-side group)
    /// counts as a single share when measured across its own axis, matching
    /// the `Ctrl-W =` convention most terminal multiplexers use, where a
    /// group of stacked panes shares one column's width rather than each
    /// stacked pane claiming its own.
    fn shares_along(&self, direction: Direction) -> u32 {
        match self {
            LayoutTree::Leaf(_) => 1,
            LayoutTree::Split {
                direction: split_dir,
                children,
                ..
            } if *split_dir == direction => {
                children.0.shares_along(direction) + children.1.shares_along(direction)
            }
            LayoutTree::Split { .. } => 1,
        }
    }

    /// Re-derive every split's `ratio` from its two children's
    /// [`Self::shares_along`] counts on that split's own axis, so every pane
    /// sharing a split axis ends up the same size regardless of the order
    /// splits and closes happened in. Recurses into both children first so a
    /// nested split's ratio is set from its own children before this level
    /// reads `shares_along` on it.
    fn equalize(&mut self) {
        if let LayoutTree::Split {
            direction,
            ratio,
            children,
        } = self
        {
            children.0.equalize();
            children.1.equalize();
            let shares0 = children.0.shares_along(*direction);
            let shares1 = children.1.shares_along(*direction);
            *ratio = shares0 as f32 / (shares0 + shares1) as f32;
        }
    }

    /// The recursive body of [`FocusedLayout::rotate_focused`]: `false`
    /// when `target` is not a leaf below `self`.
    fn rotate_run(&mut self, target: PaneId, rotation: Rotation, steps: usize) -> bool {
        let LayoutTree::Split { direction, .. } = self else {
            return false;
        };
        let direction = *direction;
        let mut members = Vec::new();
        self.collect_run_members(direction, &mut members);
        if !members
            .iter()
            .any(|m| matches!(m, LayoutTree::Leaf(id) if *id == target))
        {
            return members
                .into_iter()
                .any(|m| m.rotate_run(target, rotation, steps));
        }
        // Each placeholder leaf is overwritten when the rotated trees are
        // written back below.
        let mut trees: Vec<LayoutTree> = members
            .iter_mut()
            .map(|m| std::mem::replace(&mut **m, LayoutTree::Leaf(PaneId::default())))
            .collect();
        let shift = steps % trees.len();
        match rotation {
            Rotation::Forward => trees.rotate_right(shift),
            Rotation::Backward => trees.rotate_left(shift),
        }
        for (member, tree) in members.into_iter().zip(trees) {
            *member = tree;
        }
        true
    }

    /// The members of the same-direction run rooted at `self`, in order:
    /// every descendant reached through splits on `direction` only.
    fn collect_run_members<'a>(
        &'a mut self,
        direction: Direction,
        out: &mut Vec<&'a mut LayoutTree>,
    ) {
        // Tested apart from the binding: a guarded arm binding `children` for
        // `'a` would keep `self` borrowed in the fallback arm too.
        let in_run = matches!(self, LayoutTree::Split { direction: d, .. } if *d == direction);
        if !in_run {
            out.push(self);
            return;
        }
        if let LayoutTree::Split { children, .. } = self {
            let (first, second) = &mut **children;
            first.collect_run_members(direction, out);
            second.collect_run_members(direction, out);
        }
    }

    /// Prune `Leaf(target)`, collapsing its parent `Split` onto the sibling,
    /// and return the leftmost leaf of that promoted sibling. `None` when
    /// `target` is not a leaf below a split of `self`. Leaves the ratios to
    /// the caller's `equalize`, for the same reason as [`Self::insert_split`].
    fn prune_leaf(&mut self, target: PaneId) -> Option<PaneId> {
        match self {
            LayoutTree::Leaf(_) => None,
            LayoutTree::Split { children, .. } => {
                let hit0 = matches!(&children.0, LayoutTree::Leaf(id) if *id == target);
                let hit1 = matches!(&children.1, LayoutTree::Leaf(id) if *id == target);
                if hit0 || hit1 {
                    // Promote the sibling subtree; the placeholder left behind
                    // in `keep` is discarded when `*self` is overwritten below.
                    let keep = if hit0 {
                        &mut children.1
                    } else {
                        &mut children.0
                    };
                    let sibling = std::mem::replace(keep, LayoutTree::Leaf(PaneId::default()));
                    let survivor = sibling.first_leaf();
                    *self = sibling;
                    Some(survivor)
                } else {
                    children
                        .0
                        .prune_leaf(target)
                        .or_else(|| children.1.prune_leaf(target))
                }
            }
        }
    }
}

/// A tab's layout together with its focused pane, which is always a leaf of
/// the tree. Every method that changes either one keeps that true, so a
/// focus outside the layout cannot be represented, and every query about
/// the focused pane answers without a miss case.
#[derive(Debug, PartialEq)]
pub struct FocusedLayout {
    tree: LayoutTree,
    focus: PaneId,
}

impl FocusedLayout {
    /// A freshly inserted pane as the sole, focused leaf of a new layout.
    pub fn new(pane: UnattachedPane) -> Self {
        let id = pane.pane_id();
        Self {
            tree: LayoutTree::Leaf(id),
            focus: id,
        }
    }

    /// `tree` focused on `focus`, which must be one of its leaves.
    #[cfg(test)]
    pub(crate) fn from_parts(tree: LayoutTree, focus: PaneId) -> Self {
        assert!(tree.contains_leaf(focus), "focus is a leaf of the tree");
        Self { tree, focus }
    }

    /// The layout `EngineView::new` holds until the editor installs the
    /// first real one: the null pane id, focused.
    pub(super) fn placeholder() -> Self {
        Self {
            tree: LayoutTree::Leaf(PaneId::default()),
            focus: PaneId::default(),
        }
    }

    pub fn tree(&self) -> &LayoutTree {
        &self.tree
    }

    pub fn focused(&self) -> PaneId {
        self.focus
    }

    /// Consume the layout, detaching every pane its tree reaches: one token
    /// per leaf.
    pub fn into_detached(self) -> Vec<DetachedPane> {
        self.tree.into_detached()
    }

    /// Focus `pid`, which must be a leaf of the tree.
    pub(crate) fn focus(&mut self, pid: PaneId) {
        assert!(
            self.tree.contains_leaf(pid),
            "a focus target is a leaf of the tree"
        );
        self.focus = pid;
    }

    /// The focused pane's rect within `area`.
    pub(crate) fn focused_rect(&self, area: Rect, reserve_seam: bool) -> Rect {
        self.tree
            .find_rect(self.focus, area, reserve_seam)
            .expect("the focused pane is a leaf of the tree")
    }

    /// The pane one step from the focused one in leaf order, wrapping at
    /// either end; `None` when the focused pane is the only one.
    pub(crate) fn pane_cycled_from_focused(&self, cycle: Cycle) -> Option<PaneId> {
        let leaves = self.tree.leaves();
        let len = leaves.len();
        let idx = leaves
            .iter()
            .position(|&p| p == self.focus)
            .expect("the focused pane is a leaf of the tree");
        (len > 1).then(|| leaves[cycle.step(idx, len)])
    }

    /// The nearest pane on `side` of the focused one within `area`, or
    /// `None` when no pane lies that way.
    ///
    /// Nearest by gap along `side`'s axis among panes overlapping the
    /// focused one on the other axis (a purely diagonal pane never
    /// qualifies), tie-broken on the distance between the two panes'
    /// centers on that other axis.
    pub(crate) fn focused_neighbor(
        &self,
        area: Rect,
        reserve_seam: bool,
        side: Side,
    ) -> Option<PaneId> {
        let mut rects = Vec::new();
        self.tree.collect_rects_into(area, reserve_seam, &mut rects);
        let &(_, cur) = rects
            .iter()
            .find(|(p, _)| *p == self.focus)
            .expect("the focused pane is a leaf of the tree");
        let cur_cx = cur.x + cur.width / 2;
        let cur_cy = cur.y + cur.height / 2;
        // Gap in the high 16 bits and the center distance in the low 16, so
        // one `min_by_key` orders by gap, then distance. Both are u16, so
        // neither can spill into the other.
        rects
            .into_iter()
            .filter(|&(p, _)| p != self.focus)
            .filter_map(|(pid, r)| {
                let overlaps_v = r.top() < cur.bottom() && cur.top() < r.bottom();
                let overlaps_h = r.left() < cur.right() && cur.left() < r.right();
                let (gap, perp): (u16, u16) = match side {
                    Side::Left if overlaps_v && r.right() <= cur.left() => {
                        (cur.left() - r.right(), cur_cy.abs_diff(r.y + r.height / 2))
                    }
                    Side::Right if overlaps_v && r.left() >= cur.right() => {
                        (r.left() - cur.right(), cur_cy.abs_diff(r.y + r.height / 2))
                    }
                    Side::Up if overlaps_h && r.bottom() <= cur.top() => {
                        (cur.top() - r.bottom(), cur_cx.abs_diff(r.x + r.width / 2))
                    }
                    Side::Down if overlaps_h && r.top() >= cur.bottom() => {
                        (r.top() - cur.bottom(), cur_cx.abs_diff(r.x + r.width / 2))
                    }
                    _ => return None,
                };
                Some(((gap as u32) << 16 | (perp as u32), pid))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, pid)| pid)
    }

    /// The rect the focused pane would have within `area` after
    /// [`Self::split_focused`] on `direction`, leaving this layout unchanged.
    /// `equalize` runs whole-tree, so a split can shrink every pane sharing
    /// its axis, not only the focused one; this simulates the real
    /// insert-then-equalize sequence instead of halving the current rect.
    ///
    /// The focused pane's side of the new split is never larger than the
    /// new pane's: `insert_split` makes the focused pane the first child,
    /// and `split_rect`'s integer truncation rounds the first child's share
    /// down, so a minimum-size check against this rect alone is
    /// conservative.
    pub(crate) fn predicted_split_rect(
        &self,
        area: Rect,
        reserve_seam: bool,
        direction: Direction,
    ) -> Rect {
        let mut probe = self.tree.clone();
        let inserted = probe.insert_split(self.focus, PaneId::default(), direction);
        assert!(inserted, "the focused pane is a leaf of the tree");
        probe.equalize();
        probe
            .find_rect(self.focus, area, reserve_seam)
            .expect("the focused pane is a leaf of the tree")
    }

    /// Split the focused pane, placing `pane` after it along `direction`,
    /// then re-derive every split ratio so panes sharing a split axis stay
    /// equal-sized (see `equalize`). Focus stays where it was.
    pub(crate) fn split_focused(&mut self, pane: UnattachedPane, direction: Direction) -> PaneId {
        let new_id = pane.pane_id();
        let inserted = self.tree.insert_split(self.focus, new_id, direction);
        assert!(inserted, "the focused pane is a leaf of the tree");
        self.tree.equalize();
        new_id
    }

    /// Rotate the row or column the focused pane sits in by `steps` places.
    /// That row is the largest run of same-direction splits holding the
    /// focused leaf as a direct member; a split on the other axis inside it
    /// is one member and moves whole. The tree's shape and ratios are kept:
    /// every member is one share along the run's axis (see `shares_along`),
    /// so each place keeps its size and only its contents move. Focus stays
    /// on the same pane.
    pub(crate) fn rotate_focused(
        &mut self,
        rotation: Rotation,
        steps: usize,
    ) -> Result<(), SinglePane> {
        self.refuse_single_pane()?;
        let rotated = self.tree.rotate_run(self.focus, rotation, steps);
        assert!(rotated, "the focused pane is a leaf of the tree");
        Ok(())
    }

    /// Prune the focused leaf, collapsing its parent split onto the sibling,
    /// then re-derive every split ratio so the survivors stay equal-sized
    /// (see `equalize`). Focus moves to the leftmost leaf of the promoted
    /// sibling.
    pub(crate) fn close_focused(&mut self) -> Result<DetachedPane, SinglePane> {
        self.refuse_single_pane()?;
        let survivor = self
            .tree
            .prune_leaf(self.focus)
            .expect("the focused pane is a leaf of the tree");
        self.tree.equalize();
        Ok(DetachedPane(std::mem::replace(&mut self.focus, survivor)))
    }

    pub(crate) fn refuse_single_pane(&self) -> Result<(), SinglePane> {
        match self.tree {
            LayoutTree::Leaf(_) => Err(SinglePane),
            LayoutTree::Split { .. } => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Seam junction glyphs
// ---------------------------------------------------------------------------
//
// Where a horizontal seam meets a vertical seam, the crossing cell needs a
// junction glyph (`┬ ┴ ├ ┤ ┼`) instead of a plain `│`/`─`. Connectivity at a
// cell is a 4-bit compass mask; `junction_glyph` resolves a mask to a glyph,
// and `collect_seam_arms` derives the perpendicular bits a seam contributes
// to the cells just past its two endpoints.
//
// `collect_seam_arms` touches only two cells per seam: the one beyond each
// endpoint where a perpendicular seam would be drawn if one abuts. But those
// recorded cells can land on the *interior* of the perpendicular seam being
// drawn: a child split's seam starts or ends partway along the parent seam,
// so its endpoint-adjacent record lands on an interior cell of the parent
// (see `collect_seam_arms_t_junction`: a `│` seam starting one row below a
// `─` seam records `ARM_S` on the `─` seam's interior). The draw loop
// therefore probes every cell of each seam's rect, OR-ing the seam's `base`
// mask with any arms recorded for that cell. This stays per-seam, not a
// full-frame scan, and the arms map is sparse (at most two entries per seam)
// so the lookups hit a tiny map and the per-cell buffer writes dominate.

pub(super) const ARM_N: u8 = 0b0001;
pub(super) const ARM_E: u8 = 0b0010;
pub(super) const ARM_S: u8 = 0b0100;
pub(super) const ARM_W: u8 = 0b1000;

/// Resolve a compass-bit mask (`ARM_N | ARM_E | ...`) to the box-drawing
/// glyph with exactly those arms. Masks with fewer than two bits, or with
/// only two opposite bits, fall back to a straight line. This keeps the
/// function a total resolver even though seam geometry only ever produces
/// `│ ─ ├ ┤ ┬ ┴ ┼`.
pub(super) fn junction_glyph(mask: u8) -> &'static str {
    match mask {
        m if m == ARM_N | ARM_E | ARM_S | ARM_W => CROSS,
        m if m == ARM_E | ARM_S | ARM_W => HORIZONTAL_DOWN,
        m if m == ARM_N | ARM_E | ARM_W => HORIZONTAL_UP,
        m if m == ARM_N | ARM_E | ARM_S => VERTICAL_RIGHT,
        m if m == ARM_N | ARM_S | ARM_W => VERTICAL_LEFT,
        m if m == ARM_N | ARM_E => BOTTOM_LEFT,
        m if m == ARM_N | ARM_W => BOTTOM_RIGHT,
        m if m == ARM_E | ARM_S => TOP_LEFT,
        m if m == ARM_S | ARM_W => TOP_RIGHT,
        m if m & (ARM_E | ARM_W) != 0 && m & (ARM_N | ARM_S) == 0 => HORIZONTAL,
        _ => VERTICAL,
    }
}

/// Record the perpendicular arms each seam contributes to its two endpoint
/// cells into `out` (not cleared; caller must clear first). A seam reserves
/// its own cell (see `split_rect`), so a perpendicular seam's nearest cell
/// sits one cell *past* this seam's endpoint. E.g. a vertical seam
/// starting at row `y` contributes a southward arm to the cell at `y - 1`,
/// which is where a horizontal seam ending there would actually be drawn.
pub(super) fn collect_seam_arms(seams: &[Seam], out: &mut FxHashMap<(u16, u16), u8>) {
    for seam in seams {
        match seam.direction {
            // `Horizontal` (width split) carves a vertical `│` line.
            Direction::Horizontal => {
                let x = seam.rect.x;
                if seam.rect.y > 0 {
                    *out.entry((x, seam.rect.y - 1)).or_insert(0) |= ARM_S;
                }
                *out.entry((x, seam.rect.y + seam.rect.height)).or_insert(0) |= ARM_N;
            }
            // `Vertical` (height split) carves a horizontal `─` line.
            Direction::Vertical => {
                let y = seam.rect.y;
                if seam.rect.x > 0 {
                    *out.entry((seam.rect.x - 1, y)).or_insert(0) |= ARM_E;
                }
                *out.entry((seam.rect.x + seam.rect.width, y)).or_insert(0) |= ARM_W;
            }
        }
    }
}

/// Split `area` into two child rects plus the seam divider drawn between
/// them, reserving that cell from the split axis so `first`, `seam`, and
/// `second` together (not `first`/`second` alone) tile `area` exactly.
///
/// `reserve_seam` mirrors the `pane-dividers` setting: `true` reserves a
/// 1-cell seam; `false` reserves none, so `first`/`second` tile `area`
/// edge-to-edge and `seam` is a zero-size rect.
///
/// All arithmetic is saturating so degenerate areas (zero width/height, or a
/// ratio that leaves no room for a seam) clamp to empty rects rather than
/// panicking. Callers must not invoke this on an area too small to hold a
/// seam plus two minimal panes: `:split`/`:vsplit` guard that before
/// mutating the layout tree (see `split_focused_pane` in the editor crate).
pub(super) fn split_rect(
    area: Rect,
    vertical: bool,
    ratio: f32,
    reserve_seam: bool,
) -> (Rect, Rect, Rect) {
    let seam_reserve: u16 = if reserve_seam { 1 } else { 0 };
    if vertical {
        let usable = area.height.saturating_sub(seam_reserve);
        let h1 = ((usable as f32 * ratio) as u16).min(usable);
        let seam_h = area.height.saturating_sub(h1).min(seam_reserve);
        let r1 = Rect { height: h1, ..area };
        let seam = Rect {
            y: area.y + h1,
            height: seam_h,
            ..area
        };
        let r2 = Rect {
            y: area.y + h1 + seam_h,
            height: area.height.saturating_sub(h1 + seam_h),
            ..area
        };
        (r1, seam, r2)
    } else {
        let usable = area.width.saturating_sub(seam_reserve);
        let w1 = ((usable as f32 * ratio) as u16).min(usable);
        let seam_w = area.width.saturating_sub(w1).min(seam_reserve);
        let r1 = Rect { width: w1, ..area };
        let seam = Rect {
            x: area.x + w1,
            width: seam_w,
            ..area
        };
        let r2 = Rect {
            x: area.x + w1 + seam_w,
            width: area.width.saturating_sub(w1 + seam_w),
            ..area
        };
        (r1, seam, r2)
    }
}

/// The slice of `seam` that should render with the focused-accent color when
/// `pane` is focused: the seam is immediately adjacent on the perpendicular
/// axis, and the returned rect is the overlap of their spans on the parallel
/// axis. `None` when not adjacent or the spans don't overlap. A seam shared by
/// several sibling panes (e.g. one pane stacked over two side-by-side panes)
/// lights up only above/beside the focused sibling, not the whole seam. Works
/// for both seam orientations without needing `Seam::direction`: a
/// width-split seam has `width == 1` (checked by the vertical-adjacency arm),
/// a height-split seam has `height == 1` (checked by the horizontal-adjacency
/// arm).
pub(super) fn focused_seam_segment(seam: Rect, pane: Rect) -> Option<Rect> {
    if seam.x == pane.right() || seam.right() == pane.x {
        let y0 = seam.y.max(pane.y);
        let y1 = seam.bottom().min(pane.bottom());
        if y0 < y1 {
            return Some(Rect {
                y: y0,
                height: y1 - y0,
                ..seam
            });
        }
    }
    if seam.y == pane.bottom() || seam.bottom() == pane.y {
        let x0 = seam.x.max(pane.x);
        let x1 = seam.right().min(pane.right());
        if x0 < x1 {
            return Some(Rect {
                x: x0,
                width: x1 - x0,
                ..seam
            });
        }
    }
    None
}

/// The four cells just outside the corners of `pane`, where the pane's
/// edge seams meet perpendicular seams from siblings, forming junctions.
/// `focused_seam_segment` misses these because a junction cell sits on the
/// perpendicular seam's column/row, which is the pane's boundary and thus
/// one cell outside the accent sub-rect (which only covers the pane's own
/// span on the parallel axis). A corner is `None` when it would sit off the
/// screen origin: the screen edge carries no seam, so there is no junction
/// there to color. Coordinates past the far screen edge are harmless: the
/// draw loop's buffer clamp never visits them.
pub(super) fn focused_pane_corners(pane: Rect) -> [Option<(u16, u16)>; 4] {
    let x0 = pane.x.checked_sub(1);
    let x1 = pane.x.checked_add(pane.width);
    let y0 = pane.y.checked_sub(1);
    let y1 = pane.y.checked_add(pane.height);
    [x0.zip(y0), x1.zip(y0), x0.zip(y1), x1.zip(y1)]
}
