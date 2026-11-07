use std::num::NonZeroU32;

use hume_grid::box_glyphs::{
    BOTTOM_LEFT, BOTTOM_RIGHT, CROSS, HORIZONTAL, HORIZONTAL_DOWN, HORIZONTAL_UP, TOP_LEFT,
    TOP_RIGHT, VERTICAL, VERTICAL_LEFT, VERTICAL_RIGHT,
};
use hume_grid::{Position, Rect};
use rustc_hash::FxHashMap;

use super::UnattachedPane;
use hume_core::PaneId;

// Layout tree

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

/// Minimum rows a pane keeps when a height split or a resize shrinks it.
const MIN_PANE_HEIGHT: u16 = 3;
/// Minimum columns a pane keeps when a width split or a resize shrinks it.
/// Wider than `MIN_PANE_HEIGHT` because text needs more horizontal room than
/// vertical to stay usable.
const MIN_PANE_WIDTH: u16 = 10;

impl Direction {
    /// The fewest cells a pane keeps along this axis when a split or a
    /// resize makes it smaller.
    fn min_pane_extent(self) -> u16 {
        match self {
            Direction::Horizontal => MIN_PANE_WIDTH,
            Direction::Vertical => MIN_PANE_HEIGHT,
        }
    }

    /// `area`'s start and extent along the axis a split in this direction
    /// divides: its columns for `Horizontal`, its rows for `Vertical`.
    fn span(self, area: Rect) -> (u16, u16) {
        match self {
            Direction::Horizontal => (area.x, area.width),
            Direction::Vertical => (area.y, area.height),
        }
    }

    /// `area` cut to `len` cells from `start` along that axis.
    fn slice(self, area: Rect, start: u16, len: u16) -> Rect {
        match self {
            Direction::Horizontal => Rect {
                x: start,
                width: len,
                ..area
            },
            Direction::Vertical => Rect {
                y: start,
                height: len,
                ..area
            },
        }
    }
}

/// Which way [`EngineView::rotate_focused`](super::EngineView::rotate_focused)
/// moves the members of a split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// Each member moves right or down; the last wraps to the first.
    Forward,
    /// Each member moves left or up; the first wraps to the last.
    Backward,
}

/// Whether [`EngineView::resize_focused`](super::EngineView::resize_focused)
/// makes the focused pane larger or smaller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resize {
    Grow,
    Shrink,
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

/// Refusal of [`EngineView::split_focused`](super::EngineView::split_focused):
/// the split would leave the new pane, or one it shrinks, below the minimum
/// size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitTooSmall;

/// Refusal of [`EngineView::resize_focused`](super::EngineView::resize_focused).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeRefused {
    /// No pane sits beside the focused one along the requested axis.
    NoNeighbor,
    /// The seam cannot move that way: a pane in the member it would shrink
    /// is at its minimum size.
    AtLimit,
}

/// Refusal of [`EngineView::move_seam`](super::EngineView::move_seam): the
/// handle names no seam of the active tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaleSeam;

/// A side of the focused pane, for
/// [`ScreenLayout::focused_neighbor`](super::ScreenLayout::focused_neighbor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// A single 1-cell divider drawn between two adjacent members of a split.
///
/// Emitted by `LayoutTree::collect_seams_into`, which walks the same member
/// geometry as `collect_rects_into`, so seam and leaf geometry can never
/// drift apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seam {
    pub rect: Rect,
    pub handle: SeamHandle,
}

impl Seam {
    /// The split that produced this seam. `Horizontal` (a width split)
    /// carves a 1-column-wide, full-height seam, drawn as a vertical line
    /// (`│`). `Vertical` (a height split) carves a 1-row-tall, full-width
    /// seam, drawn as a horizontal line (`─`). Same axis inversion as
    /// [`Split::direction`] (see `split_focused_pane`'s doc comment).
    pub fn direction(&self) -> Direction {
        self.handle.direction
    }
}

/// Names a seam by a pane on each side of it: the first leaf of the member
/// before it and of the member after it. The split it divides is the two
/// panes' nearest common ancestor, so the name survives a change elsewhere
/// in the tree. It resolves only while the two panes sit in adjacent
/// members of a split in `direction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeamHandle {
    before: PaneId,
    after: PaneId,
    direction: Direction,
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

/// The whole of the space a [`SplitShare`] divides, in its units of 2^-32.
const ONE: u64 = 1 << 32;

/// The position `fraction` of the way from `lo` to `hi`, rounded down.
fn between(lo: u64, hi: u64, fraction: u64) -> u64 {
    lo + (((hi - lo) * fraction) >> 32)
}

