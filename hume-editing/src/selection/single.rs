use hume_rope::column::{BufferLineCol, DisplayLineCol};
use hume_rope::offset::{CharOffset, ExclusiveRange, InclusiveRange};

use crate::grapheme::{cluster_last_char, next_grapheme_boundary};
use crate::lines::is_line_start;
use crate::text::BufferText;

/// A display column together with the frame it was measured in.
///
/// Both variants are `hume_engine::display_lines::DisplayLineMap` quantities — one authority,
/// so both count tab expansion, wide glyphs and inline decorations (inlay
/// hints, ghost text) identically. They differ only in what they're measured
/// *from*: under soft wrap, a continuation display line renumbers its
/// columns from its own left edge (its indent, under `WrapMode::Indent`), so
/// the same character has a different [`DisplayLineCol`] than
/// [`BufferLineCol`] — reading one as the other sends the cursor sideways,
/// which [`DisplayLineCol`]/[`BufferLineCol`] being distinct types makes a
/// compile error rather than a bug to find at runtime. With wrapping off a
/// display line *is* the whole buffer line, so the two coincide and either
/// variant reads back the same number. This is why a motion switching
/// families (`j` then `2j`, or vice versa) re-derives instead of reusing a
/// latch tagged with the other variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StickyDisplayCol {
    /// Column within the current display line (`DisplayLineMap::locate`) — what
    /// `j`/`k`, page/half-page scroll, and the mouse wheel latch.
    DisplayLine {
        display_col: DisplayLineCol,
        /// The wrap column `display_col` was measured against
        /// (`DisplayLineMap::resolved_wrap_width`). A pane resize changes what
        /// column a display-line-relative latch's number means (the same
        /// display-line-relative column addresses a different buffer
        /// position once display lines re-flow at a new width), so a reader
        /// compares this against `DisplayLineMap`'s *current* resolved width and
        /// re-derives on a mismatch instead of reusing a column measured for
        /// a wrap geometry that no longer exists.
        wrap_width: Option<u16>,
    },
    /// Column within the buffer line (`DisplayLineMap::buffer_line_col`) — what an
    /// explicit numeric prefix (`9j`/`9k`) latches. Carries no wrap width:
    /// a buffer-line column counts a line's own characters and never
    /// depends on wrap geometry, unlike the `DisplayLine` variant above.
    BufferLine { display_col: BufferLineCol },
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
/// `head` must always be a valid char index (`< text.len_chars()`). Since every
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
    pub(crate) anchor: CharOffset,
    /// The moving end / cursor position.
    pub(crate) head: CharOffset,
    /// Sticky display column for vertical motion. `None` means "not latched
    /// — recompute on next vertical move." Any horizontal motion or edit that
    /// touches this selection's line resets this to `None` by construction
    /// (constructors set it to `None`; `with_sticky_display_col` sets it, and
    /// `SelectionSet::translate_in_place` carries it through an edit on a
    /// different line).
    pub(crate) sticky_display_col: Option<StickyDisplayCol>,
}

impl Selection {
    /// A collapsed selection at `pos` (anchor == head == pos). `sticky_display_col: None`.
    pub fn collapsed(pos: CharOffset) -> Self {
        Self {
            anchor: pos,
            head: pos,
            sticky_display_col: None,
        }
    }

    /// A directional range from `anchor` to `head`. `sticky_display_col: None`.
    /// Passing `anchor == head` produces a single-character selection.
    pub fn new(anchor: CharOffset, head: CharOffset) -> Self {
        Self {
            anchor,
            head,
            sticky_display_col: None,
        }
    }

