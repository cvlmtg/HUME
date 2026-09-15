//! Display-line stepping (`next`/`prev`/`advance_saturating`/`distance`/`max_scroll_top`).
//! Reaches the map only through its public `block`/`last_line`/`clamp`
//! accessors.

use super::DisplayLineMap;
use super::pos::DisplayLinePos;
use crate::pane::ViewGeometry;

impl<'a> DisplayLineMap<'a> {
    /// The next display line, crossing into the next line's block as needed.
    /// `None` only at the document's very last display line.
    pub fn next(&mut self, pos: DisplayLinePos) -> Option<DisplayLinePos> {
        let total = self.block(pos.line).total();
        if pos.slot + 1 < total {
            return Some(DisplayLinePos::new(pos.line, pos.slot + 1));
        }
        (pos.line < self.last_line()).then(|| DisplayLinePos::new(pos.line.advance(1), 0))
    }

    /// The previous display line. `None` only at the document's first display line.
    pub fn prev(&mut self, pos: DisplayLinePos) -> Option<DisplayLinePos> {
        if pos.slot > 0 {
            return Some(DisplayLinePos::new(pos.line, pos.slot - 1));
        }
        if pos.line.index() == 0 {
            return None;
        }
        let prev_line = pos.line.retreat_saturating(1);
        let total = self.block(prev_line).total();
        Some(DisplayLinePos::new(prev_line, total.saturating_sub(1)))
    }

    /// Step `delta` display lines from `pos`, saturating at either end of the
    /// document. The starting address is clamped first, so a stale viewport
    /// self-heals. Named `advance_saturating`, not bare `advance`, matching
    /// the same `_saturating`-means-clamps convention `hume-rope`'s
    /// offset/column/line domain types use — this walker's contract is no
    /// different just because it lives outside that crate.
    pub fn advance_saturating(&mut self, pos: DisplayLinePos, delta: isize) -> DisplayLinePos {
        self.advance_counted_saturating(pos, delta).0
    }

    /// [`DisplayLineMap::advance_saturating`], plus how many display lines it
    /// actually stepped — fewer than `delta.unsigned_abs()` only when the
    /// document's edge stopped the walk.
    ///
    /// Since [`DisplayLineMap::next`] and [`DisplayLineMap::prev`] are exact
    /// inverses, the count is also the distance back: after stepping `n`
    /// display lines *backward* from `pos`, `distance(result, pos) == n`.
    /// That lets a caller that scrolled backward from the cursor learn the
    /// cursor's resulting screen row without walking the same display lines
    /// forward again.
    pub fn advance_counted_saturating(
        &mut self,
        pos: DisplayLinePos,
        delta: isize,
    ) -> (DisplayLinePos, usize) {
        let mut cur = self.clamp(pos);
        let mut taken = 0;
        for _ in 0..delta.unsigned_abs() {
            let stepped = if delta >= 0 {
                self.next(cur)
            } else {
                self.prev(cur)
            };
            match stepped {
                Some(next) => cur = next,
                None => break,
            }
            taken += 1;
        }
        (cur, taken)
    }

    /// Display lines from `from` forward to `to`, or `None` if `to` is
    /// behind `from` or more than `cap` display lines ahead.
    ///
    /// Callers pass the viewport height as `cap`, which keeps this O(height)
    /// however large the document is. Both "behind" and "too far" collapse to
    /// `None` because every caller asks the same question — is `to` visible
    /// from `from` — and neither case is.
    pub fn distance(
        &mut self,
        from: DisplayLinePos,
        to: DisplayLinePos,
        cap: usize,
    ) -> Option<usize> {
        if to < from {
            return None;
        }
        // Every line's block occupies at least one display line (`block`'s
        // own `content >= 1` assert), so crossing `to.line - from.line`
        // lines costs at least that many steps — a line delta beyond `cap`
        // already proves the walk below would return `None`, without
        // formatting a single line under wrap to find out.
        if to.line.index().saturating_sub(from.line.index()) > cap {
            return None;
        }
        let mut cur = from;
        for steps in 0..=cap {
            if cur == to {
                return Some(steps);
            }
            cur = self.next(cur)?;
        }
        None
    }

    /// The furthest down the viewport top may scroll: `geo.target` display
    /// lines of look-ahead past the document's last display line — a
    /// trailing `After` virtual block included, exactly like any real buffer
    /// line — then no further. `geo` is [`Viewport::geometry`](crate::pane::Viewport::geometry),
    /// the same geometry [`Viewport::reveal`](crate::pane::Viewport::reveal) resolves,
    /// so the two agree on `margin`/`target`.
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
    pub fn max_scroll_top(&mut self, geo: ViewGeometry) -> DisplayLinePos {
        let last_line = self.last_line();
        let last = DisplayLinePos::new(last_line, self.block(last_line).total().saturating_sub(1));
        self.advance_saturating(last, -(geo.target as isize))
    }
}
