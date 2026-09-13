//! Buffer/pane enumeration, reads, lifecycle, and viewport geometry —
//! moved out of `host.rs`'s per-capability split.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::{BufferId, PaneId};

/// Buffer/pane enumeration, reads, lifecycle, and viewport geometry —
/// accessed through [`EditorHost::buffers`](super::EditorHost::buffers).
pub trait BufferHost {
    /// All open buffer ids in open-order.
    fn buffer_ids(&self) -> Vec<BufferId>;
    /// Every open pane id, across every tab — including panes in inactive
    /// tabs, not just the active tab's own.
    fn pane_ids(&self) -> Vec<PaneId>;

    // ── Buffer reads (None ⇒ unknown/stale id) ──────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool;
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf>;
    /// Fully display-ready path string (absolutized, lexically normalized,
    /// UNC-stripped, `~`-collapsed) — print verbatim. `None` for scratch/synthetic
    /// buffers, same as `buffer_path`.
    fn buffer_display_path(&self, id: BufferId) -> Option<String>;
    fn buffer_display_name(&self, id: BufferId) -> Option<String>;
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool>;
    /// Language stored on the buffer (not accounting for pending `set-buffer-language!`).
    fn buffer_stored_language(&self, id: BufferId) -> Option<String>;

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    /// Open a file at `path`, deduplicating if already open.
    /// Returns the `BufferId` (new or existing).
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String>;
    /// Close `id`.  Returns the new live focused buffer id, or `Err` when `id`
    /// does not name an open buffer.
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String>;
    /// Switch the focused pane to `target`, recording a jump entry.
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String>;

    /// Steel-side staleness token for buffer `id` (its `text_gen`, bumped by
    /// every mutation) — `None` if `id` is unknown. Not LSP-specific (any
    /// script can compare a saved value against a live read), but the LSP
    /// bridge's own `#:allow-stale` staleness check is what motivated it.
    fn buffer_generation(&self, id: BufferId) -> Option<u64>;

    /// `(buffer-text bid)` — the buffer's full live (dirty) in-memory
    /// content, always ending with the structural trailing `\n`. `None` if
    /// `id` is unknown.
    fn buffer_text(&self, id: BufferId) -> Option<String>;

    /// Number of *content* lines in `id`'s live text — every HUME buffer
    /// ends with a structural `\n`, which ropey counts as one extra empty
    /// line (see [`hume_engine::pipeline`] invariants); this excludes that
    /// phantom line, matching what the statusline and `:w` report. `None`
    /// if `id` is unknown.
    fn buffer_line_count(&self, id: BufferId) -> Option<usize>;

    /// Content lines `range` of `id`'s live text, each with its trailing line
    /// break stripped. `range` is caller-validated against
    /// [`buffer_line_count`](Self::buffer_line_count) before this is called —
    /// the `ContentLine` bound itself carries that validation, so this call
    /// does not re-clamp or re-check it, and a `range` built any other way
    /// (not checked against this buffer's own line count) is a caller bug:
    /// implementations may panic rather than return `None` (the editor
    /// implementation does, via the underlying rope's line lookup). `None` if
    /// `id` is unknown.
    fn buffer_lines(
        &self,
        id: BufferId,
        range: hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
    ) -> Option<Vec<String>>;

    /// The char offset where content `line` starts in `id`'s live text.
    /// `line` is caller-validated against
    /// [`buffer_line_count`](Self::buffer_line_count) before this is called —
    /// same contract as [`buffer_lines`](Self::buffer_lines): this call does
    /// not re-check it, and a `line` built any other way is a caller bug (the
    /// editor implementation panics, via the underlying rope's line lookup).
    /// `None` if `id` is unknown.
    ///
    /// Backs the Steel `(line->offset bid line)` builtin — the inverse
    /// direction of `char-index->line`, but not a drop-in inverse of it:
    /// `char-index->line` is 1-indexed and reads the focused buffer, this is
    /// 0-indexed and takes an explicit `id`.
    fn line_to_offset(&self, id: BufferId, line: hume_rope::line::ContentLine) -> Option<usize>;

    /// The content-domain line range currently visible for `id` (the focused
    /// pane's if shown there, else the first active-tab pane showing it), or
    /// `None` if `id` isn't shown in a pane on the active tab — including a
    /// buffer visible only in a background tab. Backs the Steel
    /// `(viewport-range bid)` builtin, which unwraps the range to a `(first
    /// . end)` integer pair at the Steel boundary — 0-based, end-exclusive.
    /// Pane geometry, not LSP state — doesn't need an attached server.
    fn viewport_range(
        &self,
        id: BufferId,
    ) -> Option<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>>;
}