    /// A directional selection with a preserved sticky display column.
    ///
    /// Used by `editor::visual_move`'s vertical motion (display-line-domain
    /// `j`/`k`/scroll/wheel, and buffer-line `9j`/`9k` — all three units
    /// share one path there) to carry the column across consecutive vertical moves,
    /// and by word-snap (`text_object::apply_nearest_word_result`) to pass
    /// an existing latch through unchanged. All other code uses
    /// [`Self::new`] or [`Self::collapsed`], which reset
    /// `sticky_display_col` to `None`.
    pub fn with_sticky_display_col(
        anchor: CharOffset,
        head: CharOffset,
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
    pub fn directed(start: CharOffset, end: CharOffset, forward: bool) -> Self {
        if forward {
            Self::new(start, end)
        } else {
            // Backward: anchor at end, head at start — cursor sits at `start`.
            Self::new(end, start)
        }
    }

    /// This selection's extent unioned with `span` — `min` of both starts,
    /// `max` of both ends — built with [`Self::directed`] so the caller
    /// controls which end becomes the anchor.
    ///
    /// Shared by every "extend to cover a newly found match" path:
    /// `hume-ops`'s `text_object::apply_text_object_extend`,
    /// `text_object::word::apply_nearest_word_result`, and
    /// `motion::apply_object_motion`'s Extend arm. A found range only
    /// guarantees it starts past (or ends before) the search origin, not
    /// that it extends past the selection's own far edge — a plain
    /// replacement would shrink the selection when the found range nests
    /// inside what's already selected; the union absorbs it with no visible
    /// change instead.
    pub fn union_span(&self, span: InclusiveRange<CharOffset>, forward: bool) -> Self {
        let new_start = self.start().min(span.start);
        let new_end = self.end().max(span.end);
        Self::directed(new_start, new_end, forward)
    }

    /// The stationary end (the end that stays put when the user extends).
    pub fn anchor(&self) -> CharOffset {
        self.anchor
    }

    /// The moving end / cursor position.
    pub fn head(&self) -> CharOffset {
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
    pub fn start(&self) -> CharOffset {
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
    pub fn end(&self) -> CharOffset {
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
    pub fn end_inclusive(&self, text: &BufferText) -> CharOffset {
        cluster_last_char(text, self.end())
    }

    /// The char offset one past this selection's last char — the exclusive
    /// counterpart to [`Self::end_inclusive`], for `text.slice(ExclusiveRange::new(start, end_exclusive))`
    /// and delete-range math. Always `next_grapheme_boundary(text, self.end())`:
    /// `end_inclusive` is defined as that boundary minus one
    /// (`cluster_last_char`'s doc), so this recovers the true exclusive bound
    /// without a raw `+ 1` at the call site.
    pub fn end_exclusive(&self, text: &BufferText) -> CharOffset {
        next_grapheme_boundary(text, self.end())
    }

    /// This selection's text as a rope slice — `text.slice(start..end_exclusive)`
    /// via [`ExclusiveRange`]. The one place that expression is spelled out;
    /// every other caller wanting a selection's exact contents goes through here.
    pub fn slice<'a>(&self, text: &'a BufferText) -> ropey::RopeSlice<'a> {
        text.slice(ExclusiveRange::new(self.start(), self.end_exclusive(text)))
    }

    /// Returns `true` if the far end of the selection sits on a `\n`.
    ///
    /// A selection produced by `select-line` always ends on the line's trailing
    /// `\n`. Charwise and word selections end on content characters.
    pub fn ends_on_newline(&self, text: &BufferText) -> bool {
        text.char_at(self.end()) == Some('\n')
    }

    /// The last char offset to delete from this selection without touching the
    /// structural trailing `\n`.
    ///
    /// Equivalent to `end_inclusive(text).min(text.last_content_char())`. Use
    /// instead of inlining that expression to make the protection intent clear.
    pub fn content_end(&self, text: &BufferText) -> CharOffset {
        self.end_inclusive(text).min(text.last_content_char())
    }

    /// The exclusive counterpart to [`Self::content_end`] — `end_exclusive(text)`
    /// clamped to `text.last_char()`, so a caller building a
    /// `text.slice(ExclusiveRange::new(start, content_end_exclusive))` for a
    /// delete never reaches past the structural trailing `\n`.
    pub fn content_end_exclusive(&self, text: &BufferText) -> CharOffset {
        self.end_exclusive(text).min(text.last_char())
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
}

/// Returns `true` if `sel` covers whole line(s) — starts at a line boundary
/// and ends on the line's trailing `\n`.
///
/// A partial line that merely happens to include a trailing `\n` returns
/// `false` because its start is not at a line boundary. Use this (not just
/// `ends_on_newline`) as the single source of truth for "this selection is
/// linewise" in the selection-geometry domain — it answers `true` for a
/// selection collapsed on an empty line, since that line's one char is both
/// its own start and its own `\n`, even when the cursor is merely incidental
/// there. For *user intent* ("was this deliberately extended across whole
/// lines?"), use [`linewise_classification`] instead.
///
/// Counterpart to `is_register_linewise` in `ops::register`, which answers
/// "is this *register text* linewise?" at paste time.
pub fn is_selection_linewise(text: &BufferText, sel: &Selection) -> bool {
    sel.ends_on_newline(text) && is_line_start(text, sel)
}

/// A selection collapsed onto a single empty line is ambiguous — see
/// [`is_selection_linewise`]'s doc for why — so this returns `None` for
/// exactly that one case; every other selection is
/// `Some(is_selection_linewise(text, sel))`, unambiguously either linewise
/// or charwise.
pub fn linewise_classification(text: &BufferText, sel: &Selection) -> Option<bool> {
    match is_selection_linewise(text, sel) {
        true if sel.is_collapsed() => None,
        linewise => Some(linewise),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
