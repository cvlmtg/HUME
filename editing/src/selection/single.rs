use crate::grapheme::next_grapheme_boundary;
use crate::text::Text;

/// A single selection range within a buffer.
///
/// Both `anchor` and `head` are **char offsets** — indices into the buffer's
/// sequence of Unicode scalar values. The cursor (the moving end that the user
/// sees blinking) is always at `head`.
///
/// When `anchor == head`, the selection covers a single character — the one at
/// index `head`. This is the smallest possible selection, not a zero-width
/// point. The cursor block sits on that character, matching Helix/Kakoune's
/// inclusive model.
///
/// `head` must always be a valid char index (`< buf.len_chars()`). Since every
/// buffer always ends with a trailing `\n`, there is always at least one
/// character to sit on — even in an "empty" buffer.
///
/// # Directional selections
///
/// - **Forward** (anchor ≤ head): the user extended towards the end of the file.
/// - **Backward** (anchor > head): the user extended towards the start.
///
/// Use `start()` / `end()` when you need the bounds irrespective of direction,
/// and `anchor` / `head` when direction matters (e.g., when extending).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    /// The stationary end of the selection. Stays put when the user extends.
    pub(crate) anchor: usize,
    /// The moving end / cursor position.
    pub(crate) head: usize,
    /// Sticky display column for visual j/k motion. `None` means "not latched
    /// — recompute on next vertical move." Any horizontal motion or edit that
    /// touches this selection's line resets this to `None` by construction
    /// (constructors set it to `None`; only `with_horiz` preserves it).
    pub(crate) horiz: Option<u32>,
}

