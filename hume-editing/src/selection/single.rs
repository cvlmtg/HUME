use crate::grapheme::next_grapheme_boundary;
use crate::lines::is_line_start;
use crate::text::Text;

/// What a [`StickyDisplayCol`]'s number is measured *from*.
///
/// A display column is only comparable to another one measured the same way.
/// Under soft wrap, a continuation row renumbers its columns from its own left
/// edge (its indent, under `WrapMode::Indent`), so the same character has a
/// different `DisplayRow` column than `BufferLine` column — reading one as the
/// other sends the cursor sideways. With wrapping off a row *is* the whole
/// line, so the two coincide and either origin reads back the same number.
/// This is why a motion switching families (`j` then `2j`, or vice versa)
/// re-derives instead of reusing a latch tagged with the other origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayColOrigin {
    /// Column within the current display row (`hume_engine::rows::RowMap`) —
    /// what `j`/`k`, page/half-page scroll, and the mouse wheel latch.
    DisplayRow,
    /// Column within the buffer line (`hume_rope::grapheme::display_col_in_line`)
    /// — what an explicit numeric prefix (`9j`/`9k`) latches.
    BufferLine,
}

/// A display column together with the frame it was measured in. See
/// [`DisplayColOrigin`] for why the two can't be compared directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StickyDisplayCol {
    pub display_col: u32,
    pub origin: DisplayColOrigin,
}

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
    /// Sticky display column for vertical motion. `None` means "not latched
    /// — recompute on next vertical move." Any horizontal motion or edit that
    /// touches this selection's line resets this to `None` by construction
    /// (constructors set it to `None`; `with_sticky_display_col` sets it, and
    /// `shift` carries it through a same-line edit).
    pub(crate) sticky_display_col: Option<StickyDisplayCol>,
}

impl Selection {
    /// A collapsed selection at `pos` (anchor == head == pos). `sticky_display_col: None`.
    pub fn collapsed(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
            sticky_display_col: None,
        }
    }

    /// A directional range from `anchor` to `head`. `sticky_display_col: None`.
    /// Passing `anchor == head` produces a single-character selection.
    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            sticky_display_col: None,
        }
    }

    /// A directional selection with a preserved sticky display column.
    ///
    /// Used by the two vertical motion paths (`editor::visual_move`'s row-domain
    /// `j`/`k`/scroll/wheel, and `hume-ops`'s buffer-line `9j`/`9k`) to carry
    /// the column across consecutive vertical moves, and by word-snap
    /// (`text_object::apply_nearest_word_result`) to pass an existing latch
    /// through unchanged. All other code uses [`Self::new`] or
    /// [`Self::collapsed`], which reset `sticky_display_col` to `None`.
    pub fn with_sticky_display_col(
        anchor: usize,
        head: usize,
        sticky_display_col: StickyDisplayCol,
    ) -> Self {
        Self {
            anchor,
            head,
            sticky_display_col: Some(sticky_display_col),
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

    /// Sticky display column for vertical motion, or `None` when not latched.
    pub fn sticky_display_col(&self) -> Option<StickyDisplayCol> {
        self.sticky_display_col
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

    /// Returns `true` if the far end of the selection sits on a `\n`.
    ///
    /// A selection produced by `select-line` always ends on the line's trailing
    /// `\n`. Charwise and word selections end on content characters.
    pub fn ends_on_newline(&self, buf: &Text) -> bool {
        buf.char_at(self.end()) == Some('\n')
    }

    /// The last char offset to delete from this selection without touching the
    /// structural trailing `\n`.
    ///
    /// Equivalent to `end_inclusive(buf).min(buf.last_content_char())`. Use
    /// instead of inlining that expression to make the protection intent clear.
    pub fn content_end(&self, buf: &Text) -> usize {
        self.end_inclusive(buf).min(buf.last_content_char())
    }

    /// Swap anchor and head. A forward selection becomes backward and vice
    /// versa. Useful for `flip selection` commands. `sticky_display_col` is
    /// cleared since the head moved to a potentially different column.
    #[must_use]
    pub fn flip(self) -> Self {
        Self {
            anchor: self.head,
            head: self.anchor,
            sticky_display_col: None,
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
    pub(crate) fn shift(self, delta: isize) -> Self {
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
        // so preserve sticky_display_col.
        Self {
            anchor,
            head,
            sticky_display_col: self.sticky_display_col,
        }
    }
}

/// Returns `true` if `sel` covers whole line(s) — starts at a line boundary
/// and ends on the line's trailing `\n`.
///
/// A partial line that merely happens to include a trailing `\n` returns
/// `false` because its start is not at a line boundary. Use this (not just
/// `ends_on_newline`) as the single source of truth for "this selection is
/// linewise" in the selection-geometry domain.
///
/// Counterpart to `is_register_linewise` in `ops::register`, which answers
/// "is this *register text* linewise?" at paste time.
pub fn is_selection_linewise(buf: &Text, sel: &Selection) -> bool {
    sel.ends_on_newline(buf) && is_line_start(buf, sel)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
