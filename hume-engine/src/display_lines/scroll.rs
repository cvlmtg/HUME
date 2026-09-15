//! The viewport's scroll verbs, and [`carry`] — the free-standing cursor-side
//! walk a view-scroll command (`commands::scroll_view` in `hume-editor`) runs
//! per selection, alongside a [`Viewport::scroll_by`] call, with the same
//! requested delta.
//!
//! Every verb here is the *only* way [`Viewport::top`](crate::pane::Viewport::top)
//! changes from outside this crate — the single chokepoint for
//! `(top_line, top_slot) -> DisplayLinePos` address resolution and scrolloff
//! arithmetic. [`Viewport::top_at`] is the read counterpart to that write
//! chokepoint: the only way to obtain a `top` that is safe to walk. A
//! [`ViewGeometry`] is resolved once per call (`Viewport::geometry`) and
//! threaded through, so no verb recomputes `margin`/`target` itself and no
//! verb can observe a zero-height viewport.
//!
//! `carry` deliberately walks the requested `delta` on its own, rather than
//! being handed how far `scroll_by` actually moved: `scroll_by`'s bound
//! (`max_scroll_top`, a scrolloff-margin bound near EOF) can be tighter than
//! `carry`'s own document/virtual-block-bounded one, so within
//! `geo.height` buffer lines of EOF a large scroll request can move the
//! viewport less than it moves the cursor — the cursor is still fully
//! visible (well inside the viewport's own margin), it just didn't need to
//! travel as far as the view did. Coupling the two would under-move the
//! cursor in exactly that case — worse, tying `carry` to `scroll_by`'s
//! *actual* movement stalls it permanently once the viewport saturates at
//! that bound (every later call would see zero further movement and repeat
//! the same landing forever), never reaching the document's true last line
//! even though it's already on screen. Farther from EOF the two bounds
//! can't diverge at all (`scroll_by`'s own `lines_to_end` skip proves it —
//! see that check's own comment), so each walking its own bound against the
//! same requested delta costs nothing there and is what keeps the cursor
//! correct near EOF.
//!
//! `commands::scroll_view` (`hume-editor`) must leave every carried
//! selection's head inside the scrolloff band, or the selection untouched —
//! and nothing enforces that on its own: `max_scroll_top`'s bound and
//! [`Viewport::reveal`]'s own settle point merely happen to share a
//! `geo.target` number. `carry` closes that gap itself instead, in a second
//! pass over whatever the delta-walk above already landed on: it clamps how
//! many display lines below the viewport's *new* top that landing sits (the
//! caller reads `Viewport::top` after its own `scroll_by` call), into `[geo.margin,
//! geo.target]` — walking forward from the new top to the band's near or
//! far edge if the raw landing fell outside it. A landing already in-band
//! (the common case — most scrolls have somewhere to land within it) passes
//! through unchanged, so "the cursor keeps its screen row" still holds
//! exactly there. The corollary this restores: [`Viewport::reveal`] is
//! provably idle after every `scroll_view` call, since whatever `carry`
//! returns already satisfies `reveal`'s own contract.

use super::{DisplayLineMap, DisplayLinePos};
use crate::pane::{ViewGeometry, Viewport};
use hume_rope::column::DisplayLineCol;

impl Viewport {
    /// The viewport's top display-line address, resolved against `dlm`.
    ///
    /// Single self-heal chokepoint for staleness, and the only way to obtain a
    /// `top` that is safe to walk: nothing validates a `top` write against the
    /// block it addresses (`Pane::recall_scroll` restores a saved offset
    /// verbatim; an LSP goto-definition jump moves `top` without re-deriving its
    /// slot; `Pane::inherit_view_state` clones one wholesale) — and the block a
    /// stale address was valid for can shrink or vanish (wrap-width change, a
    /// virtual-line decoration source removed, a resize) between the write and
    /// the next read.
    ///
    /// Writes the resolved address back rather than returning a clamped copy, so
    /// the stored field converges on every read and `Viewport::top`'s own value
    /// is never a second, staler answer to the same question. Every read site
    /// that holds a `DisplayLineMap` goes through here; the three that hold none
    /// read [`Viewport::top`] and are documented there.
    pub fn top_at(&mut self, dlm: &mut DisplayLineMap<'_>) -> DisplayLinePos {
        self.top = dlm.clamp(self.top);
        self.top
    }

