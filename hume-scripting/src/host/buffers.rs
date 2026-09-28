//! Buffer/pane enumeration, reads, lifecycle, and viewport geometry.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;

use crate::types::PaneHandle;

/// Buffer/pane enumeration, reads, lifecycle, and viewport geometry,
/// accessed through [`EditorHost::buffers`](super::EditorHost::buffers).
pub trait BufferHost {
    /// All open buffer ids in open-order.
    fn buffer_ids(&self) -> Vec<BufferId>;
    /// Every open pane, across every tab (including panes in inactive
    /// tabs, not just the active tab's own), paired with the buffer each
    /// shows. Backs `(panes)`.
    fn panes(&self) -> Vec<PaneHandle>;

    /// `(focused-pane)` → the pane focused *right now*, paired with the
    /// buffer it shows. Reads live editor state on every call. Meant for
    /// code with no pane of its own to act on (a global option-change hook,
    /// a focus-following timer) and for staleness checks in an async
    /// callback that captured a different pane at request time. Never a
    /// substitute for a command's own `pane` parameter, or for capturing the
    /// buffer a request or edit is actually *about*.
    fn focused_pane(&self) -> PaneHandle;

    /// `pane`'s own pane, still showing `pane`'s own buffer, else every
    /// other pane showing that buffer: focused pane first, then the rest of
    /// the active tab, then other tabs. Backs `(buffer-panes pane)`, the
    /// explicit alternative to the pane guess this design removes: `(car
    /// (buffer-panes pane))` reproduces it, chosen by the caller rather than
    /// applied silently.
    fn buffer_panes(&self, pane: PaneHandle) -> Vec<PaneHandle>;

    /// The kind-A gate: requires `pane` to name the currently *focused*
    /// pane, for a builtin whose action (open a popup/menu/drawer/picker,
    /// jump to the LSP status view) is meaningless anywhere but the pane the
    /// user is looking at. `Err` (never a silent default) when `pane`
    /// carries no pane, a closed one, one that no longer shows its buffer,
    /// or one that simply isn't focused.
    fn require_focused_pane(&self, pane: PaneHandle) -> Result<(), String>;

    /// `#t` if `pane` names a pane that still exists and still shows
    /// `pane`'s own buffer, `#f` otherwise (including when `pane` carries no
    /// pane component at all). Never raises; the pane-aware sibling of
    /// `buffer_exists`/`(buffer-live? pane)`. The idiom for a debounced or
    /// otherwise async continuation whose captured `pane` may have closed,
    /// or been repointed at another buffer, by the time it fires: check
    /// this first, rather than discovering the fact via a raise from
    /// whatever kind-B builtin the continuation was actually going to call.
    fn pane_live(&self, pane: PaneHandle) -> bool;

    // ── Buffer reads (None ⇒ unknown/stale id) ──────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool;
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf>;
    /// Fully display-ready path string (absolutized, lexically normalized,
    /// UNC-stripped, `~`-collapsed): print verbatim. `None` for scratch/synthetic
    /// buffers, same as `buffer_path`.
    fn buffer_display_path(&self, id: BufferId) -> Option<String>;
    fn buffer_display_name(&self, id: BufferId) -> Option<String>;
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool>;
    /// Language stored on the buffer (not accounting for pending `set-buffer-option!`'s `"language"`).
    fn buffer_stored_language(&self, id: BufferId) -> Option<String>;

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    /// Open a file at `path`, deduplicating if already open.
    /// Returns the `BufferId` (new or existing).
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String>;
    /// Close `id`. `Err` when `id` does not name an open buffer.
    fn close_buffer(&mut self, id: BufferId) -> Result<(), String>;
    /// Switch the focused pane to `target`, recording a jump entry.
    /// Redirect `pane`'s own pane to `target`, recording a jump entry.
    /// Kind-B: raises if `pane` carries no pane, a closed one, or one that
    /// no longer shows `pane`'s buffer.
    fn switch_to_buffer(&mut self, pane: PaneHandle, target: BufferId) -> Result<(), String>;

    /// Steel-side staleness token for buffer `id` (its `text_gen`, bumped by
    /// every mutation), or `None` if `id` is unknown. Not LSP-specific (any
    /// script can compare a saved value against a live read), but the LSP
    /// bridge's own `#:allow-stale` staleness check is what motivated it.
    fn buffer_generation(&self, id: BufferId) -> Option<u64>;

    /// `(buffer-text bid)`: the buffer's full live (dirty) in-memory
    /// content, always ending with the structural trailing `\n`. `None` if
    /// `id` is unknown.
    fn buffer_text(&self, id: BufferId) -> Option<String>;

    /// `(buffer-line-count bid)`: number of *content* lines in `id`'s live
    /// text. Every HUME buffer ends with a structural `\n`, which ropey
    /// counts as one extra empty line (see [`hume_engine::pipeline`]
    /// invariants); this excludes that phantom line, matching what the
    /// statusline and `:w` report. `None` if `id` is unknown.
    fn buffer_line_count(&self, id: BufferId) -> Option<usize>;

    /// Content lines `range` of `id`'s live text, each with its trailing line
    /// break stripped. `range` is caller-validated against
    /// [`buffer_line_count`](Self::buffer_line_count) before this is called;
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
    /// [`buffer_line_count`](Self::buffer_line_count) before this is called,
    /// same contract as [`buffer_lines`](Self::buffer_lines): this call does
    /// not re-check it, and a `line` built any other way is a caller bug (the
    /// editor implementation panics, via the underlying rope's line lookup).
    /// `None` if `id` is unknown.
    ///
    /// Backs the Steel `(line->offset bid line)` builtin, the inverse
    /// direction of `offset->line`.
    fn line_to_offset(&self, id: BufferId, line: hume_rope::line::ContentLine) -> Option<usize>;

    /// The content-domain line range currently visible in `pane`'s own pane.
    /// Backs the Steel `(viewport-range pane)` builtin, which unwraps the
    /// range to a `(first . end)` integer pair at the Steel boundary
    /// (0-based, end-exclusive). Pane geometry, not LSP state, so it doesn't need
    /// an attached server. Kind-B: raises if `pane` carries no pane, a
    /// closed one, one that no longer shows `pane`'s buffer, or one on a
    /// background tab (its geometry isn't kept in sync with the terminal).
    fn viewport_range(
        &self,
        pane: PaneHandle,
    ) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>, String>;
}
