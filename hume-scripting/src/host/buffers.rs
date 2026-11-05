//! Buffer/pane enumeration, reads, lifecycle, and viewport geometry.

use std::ffi::OsString;

use hume_core::{BufferId, OnUnsaved, PaneHandle};
use hume_platform::path::AbsolutePath;

use super::ShownHandle;
use crate::BufferVisibility;

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

/// A buffer's language as the editor stores it, and the origin of the write
/// that asserted it (empty when detection chose it or there is none).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredLanguage {
    pub name: Option<String>,
    pub by: hume_core::Claim<hume_core::BufferLanguageTie>,
}

/// Buffer/pane enumeration, reads, lifecycle, and viewport geometry,
/// accessed through [`EditorHost::buffers`](super::EditorHost::buffers).
pub trait BufferHost {
    /// All open buffer ids in open-order.
    fn buffer_ids(&self) -> Vec<BufferId>;
    /// Every open pane in every tab, inactive tabs included, paired with the
    /// buffer each shows. Backs `(panes)`.
    fn panes(&self) -> Vec<PaneHandle>;

    /// `(focused-pane)`: the pane focused right now, paired with the buffer
    /// it shows. For code with no pane of its own (a global option-change
    /// hook, a focus-following timer) and for an async callback's staleness
    /// check. Never a substitute for a command's own `pane` parameter, or
    /// for capturing the buffer a request or edit is about.
    fn focused_pane(&self) -> PaneHandle;

    /// Every pane showing buffer `id`: the focused pane first if it shows
    /// it, then the rest of the active tab, then other tabs. Backs
    /// `(buffer-panes pane)`, so the caller, not a builtin, picks a pane on
    /// the buffer.
    fn buffer_panes(&self, id: BufferId) -> Vec<PaneHandle>;

    /// Whether `pane` names a pane that still exists and still shows
    /// `pane`'s buffer; `false` when `pane` carries no pane. Never raises.
    /// Backs `(pane-live? pane)`.
    fn pane_live(&self, pane: PaneHandle) -> bool;

    // ── Buffer reads (None ⇒ unknown/stale id) ──────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool;
    fn buffer_path(&self, id: BufferId) -> Option<AbsolutePath>;
    /// Fully display-ready path string: print verbatim. Relative to
    /// `relative_to` when the file is inside it, the path as the user typed
    /// it tried first, then its symlink-resolved form. Otherwise absolute,
    /// UNC-stripped, `..`-free and `~`-collapsed. `None` for
    /// scratch/synthetic buffers, same as `buffer_path`.
    fn buffer_display_path(
        &self,
        id: BufferId,
        relative_to: Option<&AbsolutePath>,
    ) -> Option<String>;
    fn buffer_display_name(&self, id: BufferId) -> Option<String>;
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool>;
    /// Language stored on the buffer, not counting a pending
    /// `set-buffer-option!` `"language"`. The default for an unknown id.
    fn buffer_stored_language(&self, id: BufferId) -> StoredLanguage;
    /// Whether `visibility` sees buffer `id` by the language the buffer
    /// has now; `false` for an unknown id.
    fn buffer_visible_to(&self, id: BufferId, visibility: &BufferVisibility) -> bool;

    // ── Working directory and search path ───────────────────────────────────
    /// The editor's working directory: the base for relative paths and the
    /// default cwd of every spawned child. Backs `(cwd)`.
    fn cwd(&self) -> AbsolutePath;
    /// The `PATH` every spawned child gets and `which` searches
    /// (`Dirs::search_path`); `None` when unset.
    fn search_path(&self) -> Option<OsString>;
    /// Moves the editor's working directory to `path` and returns the new
    /// directory. The same operation as `:cd`. `Err` when `path` is not a
    /// directory.
    fn set_cwd(&mut self, path: &AbsolutePath) -> Result<AbsolutePath, String>;

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    /// Open a file at `path`, deduplicating if already open.
    /// Returns the `BufferId` (new or existing).
    fn open_buffer(&mut self, path: &AbsolutePath) -> Result<BufferId, String>;
    /// Close `id`. `Err` when `id` does not name an open buffer, when it has
    /// unsaved changes and `on_unsaved` is [`OnUnsaved::Refuse`], or while a
    /// pipe runs in it.
    fn close_buffer(&mut self, id: BufferId, on_unsaved: OnUnsaved) -> Result<(), String>;
    /// Redirect `pane`'s pane to `target`, recording a jump entry, and
    /// return the handle naming that pane and `target`.
    fn switch_to_buffer(
        &mut self,
        pane: ShownHandle,
        target: BufferId,
    ) -> Result<PaneHandle, String>;
    /// Show `text` on `pane`'s own pane in the read-only buffer labelled
    /// `[name]`, opening it on first use and replacing its text after, and
    /// return the handle naming that pane and the view. `Err` for an empty
    /// name or one the editor's own views use.
    fn show_read_only_buffer(
        &mut self,
        pane: ShownHandle,
        name: &str,
        text: &str,
    ) -> Result<PaneHandle, String>;