    /// Scroll `delta` display lines (positive = down, negative = up). `carry`
    /// walks the same requested `delta` independently, with its own bound,
    /// rather than tracking how far the view actually got — see this
    /// module's doc for why.
    ///
    /// Resolves `top` first (see [`Viewport::top_at`]), so a caller need not
    /// heal it separately before scrolling.
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
    pub fn scroll_by(&mut self, dlm: &mut DisplayLineMap<'_>, geo: ViewGeometry, delta: isize) {
        let current = self.top_at(dlm);
        let next = dlm.advance_saturating(current, delta);
        if delta < 0 {
            self.top = next;
            return;
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
            return;
        }
        let bound = dlm.max_scroll_top(geo);
        self.top = if next <= bound {
            next
        } else {
            bound.max(current)
        };
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
        let top = self.top_at(dlm);
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
            Some(display_lines_below_top) if display_lines_below_top < geo.margin => {
                self.scroll_back_from(dlm, cursor_pos, geo.margin)
            }
            Some(display_lines_below_top) => display_lines_below_top,
            None => self.scroll_back_from(dlm, cursor_pos, geo.target),
        }
    }

    /// Scroll so `cursor_pos` lands `display_lines_below_top` display lines
    /// below `top`, clamped to `[geo.margin, geo.target]` — the same
    /// scrolloff range [`Viewport::reveal`] settles a too-close cursor into.
    /// Used by the `z z`/`z k`/`z j` view commands, which write no
    /// reveal-on-demand signal of their own, so the clamp is applied here
    /// rather than left to a follow-up `reveal` call.
    ///
    /// Top-of-buffer is clamped to the document's first display line;
    /// bottom-of-buffer is *not* clamped (vim/Helix semantics — empty
    /// display lines past EOF are allowed).
    pub fn align(
        &mut self,
        dlm: &mut DisplayLineMap<'_>,
        geo: ViewGeometry,
        cursor_pos: DisplayLinePos,
        display_lines_below_top: usize,
    ) {
        let display_lines_below_top = display_lines_below_top.clamp(geo.margin, geo.target);
        self.scroll_back_from(dlm, cursor_pos, display_lines_below_top);
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

impl<'a> DisplayLineMap<'a> {
    /// The furthest down the viewport top may scroll: `geo.target` display
    /// lines of look-ahead past the document's last display line — a
    /// trailing `After` virtual block included, exactly like any real buffer
    /// line — then no further. `geo` is [`Viewport::geometry`], the same
    /// geometry [`Viewport::reveal`] resolves, so the two agree on
    /// `margin`/`target`.
    ///
    /// The two bounds' *anchors* deliberately do not agree: this one
    /// measures back from the document's last display line, while `reveal`
    /// measures from the cursor's own display line — which can never be a
    /// virtual one, since the cursor only ever occupies content display
    /// lines. That gap is what lets a scroll carry the viewport into a
    /// trailing virtual block at all; without it, the cursor being unable to
    /// follow would cap the scroll at the block's near edge. The two anchors
    /// coincide, and so land on the same top, only when the cursor sits on
    /// the document's last display line — the case a plain `Ctrl+D`/wheel
    /// scroll to EOF followed by an ordinary cursor motion exercises.
    /// Saturates at the document's first display line, so a document that
    /// fits on screen (plus its margin) cannot be scrolled at all.
    ///
    /// Takes a [`ViewGeometry`] rather than a raw height precisely so a
    /// zero-height viewport never reaches here at all — `Viewport::geometry`
    /// returns `None` for one, so every caller already branched away before
    /// constructing the `geo` this needs. `advance_saturating(last, -0)`
    /// would otherwise return the document's *last* display line — the
    /// opposite of "saturates at the first" — for the degenerate `target ==
    /// 0` a zero height produces.
    pub(crate) fn max_scroll_top(&mut self, geo: ViewGeometry) -> DisplayLinePos {
        let last_line = self.last_line();
        let last = DisplayLinePos::new(last_line, self.block(last_line).total().saturating_sub(1));
        self.advance_saturating(last, -(geo.target as isize))
    }
}

/// Where a head's display line goes after a view scroll of `delta` display
/// lines (the same signed delta passed to `Viewport::scroll_by`), band-
/// clamped against `top` — the viewport's top *after* that `scroll_by` call
/// — so a `Some` result always already satisfies [`Viewport::reveal`]'s own
/// contract. See this module's doc for why the walk itself still uses the
/// *requested* `delta`, not the amount the view actually moved, and for what
/// the band clamp adds on top of that.
///
/// Two passes: [`walk_by_delta`] finds where a plain `delta`-display-line
/// walk from `head` would land (`None` if it never reaches a content
/// line — `head` already sat at the document's edge in the direction of
/// travel, `delta == 0`, or a virtual-line block past the band swallowed the
/// walk whole); [`place_in_band`] then clamps how many display lines below
/// `top` that landing sits into `[geo.margin, geo.target]`, if it isn't
/// there already. Either pass returning `None` is not an error: it is the same
/// "cursor can't follow" state a pure view scroll into a trailing
/// virtual-line block always could produce, and it leaves the selection
/// untouched rather than collapsing it.
pub fn carry(
    dlm: &mut DisplayLineMap<'_>,
    geo: ViewGeometry,
    top: DisplayLinePos,
    head: DisplayLinePos,
    delta: isize,
) -> Option<DisplayLinePos> {
    let candidate = walk_by_delta(dlm, geo, top, head, delta)?;
    place_in_band(dlm, geo, top, candidate)
}

/// `carry`'s first pass: walk `delta` display lines from `head` in the
/// requested direction, landing on the last content display line reached.
/// Overshoots `delta` when a virtual-line block swallows the whole budget: a
/// cursor stranded at a block's near edge would make the next ordinary
/// motion jump the view backwards across the whole block to reach it, so
/// this keeps walking past the block to the first content display line
/// beyond it — but only while doing so could still land inside the
/// scrolloff band (`geo.target` display lines past `top`); a block bigger
/// than that has no legal landing spot at all, so the walk gives up instead
/// of continuing arbitrarily far through it. The document's own edge still
/// breaks the walk before either bound is satisfied.
fn walk_by_delta(
    dlm: &mut DisplayLineMap<'_>,
    geo: ViewGeometry,
    top: DisplayLinePos,
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
        if remaining == 0 {
            match dlm.distance(top, pos, geo.target) {
                Some(display_lines_below_top) if display_lines_below_top < geo.target => {}
                _ => break,
            }
        }
        let Some(next) = (if down { dlm.next(pos) } else { dlm.prev(pos) }) else {
            break;
        };
        pos = next;
        if dlm.slot(pos).is_content() {
            last_content = Some(pos);
        }
        remaining = remaining.saturating_sub(1);
    }
    last_content
}

