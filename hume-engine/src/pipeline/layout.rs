use hume_grid::{Position, Rect};
use rustc_hash::FxHashMap;

use super::PaneId;

// ---------------------------------------------------------------------------
// Layout tree
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
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
    /// carves a 1-column-wide, full-height seam — drawn as a vertical line
    /// (`│`). `Vertical` (a height split) carves a 1-row-tall, full-width
    /// seam — drawn as a horizontal line (`─`). Same axis inversion as
    /// `LayoutTree::Split` (see `split_focused_pane`'s doc comment).
    pub direction: Direction,
}

/// Recursive layout tree. Leaves reference panes; splits partition space.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutTree {
    Leaf(PaneId),
    Split {
        direction: Direction,
        /// Fraction (0.0–1.0) allocated to the first child.
        ratio: f32,
        children: Box<(LayoutTree, LayoutTree)>,
    },
}

impl LayoutTree {
    /// Compute (PaneId, Rect) pairs for the leaf panes given the total area.
    /// Results are appended to `out` (which is not cleared — caller must clear first).
    ///
    /// `reserve_seam` mirrors the `pane-dividers` setting: `true` carves a
    /// 1-cell seam out of each split axis (see `split_rect`); `false` tiles
    /// children edge-to-edge with no reserved gap.
    pub fn collect_rects_into(
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

    /// `pid`'s own rect, without collecting every other leaf's — the single-
    /// target sibling of [`Self::collect_rects_into`], for callers (a single
    /// pane lookup, a mouse-motion hit test) that don't need the whole
    /// partition and would otherwise allocate one just to search it.
    pub fn find_rect(&self, pid: PaneId, area: Rect, reserve_seam: bool) -> Option<Rect> {
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

    /// The leaf pane whose rect contains `pos`, and that rect — the
    /// position-search sibling of [`Self::find_rect`], for a mouse hit test
    /// that would otherwise collect every leaf's rect just to scan it for
    /// containment. Descends only the child whose rect contains `pos`
    /// (`O(depth)`, not `O(panes)`), falling through to the other child only
    /// when neither matches at a given split (a click in the seam gap),
    /// where the eventual `None` is still correct, just found less directly.
    pub fn find_containing(
        &self,
        pos: Position,
        area: Rect,
        reserve_seam: bool,
    ) -> Option<(PaneId, Rect)> {
        match self {
            LayoutTree::Leaf(id) => area.contains(pos).then_some((*id, area)),
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
                if r1.contains(pos) {
                    children.0.find_containing(pos, r1, reserve_seam)
                } else {
                    children.1.find_containing(pos, r2, reserve_seam)
                }
            }
        }
    }

    /// Compute the 1-cell seam divider drawn between sibling panes at every
    /// split node, given the total area. Walks the same recursion as
    /// `collect_rects_into`, off the same `split_rect` math, so seams always
    /// align with the leaf rects computed for the same `area` this frame.
    /// Results are appended to `out` (not cleared — caller must clear first).
    ///
    /// Always reserves the seam: callers only invoke this when dividers are
    /// being drawn, which is also when `collect_rects_into` is called with
    /// `reserve_seam: true` — the two stay aligned by construction.
    pub fn collect_seams_into(&self, area: Rect, out: &mut Vec<Seam>) {
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

    /// Replace `Leaf(target)` with a `Split` of `(Leaf(target), Leaf(new_pane))`.
    /// Returns whether `target` was found.
    pub fn split_leaf(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        direction: Direction,
        ratio: f32,
    ) -> bool {
        match self {
            LayoutTree::Leaf(id) if *id == target => {
                *self = LayoutTree::Split {
                    direction,
                    ratio,
                    children: Box::new((LayoutTree::Leaf(target), LayoutTree::Leaf(new_pane))),
                };
                true
            }
            LayoutTree::Leaf(_) => false,
            LayoutTree::Split { children, .. } => {
                children.0.split_leaf(target, new_pane, direction, ratio)
                    || children.1.split_leaf(target, new_pane, direction, ratio)
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

    /// Prune `Leaf(target)`, collapsing its parent `Split` onto the sibling.
    /// Returns the leftmost leaf of the promoted sibling (the new focus target),
    /// or `None` if `self` is the sole leaf.
    pub fn remove_leaf(&mut self, target: PaneId) -> Option<PaneId> {
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
                        .remove_leaf(target)
                        .or_else(|| children.1.remove_leaf(target))
                }
            }
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
// drawn — a child split's seam starts or ends partway along the parent seam,
// so its endpoint-adjacent record lands on an interior cell of the parent
// (see `collect_seam_arms_t_junction`: a `│` seam starting one row below a
// `─` seam records `ARM_S` on the `─` seam's interior). The draw loop
// therefore probes every cell of each seam's rect, OR-ing the seam's `base`
// mask with any arms recorded for that cell. This stays per-seam, not a
// full-frame scan, and the arms map is sparse (at most two entries per seam)
// so the lookups hit a tiny map — the per-cell buffer writes dominate.

/// Light box-drawing glyphs for the pane dividers. Written as escapes
/// rather than literals so a grep for one finds every use, and so an editor
/// or terminal that renders them ambiguously can't quietly swap one for
/// another during an edit.
pub(super) const HORIZONTAL: &str = "\u{2500}";
pub(super) const VERTICAL: &str = "\u{2502}";

pub(super) const ARM_N: u8 = 0b0001;
pub(super) const ARM_E: u8 = 0b0010;
pub(super) const ARM_S: u8 = 0b0100;
pub(super) const ARM_W: u8 = 0b1000;

/// Resolve a compass-bit mask (`ARM_N | ARM_E | ...`) to the box-drawing
/// glyph with exactly those arms. Masks with fewer than two bits, or with
/// only two opposite bits, fall back to a straight line — this keeps the
/// function a total resolver even though seam geometry only ever produces
/// `│ ─ ├ ┤ ┬ ┴ ┼`.
pub(super) fn junction_glyph(mask: u8) -> &'static str {
    match mask {
        m if m == ARM_N | ARM_E | ARM_S | ARM_W => "\u{253c}",
        m if m == ARM_E | ARM_S | ARM_W => "\u{252c}",
        m if m == ARM_N | ARM_E | ARM_W => "\u{2534}",
        m if m == ARM_N | ARM_E | ARM_S => "\u{251c}",
        m if m == ARM_N | ARM_S | ARM_W => "\u{2524}",
        m if m == ARM_N | ARM_E => "\u{2514}",
        m if m == ARM_N | ARM_W => "\u{2518}",
        m if m == ARM_E | ARM_S => "\u{250c}",
        m if m == ARM_S | ARM_W => "\u{2510}",
        m if m & (ARM_E | ARM_W) != 0 && m & (ARM_N | ARM_S) == 0 => HORIZONTAL,
        _ => VERTICAL,
    }
}

/// Record the perpendicular arms each seam contributes to its two endpoint
/// cells into `out` (not cleared — caller must clear first). A seam reserves
/// its own cell (see `split_rect`), so a perpendicular seam's nearest cell
/// sits one cell *past* this seam's endpoint — e.g. a vertical seam
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
/// `second` together — not `first`/`second` alone — tile `area` exactly.
///
/// `reserve_seam` mirrors the `pane-dividers` setting: `true` reserves a
/// 1-cell seam; `false` reserves none, so `first`/`second` tile `area`
/// edge-to-edge and `seam` is a zero-size rect.
///
/// All arithmetic is saturating so degenerate areas (zero width/height, or a
/// ratio that leaves no room for a seam) clamp to empty rects rather than
/// panicking. Callers must not invoke this on an area too small to hold a
/// seam plus two minimal panes — `:split`/`:vsplit` guard that before
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
    if seam.x == pane.x + pane.width || seam.x + seam.width == pane.x {
        let y0 = seam.y.max(pane.y);
        let y1 = (seam.y + seam.height).min(pane.y + pane.height);
        if y0 < y1 {
            return Some(Rect {
                y: y0,
                height: y1 - y0,
                ..seam
            });
        }
    }
    if seam.y == pane.y + pane.height || seam.y + seam.height == pane.y {
        let x0 = seam.x.max(pane.x);
        let x1 = (seam.x + seam.width).min(pane.x + pane.width);
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

/// The four cells just outside the corners of `pane` — where the pane's
/// edge seams meet perpendicular seams from siblings, forming junctions.
/// `focused_seam_segment` misses these because a junction cell sits on the
/// perpendicular seam's column/row, which is the pane's boundary and thus
/// one cell outside the accent sub-rect (which only covers the pane's own
/// span on the parallel axis). A corner is `None` when it would sit off the
/// screen origin: the screen edge carries no seam, so there is no junction
/// there to color. Coordinates past the far screen edge are harmless — the
/// draw loop's buffer clamp never visits them.
pub(super) fn focused_pane_corners(pane: Rect) -> [Option<(u16, u16)>; 4] {
    let x0 = pane.x.checked_sub(1);
    let x1 = pane.x.checked_add(pane.width);
    let y0 = pane.y.checked_sub(1);
    let y1 = pane.y.checked_add(pane.height);
    [x0.zip(y0), x1.zip(y0), x0.zip(y1), x1.zip(y1)]
}