/// The part of the space left along a split, from a member's start to the
/// split's end, that the member takes, in units of 2^-32 of that space; the
/// members after it share the rest. A saved session keeps one per member
/// but the last, so replaying the splits puts each seam back where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitShare(NonZeroU32);

impl SplitShare {
    const HALF: Self = Self(NonZeroU32::new(1 << 31).expect("half is not zero"));

    pub fn from_bits(bits: NonZeroU32) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> NonZeroU32 {
        self.0
    }

    /// The share `fraction` of [`ONE`] names, or the nearest one inside
    /// `(0, ONE)`. Only a member with no cells to speak of asks for one
    /// outside it, such as one a hand-edited session file splits off.
    fn saturating(fraction: u64) -> Self {
        let bits = u32::try_from(fraction).unwrap_or(u32::MAX);
        Self(NonZeroU32::new(bits).unwrap_or(NonZeroU32::MIN))
    }

    /// The smallest share whose [`Self::of`] `space` is `cells`, which lie
    /// strictly inside it.
    fn taking(cells: u16, space: u16) -> Self {
        Self::saturating((u64::from(cells) << 32).div_ceil(u64::from(space)))
    }

    /// The share of the space from `lo` to [`ONE`] that ends at `at`.
    fn placing(lo: u64, at: u64) -> Self {
        Self::saturating(((at - lo) << 32).div_ceil(ONE - lo))
    }

    /// The cells of `space` this share takes, rounded down.
    fn of(self, space: u16) -> u16 {
        ((u64::from(space) * self.fraction()) >> 32) as u16
    }

    fn fraction(self) -> u64 {
        u64::from(self.0.get())
    }

    /// Where this share puts a seam in the space from `lo` to `hi`.
    fn within(self, lo: u64, hi: u64) -> u64 {
        between(lo, hi, self.fraction())
    }

    /// The share of two members side by side: this one's, then `next` of
    /// the space this one leaves.
    fn followed_by(self, next: SplitShare) -> Self {
        Self::saturating(next.within(self.fraction(), ONE))
    }
}

/// Where a split's seams sit.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seams {
    /// Spread evenly over however many members the split has: no seam of it
    /// has been moved by hand.
    Equal,
    /// Moved by hand: the share of each member but the last, which
    /// [`Split::set_seams`] checks against the member count.
    Placed(Vec<SplitShare>),
}

impl Seams {
    /// The share of each of `n` members but the last: placed seams' own, or
    /// the ones that lay an equal split's cells out where they are.
    fn shares(&self, n: usize) -> Vec<SplitShare> {
        match self {
            Seams::Equal => (0..n - 1)
                .map(|k| SplitShare::saturating(ONE.div_ceil((n - k) as u64)))
                .collect(),
            Seams::Placed(shares) => shares.clone(),
        }
    }

    /// The first cell of each of `n` members in a `usable`-cell extent and
    /// the cell past its end. Each member takes its share of the cells left
    /// from where it starts, rounded down, and the last takes the rest, so
    /// no member loses a cell when the extent grows. An equal split's
    /// leftover cells go to its last members.
    fn cells(&self, n: usize, usable: u16) -> impl Iterator<Item = (u16, u16)> + '_ {
        (0..n).scan(0, move |start: &mut u16, k| {
            let left = usable - *start;
            let len = match self {
                _ if k + 1 == n => left,
                Seams::Equal => left / u16::try_from(n - k).unwrap_or(u16::MAX),
                Seams::Placed(shares) => shares[k].of(left),
            };
            let first = *start;
            *start += len;
            Some((first, *start))
        })
    }

    /// These seams of `n` members once a new member goes after member `k`,
    /// in its space. Equal seams stay equal unless `share` is given;
    /// otherwise member `k` keeps `share` of that space, or half. A lone
    /// pane is one member with equal seams.
    fn with_member_after(&self, k: usize, n: usize, share: Option<SplitShare>) -> Seams {
        if share.is_none() && *self == Seams::Equal {
            return Seams::Equal;
        }
        let share = share.unwrap_or(SplitShare::HALF);
        let mut shares = self.shares(n);
        if k + 1 == n {
            shares.push(share);
        } else {
            let own = shares[k];
            let kept = SplitShare::saturating((own.fraction() * share.fraction()).div_ceil(ONE));
            shares[k] = kept;
            shares.insert(k + 1, SplitShare::placing(kept.fraction(), own.fraction()));
        }
        Seams::Placed(shares)
    }
}