    /// Buffer `id`'s text generation, bumped by every mutation, or `None` if
    /// `id` is unknown. A script compares a saved value against a live read
    /// to detect staleness.
    fn buffer_generation(&self, id: BufferId) -> Option<u64>;

    /// `(buffer-search pane pattern)`: the first match in `id`'s text of
    /// `pattern`, read as input at the `/` prompt (its flags and smart case
    /// included), as a char range, `None` inside the `Ok` when nothing
    /// matches. `Err` for an empty or invalid pattern. Outer `None` if `id` is
    /// unknown.
    fn buffer_search(
        &self,
        id: BufferId,
        pattern: &str,
    ) -> Option<Result<Option<std::ops::Range<usize>>, String>>;

    /// `(word-search-pattern pane word)`: the search input `*` sets for
    /// `word` in `id`'s buffer, under that buffer's `word-chars`. `None` if
    /// `id` is unknown.
    fn word_search_pattern(&self, id: BufferId, word: &str) -> Option<String>;

    /// `(buffer-undo-tree pane)`: every revision in `id`'s undo history, in
    /// id order, which is also parents-before-children. `None` if `id` is
    /// unknown.
    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<UndoNode>>;

    /// `(buffer-text bid)`: the buffer's full live (dirty) in-memory
    /// content, always ending with the structural trailing `\n`, or with
    /// `range` its chars `start..end`. `None` if `id` is unknown; `Err`
    /// naming the offending end when `range` does not fit the text.
    fn buffer_text(
        &self,
        id: BufferId,
        range: Option<(usize, usize)>,
    ) -> Option<Result<String, String>>;

    /// `(buffer-line-count bid)`: the number of content lines in `id`'s live
    /// text, without the empty line ropey counts after the structural `\n`,
    /// as the statusline and `:w` report it. `None` if `id` is unknown.
    fn buffer_line_count(&self, id: BufferId) -> Option<usize>;

    /// `(buffer-char-count bid)`: length of `id`'s live text in chars, the
    /// structural trailing `\n` included, so the largest offset
    /// `char-offset->line` accepts. `None` if `id` is unknown.
    fn buffer_char_count(&self, id: BufferId) -> Option<usize>;

    /// Content lines `range` of `id`'s live text, each with its trailing line
    /// break stripped. The caller checks `range` against
    /// [`buffer_line_count`](Self::buffer_line_count); this does not, and an
    /// implementation may panic on a `range` built any other way (the
    /// editor's does). `None` if `id` is unknown.
    fn buffer_lines(
        &self,
        id: BufferId,
        range: hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
    ) -> Option<Vec<String>>;

    /// The char offset where content `line` starts in `id`'s live text, for
    /// `(line->char-offset bid line)`. `line` is caller-checked as in
    /// [`buffer_lines`](Self::buffer_lines), with the same panic on one
    /// built any other way. `None` if `id` is unknown.
    fn line_to_char_offset(
        &self,
        id: BufferId,
        line: hume_rope::line::ContentLine,
    ) -> Option<usize>;

    /// The content lines visible in `pane`'s pane, for
    /// `(viewport-range pane)` (0-based, end-exclusive). A pane on a
    /// background tab answers too: its size follows the terminal, but its
    /// scroll position stays where it was until that tab is focused again.
    /// `&mut self` so the host can first lay its panes out against a band
    /// opened or closed since its last frame.
    fn viewport_range(
        &mut self,
        pane: ShownHandle,
    ) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>, String>;
}