/// `carry`'s second pass: clamp how many display lines below `top`
/// `candidate` sits into `[geo.margin, geo.target]` — walking forward from
/// `top` to the band's near or far edge via [`walk_from_top`] if it isn't
/// there already. A landing already in-band (the common case) passes
/// through unchanged, so "the cursor keeps its screen row" still holds
/// exactly for it.
fn place_in_band(
    dlm: &mut DisplayLineMap<'_>,
    geo: ViewGeometry,
    top: DisplayLinePos,
    candidate: DisplayLinePos,
) -> Option<DisplayLinePos> {
    match dlm.distance(top, candidate, geo.height) {
        Some(display_lines_below_top)
            if (geo.margin..=geo.target).contains(&display_lines_below_top) =>
        {
            Some(candidate)
        }
        Some(display_lines_below_top) if display_lines_below_top > geo.target => {
            walk_from_top(dlm, geo, top, geo.target)
        }
        // `display_lines_below_top < geo.margin`, or `candidate` sits before
        // `top` (or too far past `geo.height` to tell) — either way, not in band.
        _ => walk_from_top(dlm, geo, top, geo.margin),
    }
}

/// Walk forward (`next`) from `top` by `display_lines_below_top` display
/// lines, landing on the last content display line reached — continuing
/// past a virtual landing exactly like [`walk_by_delta`]'s own overshoot,
/// capped at `geo.target` total steps so this can never itself land past the
/// band. `top` counts as a landing in its own right when it's already
/// content and `display_lines_below_top == 0` (the `geo.margin == 0` case a
/// very short viewport clamps to).
fn walk_from_top(
    dlm: &mut DisplayLineMap<'_>,
    geo: ViewGeometry,
    top: DisplayLinePos,
    display_lines_below_top: usize,
) -> Option<DisplayLinePos> {
    let mut pos = top;
    let mut last_content = dlm.slot(top).is_content().then_some(top);
    let mut steps = 0usize;
    while steps < display_lines_below_top || last_content.is_none() {
        if steps >= geo.target {
            break;
        }
        let Some(next) = dlm.next(pos) else { break };
        pos = next;
        steps += 1;
        if dlm.slot(pos).is_content() {
            last_content = Some(pos);
        }
    }
    last_content
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