/// The cells of `extent` left to `n` members once the seams between them
/// take theirs.
fn usable(extent: u16, n: usize, reserve_seam: bool) -> u16 {
    if reserve_seam {
        extent.saturating_sub(u16::try_from(n - 1).unwrap_or(u16::MAX))
    } else {
        extent
    }
}

/// A group of panes laid out along `direction`, with a seam between each
/// two members. There are at least two members, and none is a split in the
/// same direction, whose members would belong to this split.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    /// `Horizontal` divides the width and `Vertical` the height: the axis
    /// names say which way the divider between two members runs.
    direction: Direction,
    members: Vec<LayoutTree>,
    seams: Seams,
}

impl Split {
    pub fn direction(&self) -> Direction {
        self.direction
    }

    pub fn members(&self) -> &[LayoutTree] {
        &self.members
    }

    /// For each member after the first, in order: the share to split the
    /// member before it with, while that one is the last, to grow this
    /// split from its first member. `None` while the seams are equal, since
    /// a plain split keeps them equal.
    pub fn rebuild_shares(&self) -> impl Iterator<Item = Option<SplitShare>> + '_ {
        (1..self.members.len()).map(|k| match &self.seams {
            Seams::Equal => None,
            Seams::Placed(shares) => Some(shares[k - 1]),
        })
    }

    /// Set the seams, which must suit the members the split has now.
    fn set_seams(&mut self, seams: Seams) {
        if let Seams::Placed(shares) = &seams {
            assert_eq!(
                shares.len() + 1,
                self.members.len(),
                "one share for each member but the last"
            );
        }
        self.seams = seams;
    }

    /// The index of the member that is `Leaf(pid)`.
    fn leaf_member(&self, pid: PaneId) -> Option<usize> {
        self.members
            .iter()
            .position(|m| *m == LayoutTree::Leaf(pid))
    }

    /// Each member's rect within `area`, paired with the seam after it (see
    /// [`Self::lay_out`]).
    fn member_rects(&self, area: Rect, reserve_seam: bool) -> impl Iterator<Item = (Rect, Rect)> {
        Self::lay_out(
            self.direction,
            area,
            reserve_seam,
            self.members.len(),
            &self.seams,
        )
    }

    /// The rects of `n` members along `direction` within `area`, placed by
    /// `seams` (see [`Seams::cells`]), each paired with the seam after it:
    /// a 1-cell rect when `reserve_seam` is on, empty after the last member
    /// or with it off. Every bound saturates, so an area too small for the
    /// seams yields empty rects rather than a panic.
    fn lay_out(
        direction: Direction,
        area: Rect,
        reserve_seam: bool,
        n: usize,
        seams: &Seams,
    ) -> impl Iterator<Item = (Rect, Rect)> {
        let (start, extent) = direction.span(area);
        let end = start.saturating_add(extent);
        let usable = usable(extent, n, reserve_seam);
        seams
            .cells(n, usable)
            .enumerate()
            .map(move |(k, (lo, hi))| {
                let seams_before = if reserve_seam { k as u16 } else { 0 };
                let pos = start
                    .saturating_add(lo)
                    .saturating_add(seams_before)
                    .min(end);
                let len = (hi - lo).min(end - pos);
                let seam_pos = pos + len;
                let seam_len = if reserve_seam && k + 1 < n {
                    (end - seam_pos).min(1)
                } else {
                    0
                };
                (
                    direction.slice(area, pos, len),
                    direction.slice(area, seam_pos, seam_len),
                )
            })
    }

    /// Whether `members`, laid out along `direction` within `area` by
    /// `seams`, take a new member after member `k`, placed as
    /// [`Seams::with_member_after`] places it, with every pane keeping its
    /// minimum size (see [`LayoutTree::keeps_minimums`]) and the new one at
    /// least that.
    fn fits_member_after(
        direction: Direction,
        members: &[LayoutTree],
        seams: &Seams,
        k: usize,
        share: Option<SplitShare>,
        area: Rect,
        reserve_seam: bool,
    ) -> bool {
        let n = members.len();
        let grown = seams.with_member_after(k, n, share);
        let mut old = Self::lay_out(direction, area, reserve_seam, n, seams);
        let mut members = members.iter();
        Self::lay_out(direction, area, reserve_seam, n + 1, &grown)
            .enumerate()
            .all(|(j, (after, _))| {
                if j == k + 1 {
                    return direction.span(after).1 >= direction.min_pane_extent();
                }
                let member = members.next().expect("a member for each old place");
                let (before, _) = old.next().expect("an old place for each member");
                member.keeps_minimums(direction, before, after, reserve_seam)
            })
    }

    /// The index and rect of the member holding `pid`.
    fn member_holding(&self, pid: PaneId, area: Rect, reserve_seam: bool) -> Option<(usize, Rect)> {
        self.members
            .iter()
            .zip(self.member_rects(area, reserve_seam))
            .enumerate()
            .find(|(_, (member, _))| member.contains_leaf(pid))
            .map(|(k, (_, (rect, _)))| (k, rect))
    }

    /// Insert `member` after member `k`, in member `k`'s space. A placed
    /// split leaves member `k` the part of that space `share` gives, or
    /// half; an equal one stays equal unless `share` is given.
    fn insert_after(&mut self, k: usize, member: LayoutTree, share: Option<SplitShare>) {
        let seams = self.seams.with_member_after(k, self.members.len(), share);
        self.members.insert(k + 1, member);
        self.set_seams(seams);
    }

    /// Remove member `k`. A placed split gives its space to the member
    /// before it, or to the one after it when it is first; an equal one
    /// stays equal. Returns where that neighbour ends up.
    fn remove(&mut self, k: usize) -> usize {
        let neighbour = k.saturating_sub(1);
        let seams = match &self.seams {
            Seams::Equal => Seams::Equal,
            Seams::Placed(shares) => {
                let mut shares = shares.clone();
                if neighbour + 2 == self.members.len() {
                    shares.remove(neighbour);
                } else {
                    let next = shares.remove(neighbour + 1);
                    shares[neighbour] = shares[neighbour].followed_by(next);
                }
                Seams::Placed(shares)
            }
        };
        self.members.remove(k);
        self.set_seams(seams);
        neighbour
    }

    /// Take `inner`, a split in this direction standing in member `k`'s
    /// space, apart into its members, keeping every seam where it sits. Two
    /// equal splits make an equal one.
    fn absorb(&mut self, k: usize, inner: Split) {
        let seams = match (&self.seams, &inner.seams) {
            (Seams::Equal, Seams::Equal) => Seams::Equal,
            _ => {
                let n = self.members.len();
                let last = k + 1 == n;
                let mut shares = self.seams.shares(n);
                // Member `k`'s space, as a part of the space left from where
                // it starts.
                let room = if last { ONE } else { shares[k].fraction() };
                let starts: Vec<u64> = std::iter::once(0)
                    .chain(inner.seams.shares(inner.members.len()).into_iter().scan(
                        0,
                        |at: &mut u64, share| {
                            *at = share.within(*at, ONE);
                            Some(*at)
                        },
                    ))
                    .map(|at| between(0, room, at))
                    .chain((!last).then_some(room))
                    .collect();
                let inner_shares = starts
                    .windows(2)
                    .map(|pair| SplitShare::placing(pair[0], pair[1]));
                shares.splice(k..k + usize::from(!last), inner_shares);
                Seams::Placed(shares)
            }
        };
        self.members.splice(k..=k, inner.members);
        self.set_seams(seams);
    }

    /// Move seam `k` one cell at a time toward the screen cell along the
    /// split axis that `target` gives for the cell it is drawn on, stopping
    /// before a step that would leave a pane of the member it shrinks below
    /// its minimum size (see [`LayoutTree::keeps_minimums`]). The member it
    /// grows only gains cells (see [`Seams::cells`]), and no other member
    /// moves. `false` when the seam stays where it was.
    fn place_seam(
        &mut self,
        k: usize,
        area: Rect,
        reserve_seam: bool,
        target: impl FnOnce(i64) -> i64,
    ) -> bool {
        let n = self.members.len();
        let (start, extent) = self.direction.span(area);
        let usable = usable(extent, n, reserve_seam);
        let (lo, current, hi) = {
            let mut cells = self.seams.cells(n, usable).skip(k);
            let (lo, current) = cells.next().expect("member k");
            let (_, hi) = cells.next().expect("member k + 1");
            (lo, current, hi)
        };
        // The screen cell the seam is drawn on when it ends member `k` at
        // cell 0 of the usable extent.
        let offset = i64::from(start) + if reserve_seam { k as i64 } else { 0 };
        let to = target(offset + i64::from(current)) - offset;
        let shrinking = if to > i64::from(current) { k + 1 } else { k };
        let shares = self.seams.shares(n);
        let with_seam_at = |cell: u16| {
            let mut shares = shares.clone();
            shares[k] = SplitShare::taking(cell - lo, usable - lo);
            if k + 2 < n {
                shares[k + 1] = SplitShare::taking(hi - cell, usable - cell);
            }
            Seams::Placed(shares)
        };
        let shrunk = |seams: &Seams| {
            let (rect, _) = Self::lay_out(self.direction, area, reserve_seam, n, seams)
                .nth(shrinking)
                .expect("the member the seam shrinks");
            rect
        };
        let was = shrunk(&self.seams);
        let mut placed = current;
        while i64::from(placed) != to {
            // A step that empties member `k` or `k + 1` leaves a pane in it
            // below its minimum, and no share places it.
            let next = if to > i64::from(placed) {
                placed.checked_add(1)
            } else {
                placed.checked_sub(1)
            }
            .filter(|&cell| lo < cell && cell < hi);
            let Some(next) = next else { break };
            let now = shrunk(&with_seam_at(next));
            if !self.members[shrinking].keeps_minimums(self.direction, was, now, reserve_seam) {
                break;
            }
            placed = next;
        }
        if placed == current {
            return false;
        }
        self.set_seams(with_seam_at(placed));
        true
    }
}

