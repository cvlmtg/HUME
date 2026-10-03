//! BufferText diffing.

use hume_editing::changeset::ChangeHunk;
use hume_engine::pipeline::BufferId;

/// A single word-level change between two texts, e.g. a single changed
/// line's old/new text, as passed from a `diff-lines`/`diff-buffer-lines`
/// `Replace` hunk. Ranges are 0-based **char offsets**, not byte offsets,
/// matching `WordHunk`/`ExtraHighlightEntry`/`set-virtual-lines!`'s
/// `'segments`. `Equal` runs are dropped, same as a [`ChangeHunk`].
///
/// Unlike a [`ChangeHunk`] (line-index `start` into a rebuilt line list), a
/// word hunk is one contiguous span of text per side, so it carries `end`
/// (an exclusive char offset) and one `String` per side rather than a line
/// list. Reusing `ChangeHunk`'s shape here would force a fake
/// single-element `Vec<String>` that doesn't mean the same thing.
///
/// A zero-width side (`start == end`) needs no special case, same
/// rationale as `ChangeHunk`'s empty-line-list side: it already sits exactly
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

/// BufferText diffing, accessed through [`EditorHost::diff`](super::EditorHost::diff). Backs
/// `(diff-lines old-text new-text)` / `(diff-buffer-lines bid ref-text)` /
/// `(diff-words old-text new-text)`.
///
/// `diff_lines`/`diff_buffer_lines` treat their string inputs as buffer
/// text: every line ending becomes LF and a trailing newline is added if
/// missing, matching how HUME would load them from disk. This is a
/// deliberate divergence from `git diff`'s raw byte comparison: a file
/// missing its final newline reports no change on that line, since nothing
/// would change about it on save either. `diff_words` does **no** such
/// normalization: its inputs are
/// single lines already extracted from `BufferText`-normalized content (typically
/// one side of a `Replace` hunk), so wrapping them again would be a no-op
/// at best.
pub trait DiffHost {
    /// Line-level hunks between `old` and `new`, `Equal` runs dropped.
    fn diff_lines(&self, old: &str, new: &str) -> Vec<ChangeHunk>;

    /// As [`diff_lines`](DiffHost::diff_lines), diffing `ref_text` against
    /// `bid`'s live (dirty) in-memory text, which avoids materializing the whole
    /// buffer as a Steel string on every debounced call. `None` for an
    /// unknown/stale `bid`, the single liveness check this call needs
    /// (looking up the buffer's text also answers "does it exist"), so the
    /// Steel boundary maps `None` straight to an error rather than checking
    /// liveness a second time first.
    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<Vec<ChangeHunk>>;

    /// What separates `bid`'s live text from revision `revision` of its undo
    /// history: the revision's text is the old side, the live text the new.
    /// `Err` for a revision the history has no record of. Hunks carry the
    /// word spans the history's own changesets give them.
    fn revision_diff(&self, bid: BufferId, revision: usize) -> Result<Vec<ChangeHunk>, String>;

    /// Word-level hunks between `old` and `new`, `Equal` runs dropped. The
    /// returned `bool` mirrors `WordDiff::deadline_hit()`: `true` means the
    /// underlying Myers pass could not finish within its deadline and
    /// returned a coarse (Replace-all) result. Unlike line-diff's Myers
    /// fallback (still a correct partition), a word-diff timeout result
    /// should be treated as a fallback, not a precise diff (skip word
    /// highlighting, fall back to a whole-line scope).
    fn diff_words(&self, old: &str, new: &str) -> (Vec<WordDiffHunk>, bool);
}
