//! The viewport's scroll verbs, and [`carry`] — the free-standing cursor-side
//! walk a view-scroll command (`commands::scroll_view` in `hume-editor`) runs
//! per selection, alongside a [`Viewport::scroll_by`] call, with the same
//! requested delta.
//!
//! Every verb here is the *only* way [`Viewport::top`](crate::pane::Viewport::top)
//! changes from outside this crate. Replaces what used to be four
//! independent read-modify-write sites in `hume-editor` (`clamp_viewport_top`,
//! `by_display_lines`, `ensure_cursor_visible`, `scroll_cursor_to_display_line`),
//! each re-deriving its own `(top_line, top_slot) -> DisplayLinePos` address
//! and its own scrolloff arithmetic. A [`ViewGeometry`] is resolved once per
//! call (`Viewport::geometry`) and threaded through, so no verb recomputes
//! [`vertical_margins`](crate::pane::vertical_margins) itself and no verb can
//! observe a zero-height viewport.
//!
//! `carry` deliberately does **not** take the value `scroll_by` returns:
//! `scroll_by`'s bound (`max_scroll_top`, a scrolloff-margin bound near EOF)
//! can be tighter than the document's true edge, so a large scroll request
//! can move the viewport less than `carry`'s own document/virtual-block-
//! bounded walk would move the cursor — the cursor is still fully visible
//! (well inside the viewport's own margin), it just didn't need to travel as
//! far as the view did. Coupling the two would under-move the cursor in
//! exactly that case (a `Ctrl+D` well before the document's real end). Each
//! walks its own bound against the same requested delta instead.

use super::pos::BlockSlot;
use super::{DisplayLineMap, DisplayLinePos};
use crate::pane::{ViewGeometry, Viewport};
use hume_rope::column::DisplayLineCol;

impl Viewport {
    /// Pull `top` onto a display line that actually exists.
    ///
    /// Single self-heal chokepoint for staleness: nothing else validates a
    /// `top` write against the block it addresses (`Pane::recall_scroll`
    /// restores a saved offset verbatim; an LSP goto-definition jump moves
    /// `top` without re-deriving its slot) — and the block a stale address
    /// was valid for can shrink or vanish (wrap-width change, a virtual-line
    /// decoration source removed, a resize) between the write and the next
    /// read. Call once per pane per frame, before any other verb, so every
    /// other write site can stay unvalidated.
    pub fn heal(&mut self, dlm: &mut DisplayLineMap<'_>) {
        self.top = dlm.clamp(self.top);
    }

    /// Scroll `delta` display lines (positive = down, negative = up),
    /// returning how many it actually moved, signed the same way. `carry`
    /// does *not* consume this return value — see this module's doc for why
    /// the cursor walks the same requested `delta` independently, with its
    /// own bound, rather than tracking how far the view actually got.
    ///
    /// Heals `top` first (see [`Viewport::heal`]), so a caller need not call
    /// it separately before scrolling.
    ///
    /// A *downward* scroll is bounded by [`DisplayLineMap::max_scroll_top`]
    /// and additionally never moves `top` backwards past where it already
    /// sits: [`Viewport::align`] (`z z`/`z k`/`z j`) and an LSP
    /// goto-definition jump both deliberately leave `top` past
    /// `max_scroll_top` (see that function's own doc on not clamping at the
    /// bottom), so a plain `next.min(max_scroll_top)` would pick the
    /// *earlier* of the two (`DisplayLinePos: Ord` is document order),
    /// snapping a downward notch backwards. An *upward* scroll only
    /// saturates at the document's first display line.
    pub fn scroll_by(
        &mut self,
        dlm: &mut DisplayLineMap<'_>,
        geo: ViewGeometry,
        delta: isize,
    ) -> isize {
        self.heal(dlm);
        let current = self.top;
        let (next, taken) = dlm.advance_counted_saturating(current, delta);
        if delta < 0 {
            self.top = next;
            return -(taken as isize);
        }
        // `max_scroll_top` walks back from the document's very last display
        // line, formatting every line it crosses under wrap — worth skipping
        // when `next` is provably nowhere near EOF. Every buffer line
        // contributes at least one display line, so a `next` more than
        // `geo.height` *buffer* lines short of the document's last one (an
        // O(1) check) can't be within the bound's own `target` (<=
        // `geo.height`) *display* lines of it, and the bound can't bind.
        let lines_to_end = dlm.last_line().index().saturating_sub(next.line.index());
        if lines_to_end > geo.height {
            self.top = next;
            return taken as isize;
        }
        let bound = dlm.max_scroll_top(geo);
        if next <= bound {
            self.top = next;
            return taken as isize;
        }
        let bounded = bound.max(current);
        self.top = bounded;
        dlm.distance(current, bounded, geo.height).unwrap_or(0) as isize
    }

    /// Adjust `top` so `cursor_pos` is visible with `geo.margin` display
    /// lines of look-ahead above and below it. Returns the cursor's
    /// resulting screen row — its distance below the (possibly just-moved)
    /// `top`.
    pub fn reveal(
        &mut self,
        dlm: &mut DisplayLineMap<'_>,
        geo: ViewGeometry,
        cursor_pos: DisplayLinePos,
    ) -> usize {
        let top = self.top;
        if cursor_pos < top {
            return self.scroll_back_from(dlm, cursor_pos, geo.margin);
        }
        // Capped at `geo.target` rather than `geo.height`: walking past
        // `geo.target` under wrap only pays for `format_buffer_line` calls
        // whose answer nothing inspects — `distance` never returns more than
        // its own cap, so a `Some` result here is always `<= geo.target`,
        // well inside the second arm below without needing its own
        // upper-bound check.
        match dlm.distance(top, cursor_pos, geo.target) {
            Some(rows) if rows < geo.margin => self.scroll_back_from(dlm, cursor_pos, geo.margin),
            Some(rows) => rows,
            None => self.scroll_back_from(dlm, cursor_pos, geo.target),
        }
    }