/// Where a new pane goes, as [`LayoutTree::split_site`] finds it: the
/// member taken at each split from the root down to the subtree it joins,
/// then the member of that subtree's split it goes after, or `None` when
/// the subtree is the leaf it pairs with.
struct SplitSite {
    path: Vec<usize>,
    after: Option<usize>,
    direction: Direction,
}

/// Recursive layout tree. Leaves reference panes; splits partition space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutTree {
    Leaf(PaneId),
    Split(Split),
}

impl LayoutTree {
    /// `members` side by side along `direction`, with equal seams.
    #[cfg(test)]
    pub(crate) fn split(direction: Direction, members: Vec<LayoutTree>) -> Self {
        assert!(members.len() >= 2, "a split has two members or more");
        assert!(
            !members
                .iter()
                .any(|m| matches!(m, LayoutTree::Split(s) if s.direction == direction)),
            "a split in the same direction belongs to its parent"
        );
        LayoutTree::Split(Split {
            direction,
            members,
            seams: Seams::Equal,
        })
    }

    /// Compute (PaneId, Rect) pairs for the leaf panes given the total area.
    /// Results are appended to `out` (which is not cleared; caller must clear first).
    ///
    /// `reserve_seam` mirrors the `pane-dividers` setting: `true` carves a
    /// 1-cell seam between each two members of a split; `false` tiles them
    /// edge-to-edge with no reserved gap.
    pub(crate) fn collect_rects_into(
        &self,
        area: Rect,
        reserve_seam: bool,
        out: &mut Vec<(PaneId, Rect)>,
    ) {
        match self {
            LayoutTree::Leaf(id) => out.push((*id, area)),
            LayoutTree::Split(split) => {
                for (member, (rect, _)) in split
                    .members
                    .iter()
                    .zip(split.member_rects(area, reserve_seam))
                {
                    member.collect_rects_into(rect, reserve_seam, out);
                }
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
            LayoutTree::Split(split) => {
                let (k, rect) = split.member_holding(pid, area, reserve_seam)?;
                split.members[k].find_rect(pid, rect, reserve_seam)
            }
        }
    }

    /// Compute the 1-cell seam divider drawn between each two members of
    /// every split, given the total area. Walks the same member geometry as
    /// `collect_rects_into`, so seams always align with the leaf rects
    /// computed for the same `area` this frame. Results are appended to
    /// `out` (not cleared; caller must clear first).
    ///
    /// Always reserves the seam: callers only invoke this when dividers are
    /// being drawn, which is also when `collect_rects_into` is called with
    /// `reserve_seam: true`, so the two stay aligned.
    pub(crate) fn collect_seams_into(&self, area: Rect, out: &mut Vec<Seam>) {
        let LayoutTree::Split(split) = self else {
            return;
        };
        for (k, (rect, seam)) in split.member_rects(area, true).enumerate() {
            if seam.width > 0 && seam.height > 0 {
                out.push(Seam {
                    rect: seam,
                    handle: SeamHandle {
                        before: split.members[k].first_leaf(),
                        after: split.members[k + 1].first_leaf(),
                        direction: split.direction,
                    },
                });
            }
            split.members[k].collect_seams_into(rect, out);
        }
    }

    /// Where a new pane after `Leaf(target)` along `direction` goes: into
    /// `target`'s own split when that runs the same way, otherwise into a
    /// new split of the two in `target`'s place. `None` when `target` is not
    /// a leaf below `self`.
    fn split_site(&self, target: PaneId, direction: Direction) -> Option<SplitSite> {
        match self {
            LayoutTree::Leaf(id) => (*id == target).then(|| SplitSite {
                path: Vec::new(),
                after: None,
                direction,
            }),
            LayoutTree::Split(split) => {
                if split.direction == direction
                    && let Some(k) = split.leaf_member(target)
                {
                    return Some(SplitSite {
                        path: Vec::new(),
                        after: Some(k),
                        direction,
                    });
                }
                let k = split.members.iter().position(|m| m.contains_leaf(target))?;
                let mut site = split.members[k].split_site(target, direction)?;
                site.path.insert(0, k);
                Some(site)
            }
        }
    }

    /// Put `new_pane` at `site`, a site of `self`.
    fn insert_split(&mut self, site: &SplitSite, new_pane: PaneId, share: Option<SplitShare>) {
        let subtree = site.path.iter().fold(self, |node, &k| match node {
            LayoutTree::Split(split) => &mut split.members[k],
            LayoutTree::Leaf(_) => unreachable!("a split site's path runs through splits"),
        });
        let new_pane = LayoutTree::Leaf(new_pane);
        match (site.after, subtree) {
            (Some(k), LayoutTree::Split(split)) => split.insert_after(k, new_pane, share),
            (None, subtree) => {
                // Overwritten below by the pair.
                let leaf = std::mem::replace(subtree, LayoutTree::Leaf(PaneId::default()));
                let mut pair = Split {
                    direction: site.direction,
                    members: vec![leaf, new_pane],
                    seams: Seams::Equal,
                };
                pair.set_seams(Seams::Equal.with_member_after(0, 1, share));
                *subtree = LayoutTree::Split(pair);
            }
            (Some(_), LayoutTree::Leaf(_)) => unreachable!("a site after a member is a split"),
        }
    }

    /// Whether a new pane at `site`, a site of `self`, within `area` keeps
    /// every pane at its minimum size, the new one included (see
    /// [`Split::fits_member_after`]).
    fn split_fits(
        &self,
        site: &SplitSite,
        share: Option<SplitShare>,
        area: Rect,
        reserve_seam: bool,
    ) -> bool {
        let (subtree, area) = site.path.iter().fold((self, area), |(node, area), &k| {
            let LayoutTree::Split(split) = node else {
                unreachable!("a split site's path runs through splits");
            };
            let (rect, _) = split
                .member_rects(area, reserve_seam)
                .nth(k)
                .expect("a member on the path");
            (&split.members[k], rect)
        });
        let (members, seams, k) = match (site.after, subtree) {
            (Some(k), LayoutTree::Split(split)) => (split.members.as_slice(), &split.seams, k),
            (None, leaf) => (std::slice::from_ref(leaf), &Seams::Equal, 0),
            (Some(_), LayoutTree::Leaf(_)) => unreachable!("a site after a member is a split"),
        };
        Split::fits_member_after(site.direction, members, seams, k, share, area, reserve_seam)
    }

    /// Whether this subtree, laid out in `after` instead of `before`, leaves
    /// every pane in it at least the minimum size along `axis`, or no
    /// smaller there than it was. A placed split scales its seams with its
    /// rect, so a pane deep in it can reach its minimum before the subtree
    /// as a whole would.
    fn keeps_minimums(
        &self,
        axis: Direction,
        before: Rect,
        after: Rect,
        reserve_seam: bool,
    ) -> bool {
        match self {
            LayoutTree::Leaf(_) => {
                let (was, now) = (axis.span(before).1, axis.span(after).1);
                now >= was || now >= axis.min_pane_extent()
            }
            LayoutTree::Split(split) => split
                .members
                .iter()
                .zip(split.member_rects(before, reserve_seam))
                .zip(split.member_rects(after, reserve_seam))
                .all(|((member, (before, _)), (after, _))| {
                    member.keeps_minimums(axis, before, after, reserve_seam)
                }),
        }
    }

    /// Whether `target` is reachable as a leaf in this subtree.
    pub fn contains_leaf(&self, target: PaneId) -> bool {
        match self {
            LayoutTree::Leaf(id) => *id == target,
            LayoutTree::Split(split) => split.members.iter().any(|m| m.contains_leaf(target)),
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
            LayoutTree::Split(split) => {
                for member in split.members {
                    member.collect_detached(out);
                }
            }
        }
    }

    /// Leftmost leaf, found by descending into the first member of each
    /// split.
    fn first_leaf(&self) -> PaneId {
        match self {
            LayoutTree::Leaf(id) => *id,
            LayoutTree::Split(split) => split.members[0].first_leaf(),
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
            LayoutTree::Split(split) => {
                for member in &split.members {
                    member.collect_leaves(out);
                }
            }
        }
    }

    /// Set every split's seams equal.
    fn equalize(&mut self) {
        if let LayoutTree::Split(split) = self {
            split.seams = Seams::Equal;
            for member in &mut split.members {
                member.equalize();
            }
        }
    }

    /// The recursive body of [`FocusedLayout::rotate_focused`]: `false`
    /// when `target` is not a leaf below `self`.
    fn rotate_run(&mut self, target: PaneId, rotation: Rotation, steps: usize) -> bool {
        let LayoutTree::Split(split) = self else {
            return false;
        };
        if split.leaf_member(target).is_none() {
            return split
                .members
                .iter_mut()
                .any(|m| m.rotate_run(target, rotation, steps));
        }
        let shift = steps % split.members.len();
        match rotation {
            Rotation::Forward => split.members.rotate_right(shift),
            Rotation::Backward => split.members.rotate_left(shift),
        }
        true
    }

    /// Prune `Leaf(target)` from its split, which collapses onto its last
    /// member when only one is left, and return the leftmost leaf of the
    /// neighbour that takes the pruned space (see [`Split::remove`]). A
    /// collapse that leaves a split standing in a split of its own
    /// direction is taken apart into it. `None` when `target` is not a leaf
    /// below a split of `self`.
    fn prune_leaf(&mut self, target: PaneId) -> Option<PaneId> {
        let LayoutTree::Split(split) = self else {
            return None;
        };
        if let Some(k) = split.leaf_member(target) {
            let neighbour = split.remove(k);
            let survivor = split.members[neighbour].first_leaf();
            if let [only] = split.members.as_mut_slice() {
                // The placeholder is dropped with the split it was left in.
                *self = std::mem::replace(only, LayoutTree::Leaf(PaneId::default()));
            }
            return Some(survivor);
        }
        let (k, survivor) = split
            .members
            .iter_mut()
            .enumerate()
            .find_map(|(k, m)| m.prune_leaf(target).map(|survivor| (k, survivor)))?;
        // Overwritten below, either by the member itself or by its members.
        match std::mem::replace(&mut split.members[k], LayoutTree::Leaf(PaneId::default())) {
            LayoutTree::Split(inner) if inner.direction == split.direction => {
                split.absorb(k, inner)
            }
            member => split.members[k] = member,
        }
        Some(survivor)
    }

    /// The recursive body of [`FocusedLayout::resize_focused`]: move the
    /// seam beside the member holding `target` in the innermost split along
    /// `axis` that holds it, by `cells` toward the member after it (or
    /// before it, for the last member). `None` when no split along `axis`
    /// holds `target`, otherwise whether the seam moved.
    fn resize_along(
        &mut self,
        target: PaneId,
        area: Rect,
        reserve_seam: bool,
        axis: Direction,
        cells: i64,
    ) -> Option<bool> {
        let LayoutTree::Split(split) = self else {
            return None;
        };
        let (k, rect) = split.member_holding(target, area, reserve_seam)?;
        if let Some(moved) = split.members[k].resize_along(target, rect, reserve_seam, axis, cells)
        {
            return Some(moved);
        }
        (split.direction == axis).then(|| {
            let (seam, by) = if k + 1 < split.members.len() {
                (k, cells)
            } else {
                (k - 1, -cells)
            };
            split.place_seam(seam, area, reserve_seam, |current| current + by)
        })
    }

    /// The recursive body of [`FocusedLayout::move_seam`].
    fn move_seam(
        &mut self,
        handle: SeamHandle,
        area: Rect,
        reserve_seam: bool,
        cell: u16,
    ) -> Result<(), StaleSeam> {
        let LayoutTree::Split(split) = self else {
            return Err(StaleSeam);
        };
        let (k, rect) = split
            .member_holding(handle.before, area, reserve_seam)
            .ok_or(StaleSeam)?;
        if split.members[k].contains_leaf(handle.after) {
            return split.members[k].move_seam(handle, rect, reserve_seam, cell);
        }
        let adjacent = split
            .members
            .get(k + 1)
            .is_some_and(|m| m.contains_leaf(handle.after));
        if !adjacent || split.direction != handle.direction {
            return Err(StaleSeam);
        }
        split.place_seam(k, area, reserve_seam, |_| i64::from(cell));
        Ok(())
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

    /// Whether splitting the focused pane on `direction` within `area`, with
    /// the seam where `share` puts it, would leave the new pane, and every
    /// pane it shrinks, at least the minimum size along that axis. In an
    /// equal split a new member shrinks every member, not only the focused
    /// one. A pane the split leaves alone is not checked, even when it is
    /// already smaller than the minimum.
    pub(super) fn split_fits(
        &self,
        area: Rect,
        reserve_seam: bool,
        direction: Direction,
        share: Option<SplitShare>,
    ) -> bool {
        self.tree
            .split_fits(&self.split_site(direction), share, area, reserve_seam)
    }

    /// Put `pane` after the focused pane along `direction`, without checking
    /// that it fits: [`EngineView::split_focused`](super::EngineView::split_focused)
    /// checks with [`Self::split_fits`] first. An equal split stays equal and
    /// a placed one halves the focused pane's space, unless `share` says
    /// where the new seam goes (see [`Split::insert_after`]). Focus stays
    /// where it was.
    pub(super) fn splice_beside_focused(
        &mut self,
        pane: UnattachedPane,
        direction: Direction,
        share: Option<SplitShare>,
    ) -> PaneId {
        let new_id = pane.pane_id();
        let site = self.split_site(direction);
        self.tree.insert_split(&site, new_id, share);
        new_id
    }

    /// Where a new pane after the focused one along `direction` goes (see
    /// [`LayoutTree::split_site`]).
    fn split_site(&self, direction: Direction) -> SplitSite {
        self.tree
            .split_site(self.focus, direction)
            .expect("the focused pane is a leaf of the tree")
    }

    /// Rotate the split holding the focused leaf as a direct member by
    /// `steps` places. A split on the other axis inside it is one member
    /// and moves whole. The seams stay, so each place keeps its size and
    /// only its contents move. Focus stays on the same pane.
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

    /// Prune the focused leaf. Its split stays equal if it was, otherwise its
    /// space goes to the member before it, or after it when it is first.
    /// Focus moves to the leftmost leaf of that neighbour.
    pub(crate) fn close_focused(&mut self) -> Result<DetachedPane, SinglePane> {
        self.refuse_single_pane()?;
        let survivor = self
            .tree
            .prune_leaf(self.focus)
            .expect("the focused pane is a leaf of the tree");
        Ok(DetachedPane(std::mem::replace(&mut self.focus, survivor)))
    }

    pub(crate) fn refuse_single_pane(&self) -> Result<(), SinglePane> {
        match self.tree {
            LayoutTree::Leaf(_) => Err(SinglePane),
            LayoutTree::Split(_) => Ok(()),
        }
    }

    /// Grow or shrink the focused pane by `cells` along `axis` within
    /// `area`, moving the seam after its member of the innermost split on
    /// `axis` that holds it, or the seam before it when it is the last
    /// member. Only the two members beside that seam change size, and only
    /// as far as every pane in them keeps its minimum size; that split's
    /// seams are placed from then on.
    pub(crate) fn resize_focused(
        &mut self,
        area: Rect,
        reserve_seam: bool,
        axis: Direction,
        resize: Resize,
        cells: u16,
    ) -> Result<(), ResizeRefused> {
        let by = match resize {
            Resize::Grow => i64::from(cells),
            Resize::Shrink => -i64::from(cells),
        };
        match self
            .tree
            .resize_along(self.focus, area, reserve_seam, axis, by)
        {
            None => Err(ResizeRefused::NoNeighbor),
            Some(false) => Err(ResizeRefused::AtLimit),
            Some(true) => Ok(()),
        }
    }

    /// Move the seam `handle` names so it lands on `to`'s cell along its
    /// axis within `area`, as far as the members beside it allow (see
    /// [`Self::resize_focused`]).
    pub(crate) fn move_seam(
        &mut self,
        handle: SeamHandle,
        area: Rect,
        reserve_seam: bool,
        to: Position,
    ) -> Result<(), StaleSeam> {
        let cell = match handle.direction {
            Direction::Horizontal => to.x,
            Direction::Vertical => to.y,
        };
        self.tree.move_seam(handle, area, reserve_seam, cell)
    }

    /// Set every seam of the tree equal.
    pub(crate) fn equalize(&mut self) {
        self.tree.equalize();
    }
}

// Seam junction glyphs
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
/// its own cell (see `Split::member_rects`), so a perpendicular seam's
/// nearest cell sits one cell *past* this seam's endpoint. E.g. a vertical seam
/// starting at row `y` contributes a southward arm to the cell at `y - 1`,
/// which is where a horizontal seam ending there would actually be drawn.
pub(super) fn collect_seam_arms(seams: &[Seam], out: &mut FxHashMap<(u16, u16), u8>) {
    for seam in seams {
        match seam.direction() {
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

#[cfg(test)]
mod tests;
