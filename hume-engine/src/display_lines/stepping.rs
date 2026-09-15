//! Display-line stepping (`next`/`prev`/`advance_saturating`/`distance`).
//! Reaches the map only through its public `block`/`last_line`/`clamp`
//! accessors.

use super::DisplayLineMap;
use super::pos::DisplayLinePos;

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
}