impl Selection {
    /// A collapsed selection at `pos` (anchor == head == pos). `horiz: None`.
    pub fn collapsed(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
            horiz: None,
        }
    }

    /// A directional range from `anchor` to `head`. `horiz: None`.
    /// Passing `anchor == head` produces a single-character selection.
    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            horiz: None,
        }
    }

    /// A directional selection with a preserved sticky display column.
    ///
    /// Used *only* by visual j/k motion to carry the column across consecutive
    /// vertical moves. All other code uses [`Self::new`] or [`Self::collapsed`] which reset
    /// `horiz` to `None` by construction.
    pub fn with_horiz(anchor: usize, head: usize, horiz: u32) -> Self {
        Self {
            anchor,
            head,
            horiz: Some(horiz),
        }
    }

    /// Create a selection spanning `[start, end]` with an explicit direction.
    ///
    /// `forward` controls which end becomes the anchor and which becomes the
    /// head (the cursor):
    /// - `true`  → `anchor = start`, `head = end`  (forward / rightward)
    /// - `false` → `anchor = end`,   `head = start` (backward / leftward)
    ///
    /// This is the preferred constructor when a selection is built from
    /// content-aware bounds (e.g. trimmed whitespace edges, line extents) and
    /// the original direction must be preserved. It avoids leaking
    /// `anchor`/`head` field knowledge into every call site.
    pub fn directed(start: usize, end: usize, forward: bool) -> Self {
        if forward {
            Self::new(start, end)
        } else {
            // Backward: anchor at end, head at start — cursor sits at `start`.
            Self::new(end, start)
        }
    }

    /// The stationary end (the end that stays put when the user extends).
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// The moving end / cursor position.
    pub fn head(&self) -> usize {
        self.head
    }

    /// Sticky display column for visual j/k motion, or `None` when not latched.
    pub fn horiz(&self) -> Option<u32> {
        self.horiz
    }

    /// Is this a single-character selection (anchor == head)?
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    /// The smaller of the two offsets — the start of the selected range.
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// The larger of the two offsets — the far end of the selected range.
    ///
    /// Returns the **start** of the grapheme cluster at that position. For
    /// single-codepoint graphemes (the common case) this equals the last char
    /// in the selection. For multi-codepoint clusters (e.g. `e + \u{0301}`)
    /// the combining codepoints that follow are NOT included — use
    /// [`Self::end_inclusive`] when computing deletion or slice bounds.
    ///
    /// In the inclusive cursor model this char IS part of the selection (the
    /// cursor or anchor sits on it). This is NOT an exclusive bound.
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// The last char position covered by this selection, inclusive of any
    /// combining codepoints that extend the grapheme at [`Self::end`].
    ///
    /// For single-codepoint graphemes this equals `end()`. For multi-codepoint
    /// clusters (e.g. `e + \u{0301}` = é) this extends to the last codepoint
    /// so that delete and slice operations never orphan a combining mark.
    ///
    /// Use this (not `end()`) when computing char ranges for deletion or
    /// buffer slices — all edit operations should use `end_inclusive`.
    pub fn end_inclusive(&self, buf: &Text) -> usize {
        // next_grapheme_boundary returns one past the cluster; subtract 1 to
        // get the last codepoint index (inclusive upper bound for the range).
        next_grapheme_boundary(buf, self.end()).saturating_sub(1)
    }

    /// Swap anchor and head. A forward selection becomes backward and vice
    /// versa. Useful for `flip selection` commands. `horiz` is cleared since
    /// the head moved to a potentially different column.
    #[must_use]
    pub fn flip(self) -> Self {
        Self {
            anchor: self.head,
            head: self.anchor,
            horiz: None,
        }
    }

    /// Move both anchor and head by `delta` chars (positive = forward).
    ///
    /// Used when an edit *before* this selection shifts all offsets.
    ///
    /// # Panics
    /// Panics if the shift would move either end below zero (underflow).
    /// This is always a bug in the caller — an edit cannot shift a selection
    /// to a negative position.
    #[cfg(test)]
    #[must_use]
    pub fn shift(self, delta: isize) -> Self {
        // `checked_add_signed` (stable since Rust 1.66) adds a signed delta to
        // a usize and returns None on overflow *or* underflow. Compared to the
        // previous `(x as isize + delta) as usize` cast pair, this fails loudly
        // in *both* debug and release builds — the cast silently wraps in
        // release, producing a huge position that corrupts the buffer.
        let anchor = self
            .anchor
            .checked_add_signed(delta)
            .expect("shift underflow: anchor cannot go below zero");
        let head = self
            .head
            .checked_add_signed(delta)
            .expect("shift underflow: head cannot go below zero");
        // Shifting changes the absolute position but not the column relationship,
        // so preserve horiz.
        Self {
            anchor,
            head,
            horiz: self.horiz,
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── Selection ─────────────────────────────────────────────────────────────

    #[test]
    fn cursor_is_collapsed() {
        let s = Selection::collapsed(5);
        assert_eq!(s.anchor, 5);
        assert_eq!(s.head, 5);
        assert!(s.is_collapsed());
    }

    #[test]
    fn forward_selection_start_end() {
        let s = Selection::new(2, 7); // anchor < head → forward
        assert_eq!(s.start(), 2);
        assert_eq!(s.end(), 7);
        assert!(!s.is_collapsed());
    }

    #[test]
    fn backward_selection_start_end() {
        let s = Selection::new(7, 2); // anchor > head → backward
        assert_eq!(s.start(), 2);
        assert_eq!(s.end(), 7);
    }

    #[test]
    fn flip_reverses_direction() {
        let fwd = Selection::new(2, 7);
        let bwd = fwd.flip();
        assert_eq!(bwd.anchor, 7);
        assert_eq!(bwd.head, 2);
        assert_eq!(fwd.flip().flip(), fwd); // double-flip is identity
    }

    #[test]
    fn shift_positive() {
        let s = Selection::new(2, 7);
        let shifted = s.shift(3);
        assert_eq!(shifted.anchor, 5);
        assert_eq!(shifted.head, 10);
    }

    #[test]
    fn shift_negative() {
        let s = Selection::new(5, 10);
        let shifted = s.shift(-3);
        assert_eq!(shifted.anchor, 2);
        assert_eq!(shifted.head, 7);
    }

    // ── Selection::directed ───────────────────────────────────────────────────

    #[test]
    fn directed_forward_places_anchor_at_start() {
        let sel = Selection::directed(3, 7, true);
        assert_eq!(sel.anchor, 3);
        assert_eq!(sel.head, 7);
        assert!(!sel.is_collapsed());
    }

    #[test]
    fn directed_backward_places_anchor_at_end() {
        let sel = Selection::directed(3, 7, false);
        assert_eq!(sel.anchor, 7);
        assert_eq!(sel.head, 3);
        assert!(!sel.is_collapsed());
    }

    #[test]
    fn directed_cursor_is_same_regardless_of_direction() {
        let fwd = Selection::directed(5, 5, true);
        let bwd = Selection::directed(5, 5, false);
        assert!(fwd.is_collapsed());
        assert!(bwd.is_collapsed());
        assert_eq!(fwd, bwd);
    }

    // ── Selection::shift panic ────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "shift underflow")]
    fn shift_underflow_panics() {
        let sel = Selection::collapsed(2);
        let _ = sel.shift(-3); // 2 + (-3) underflows
    }
}
