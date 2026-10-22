//! Buffer/pane enumeration, reads, lifecycle, and viewport geometry.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;

use crate::types::PaneHandle;

/// One revision of a buffer's undo history, as `(buffer-undo-tree pane)`
/// reports it. `id` and `parent` are the raw integers Steel sees; the only
/// way back to a revision is [`EditHost::goto_revision`], which validates the
/// number against the buffer's history.
///
/// [`EditHost::goto_revision`]: super::EditHost::goto_revision
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoNode {
    pub id: usize,
    /// `None` only for the root.
    pub parent: Option<usize>,
    /// Wall-clock age in whole seconds when the tree was read.
    pub age_secs: u64,
    /// This revision matches the buffer's current text.
    pub current: bool,
    /// This revision is the one last saved (or first opened).
    pub saved: bool,
}

/// Which form of a buffer's display path [`BufferHost::buffer_display_path`]
/// returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayPathForm {
    /// Absolute, `~`-collapsed.
    Absolute,
    /// Relative to the editor's working directory when the file is inside
    /// it, [`Absolute`](Self::Absolute) otherwise. The path as the user typed
    /// it is tried first, then its symlink-resolved form.
    CwdRelative,
}

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
    /// Fully display-ready path string in `form` (UNC-stripped; absolute,
    /// lexically normalized and `~`-collapsed unless relative): print
    /// verbatim. `None` for scratch/synthetic buffers, same as `buffer_path`.
    fn buffer_display_path(&self, id: BufferId, form: DisplayPathForm) -> Option<String>;
    fn buffer_display_name(&self, id: BufferId) -> Option<String>;
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool>;
    /// Language stored on the buffer (not accounting for pending `set-buffer-option!`'s `"language"`).
    fn buffer_stored_language(&self, id: BufferId) -> Option<String>;

    // ── Working directory and search path ───────────────────────────────────
    /// The editor's working directory: the base for relative paths and the
    /// default cwd of every spawned child. Backs `(cwd)`.
    fn cwd(&self) -> PathBuf;
    /// The `PATH` every spawned child gets and `which` searches
    /// (`Dirs::search_path`); `None` when unset.
    fn search_path(&self) -> Option<OsString>;
    /// Moves the editor's working directory to `path` (a relative `path` joins
    /// onto the current one) and returns the new directory. The same
    /// operation as `:cd`. `Err` when `path` is not a directory.
    fn set_cwd(&mut self, path: &Path) -> Result<PathBuf, String>;

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    /// Open a file at `path`, deduplicating if already open.
    /// Returns the `BufferId` (new or existing).
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String>;
    /// Close `id`. `Err` when `id` does not name an open buffer, or while a
    /// pipe runs in it.
    fn close_buffer(&mut self, id: BufferId) -> Result<(), String>;
    /// Switch the focused pane to `target`, recording a jump entry.
    /// Redirect `pane`'s own pane to `target`, recording a jump entry.
    /// Kind-B: raises if `pane` carries no pane, a closed one, or one that
    /// no longer shows `pane`'s buffer.
    fn switch_to_buffer(&mut self, pane: PaneHandle, target: BufferId) -> Result<(), String>;
    /// Show `text` on `pane`'s own pane in the read-only buffer labelled
    /// `[name]`, opening it on first use and replacing its text after.
    /// Kind-B, like [`Self::switch_to_buffer`]. `Err` for an empty name or
    /// one the editor's own views use.
    fn show_read_only_buffer(
        &mut self,
        pane: PaneHandle,
        name: &str,
        text: &str,
    ) -> Result<(), String>;

    /// Steel-side staleness token for buffer `id` (its text generation, bumped by
    /// every mutation), or `None` if `id` is unknown. Not LSP-specific (any
    /// script can compare a saved value against a live read), but the LSP
    /// bridge's own `#:allow-stale` staleness check is what motivated it.
    fn buffer_generation(&self, id: BufferId) -> Option<u64>;

    /// `(buffer-undo-tree pane)`: every revision in `id`'s undo history, in
    /// id order, which is also parents-before-children. `None` if `id` is
    /// unknown.
    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<UndoNode>>;

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
    /// Backs the Steel `(line->char-offset bid line)` builtin, the inverse
    /// direction of `char-offset->line`.
    fn line_to_char_offset(
        &self,
        id: BufferId,
        line: hume_rope::line::ContentLine,
    ) -> Option<usize>;

    /// The content-domain line range currently visible in `pane`'s own pane.
    /// Backs the Steel `(viewport-range pane)` builtin, which unwraps the
    /// range to a `(hash 'start-line first 'end-line end)` at the Steel boundary
    /// (0-based, end-exclusive). Pane geometry, not LSP state, so it doesn't need
    /// an attached server. Kind-B: raises if `pane` carries no pane, a
    /// closed one, or one that no longer shows `pane`'s buffer. A pane on a
    /// background tab answers too: its size follows the terminal, but its
    /// scroll position stays where it was until that tab is focused again.
    /// Takes `&mut self` so the host can first lay its panes out against a
    /// band opened or closed since its last frame.
    fn viewport_range(
        &mut self,
        pane: PaneHandle,
    ) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>, String>;
}