    /// Scroll so `cursor_pos` lands `row` display lines below `top`, clamped
    /// to `[geo.margin, geo.target]` — the same scrolloff range
    /// [`Viewport::reveal`] settles a too-close cursor into. Used by the
    /// `z z`/`z k`/`z j` view commands, which write no reveal-on-demand
    /// signal of their own, so the clamp is applied here rather than left to
    /// a follow-up `reveal` call.
    ///
    /// Top-of-buffer is clamped to the document's first display line;
    /// bottom-of-buffer is *not* clamped (vim/Helix semantics — empty
    /// display lines past EOF are allowed).
    pub fn align(
        &mut self,
        dlm: &mut DisplayLineMap<'_>,
        geo: ViewGeometry,
        cursor_pos: DisplayLinePos,
        row: usize,
    ) {
        let row = row.clamp(geo.margin, geo.target);
        self.scroll_back_from(dlm, cursor_pos, row);
    }

    /// Adjust `horizontal_offset` so `cursor_display_col` stays visible.
    /// Wrapping modes have no horizontal scroll, so the offset is forced to
    /// 0 there. The horizontal margin is fixed — scrolloff governs only the
    /// vertical axis, so this takes no `ViewGeometry`.
    pub fn reveal_horizontal(
        &mut self,
        dlm: &mut DisplayLineMap<'_>,
        cursor_display_col: DisplayLineCol,
    ) {
        const H_MARGIN: usize = 5;

        if dlm.is_wrapping() {
            self.horizontal_offset = DisplayLineCol::new(0);
            return;
        }

        // The rest of this function mixes the column with plain margin/width
        // counts throughout, so it drops to `.get()`'s bare `u32` at the top
        // rather than threading `DisplayLineCol` arithmetic through.
        let cursor_display_col = cursor_display_col.get() as usize;
        // `locate`'s column is content-relative (the gutter isn't part of
        // it), so the margin must compare against the content width the map
        // itself was built with — not `Viewport::width`, which still
        // includes the gutter and so under-counts how many columns are
        // actually visible.
        let content_width = dlm.content_width() as usize;
        if content_width == 0 {
            return;
        }

        let margin = H_MARGIN.min(content_width / 2);
        let offset = self.horizontal_offset.get() as usize;

        if cursor_display_col < offset + margin {
            self.horizontal_offset =
                DisplayLineCol::new(cursor_display_col.saturating_sub(margin) as u32);
        } else if cursor_display_col >= offset + content_width - margin {
            self.horizontal_offset = DisplayLineCol::new(
                cursor_display_col.saturating_sub(content_width - margin - 1) as u32,
            );
        }
    }

    /// Put `top` `display_lines_above` display lines before `cursor_pos`,
    /// saturating at the document's first display line. Returns how many
    /// display lines it actually stepped back — which, `next`/`prev` being
    /// inverses, is the cursor's screen row under the new `top`.
    fn scroll_back_from(
        &mut self,
        dlm: &mut DisplayLineMap<'_>,
        cursor_pos: DisplayLinePos,
        display_lines_above: usize,
    ) -> usize {
        let (top, stepped) =
            dlm.advance_counted_saturating(cursor_pos, -(display_lines_above as isize));
        self.top = top;
        stepped
    }
}

/// Where a head's display line goes after a view scroll of `delta` display
/// lines (the same signed delta passed to `Viewport::scroll_by` — see this
/// module's doc for why `carry` uses the *requested* delta, not the amount
/// the view actually moved).
///
/// Walks `delta` display lines from `head` in that direction, landing on the
/// last content display line reached. Overshoots `delta` when a virtual-line
/// block swallows the whole budget: a cursor stranded at a block's near edge
/// would make the next ordinary motion jump the view backwards across the
/// whole block to reach it, so this keeps walking past the block to the
/// first content display line beyond it. The document's own edge still
/// breaks the walk before either bound is satisfied.
///
/// Returns `None` when the walk crossed no content display line at all —
/// `head` already sat at the document's edge in the direction of travel, or
/// `delta == 0` — the exact case that leaves the selection untouched rather
/// than collapsing it. A `None` result is not an error: it is the same
/// "cursor can't follow" state a pure view scroll into a trailing
/// virtual-line block always could produce.
pub fn carry(
    dlm: &mut DisplayLineMap<'_>,
    head: DisplayLinePos,
    delta: isize,
) -> Option<DisplayLinePos> {
    if delta == 0 {
        return None;
    }
    let down = delta > 0;
    let mut pos = head;
    let mut last_content = None;
    let mut remaining = delta.unsigned_abs();
    while remaining > 0 || last_content.is_none() {
        let Some(next) = (if down { dlm.next(pos) } else { dlm.prev(pos) }) else {
            break;
        };
        pos = next;
        if matches!(dlm.slot(pos), BlockSlot::Content(_)) {
            last_content = Some(pos);
        }
        remaining = remaining.saturating_sub(1);
    }
    last_content
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
