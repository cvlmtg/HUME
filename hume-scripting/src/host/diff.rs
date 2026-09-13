//! BufferText diffing — moved out of `host.rs`'s per-capability split.

use hume_engine::pipeline::BufferId;

/// A single line-level change between two texts, 0-based and Steel-surface
/// ready — `set-signs!`/`set-virtual-lines!` are 0-indexed at the Steel
/// boundary, so no arithmetic is needed to feed a hunk into either. The
/// count each side covers is `old_lines.len()`/`new_lines.len()` — there is
/// no separate count field to keep in sync. A zero-length side needs no
/// special anchoring case: its empty line list already sits exactly at the
/// insertion/deletion point (`old_lines` empty for a pure insert, `new_lines`
/// empty for a pure deletion). `Equal` runs are never represented —
/// `DiffHost` methods drop them before returning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    /// Content line in the old text where this hunk starts. A pure insertion
    /// (`old_lines` empty) names an *insertion position*, not a changed
    /// line — it may legitimately equal the old text's content line count
    /// (one past the last line), the same one-past-last-line value
    /// `ContentLineCount::end_exclusive()` names, which is why this is
    /// minted trusted (`ContentLine::new`) rather than through `::checked`,
    /// which would reject exactly that value.
    pub old_start: hume_rope::line::ContentLine,
    /// Line index in the new text where this hunk starts — see `old_start`'s
    /// doc; the same insertion-position case applies here for a pure
    /// deletion (`new_lines` empty).
    pub new_start: hume_rope::line::ContentLine,
    /// The covered old-side lines, trailing newlines stripped.
    pub old_lines: Vec<String>,
    /// The covered new-side lines, trailing newlines stripped.
    pub new_lines: Vec<String>,
}

/// A single word-level change between two texts — e.g. a single changed
/// line's old/new text, as passed from a `diff-lines`/`diff-buffer-lines`
/// `Replace` hunk. Ranges are 0-based **char offsets**, not byte offsets,
/// matching `WordHunk`/`ExtraHighlightEntry`/`set-virtual-lines!`'s
/// `'segments`. `Equal` runs are dropped, same as [`DiffHunk`].
///
/// Unlike [`DiffHunk`] (line-index `start` into a rebuilt line list), a
/// word hunk is one contiguous span of text per side, so it carries `end`
/// (an exclusive char offset) and one `String` per side rather than a line
/// list — reusing `DiffHunk`'s shape here would force a fake
/// single-element `Vec<String>` that doesn't mean the same thing.
///
/// A zero-width side (`start == end`) needs no special case, same
/// rationale as `DiffHunk`'s empty-line-list side: it already sits exactly
/// at the insertion/deletion point (pure insert: `old_start == old_end`,
/// pure delete: `new_start == new_end`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordDiffHunk {
    pub old_start: usize,
    pub old_end: usize,
    pub new_start: usize,
    pub new_end: usize,
    pub old_text: String,
    pub new_text: String,
}

/// BufferText diffing — accessed through [`EditorHost::diff`](super::EditorHost::diff). Backs
/// `(diff-lines old-text new-text)` / `(diff-buffer-lines bid ref-text)` /
/// `(diff-words old-text new-text)`.
///
/// `diff_lines`/`diff_buffer_lines` treat their string inputs as buffer
/// text: every line ending becomes LF and a trailing newline is added if
/// missing, matching how HUME would load them from disk. This is a
/// deliberate divergence from `git diff`'s raw byte comparison — a file
/// missing its final newline reports no change on that line, since nothing
/// would change about it on save either. `diff_words` does **no** such
/// normalization: its inputs are
/// single lines already extracted from `BufferText`-normalized content (typically
/// one side of a `Replace` hunk), so wrapping them again would be a no-op
/// at best.
pub trait DiffHost {
    /// Line-level hunks between `old` and `new`, `Equal` runs dropped.
    fn diff_lines(&self, old: &str, new: &str) -> Vec<DiffHunk>;

    /// As [`diff_lines`](DiffHost::diff_lines), diffing `ref_text` against
    /// `bid`'s live (dirty) in-memory text — avoids materializing the whole
    /// buffer as a Steel string on every debounced call. `None` for an
    /// unknown/stale `bid` — the single liveness check this call needs
    /// (looking up the buffer's text also answers "does it exist"), so the
    /// Steel boundary maps `None` straight to an error rather than checking
    /// liveness a second time first.
    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<Vec<DiffHunk>>;

    /// Word-level hunks between `old` and `new`, `Equal` runs dropped. The
    /// returned `bool` mirrors `WordDiff::deadline_hit()`: `true` means the
    /// underlying Myers pass could not finish within its deadline and
    /// returned a coarse (Replace-all) result — unlike line-diff's Myers
    /// fallback (still a correct partition), a word-diff timeout result
    /// should be treated as a fallback, not a precise diff (skip word
    /// highlighting, fall back to a whole-line scope).
    fn diff_words(&self, old: &str, new: &str) -> (Vec<WordDiffHunk>, bool);
}
