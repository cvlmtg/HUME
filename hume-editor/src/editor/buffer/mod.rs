use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::search::{SearchMatches, SearchPattern};
use crate::editor::edit_session::EditGroup;
use crate::editor::position_stores::PositionStores;
use crate::editor::settings::BufferOverrides;
use hume_editing::changeset::{ChangeSet, changesets_from_line_diff};
use hume_editing::edit::{Edited, TextChange};
use hume_editing::history::{History, RevisionId};
use hume_editing::selection::{RecordedSelections, SelectionSet};
use hume_editing::state::EditState;
use hume_editing::text::{BufferText, TextVersion};
use hume_editing::transaction::Transaction;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_platform::io::FileMeta;

mod disk;
// Sibling buffer submodules (`file_open::enter_buffer`,
// `Buffer::disk_state`'s field type) reach `disk::{DiskCheckTrigger,
// DiskState}` via `super::disk::` instead, staying inside the `buffer`
// module tree. `DiskCheckTrigger` is re-exported unconditionally because
// `commands::typed_buffer::typed_checktime`, outside the tree, needs
// `DiskCheckTrigger::Explicit` to name its own trigger; `DiskState` stays
// `#[cfg(test)]`-only since only test code (a different module tree
// entirely, passing a trigger to `check_buffer_disk_state` directly or
// injecting a `DiskState` value to test the state machine without depending
// on filesystem mtime precision) needs it outside `buffer` itself.
pub(in crate::editor) use disk::DiskCheckTrigger;
#[cfg(test)]
pub(in crate::editor) use disk::DiskState;
mod file_open;
pub(in crate::editor) mod lifecycle;
pub(in crate::editor) mod store;
use hume_treesitter::registry::LanguageId;
use hume_treesitter::syntax::Syntax;

// ── Buffer ────────────────────────────────────────────────────────────────────

/// What one composed undo/redo walk hands back. See
/// [`Buffer::apply_transactions`]'s doc for each part's contract. Named
/// once here so `undo_n`/`redo_n`, `doc_ops::apply_doc_history_walk`, and
/// `commands::edit::history_step` all spell the same shape.
pub(in crate::editor) type HistoryWalkResult = Option<(SelectionSet, ChangeSet, usize)>;

/// Content-only document: text, undo history, search state, and per-buffer overrides.
///
/// `Buffer` is the SSOT for everything intrinsic to an open file and shared
/// across all panes viewing it. It does **not** own:
/// - selections (per-(pane, buffer), live on `PaneBufferState`)
/// - viewport / scroll (per-pane, live on engine `Pane`)
/// - per-pane search cursor (live on `PaneBufferState`)
/// - edit groups / insert sessions (per-(pane, buffer), live on `PaneBufferState`)
///
/// ## Edit API
///
/// All text mutations funnel through [`Self::install`], which carries every
/// position stored against the text through the change: each mutator takes
/// the [`PositionStores`] for that reason, so text cannot change without
/// them. The three edit entry points (`apply_edit`, `apply_edit_grouped`,
/// `apply_edit_regrouped`) each return the acting pane's post-edit
/// `SelectionSet` + the `ChangeSet` it applied, and handle undo bookkeeping
/// internally.
pub(crate) struct Buffer {
    text: BufferText,
    history: History,
    /// The revision at which the buffer was last saved (or first opened).
    /// `None` means the saved state was overwritten by an `undo-levels`
    /// promotion and no longer exists anywhere in the tree: the buffer is
    /// dirty until the next save.
    saved_revision: Option<RevisionId>,
    /// Canonical file path (after symlink resolution). `None` for scratch buffers.
    pub(super) path: Option<PathBuf>,
    /// Fully display-ready path string (absolutized, lexically normalized,
    /// UNC-stripped, `~`-collapsed), the single form shown to the user
    /// everywhere a buffer path appears. Consumers print it verbatim; the
    /// only allowed runtime-time exception is statusline width-shortening.
    /// Always `Some` when `path` is `Some`: `set_path` derives it
    /// structurally (see `Buffer::set_path`); callers that resolved a
    /// user-typed path overwrite it afterwards with `set_display_path` for
    /// the typed-derived form. `None` for scratch/synthetic buffers.
    pub(super) display_path: Option<String>,
    /// File metadata captured at open/save time (permissions, uid/gid).
    /// `None` for scratch buffers; populated after a successful save.
    pub(in crate::editor) file_meta: Option<FileMeta>,
    /// Active search pattern shared by all panes viewing this buffer.
    /// `None` when no search is active. A present `SearchPattern` is always
    /// fully-valid. Invalid regexes leave this as `None`.
    pub(in crate::editor) search_pattern: Option<SearchPattern>,
    /// Cached match list for `search_pattern`. Invalidated by revision change
    /// or pattern change; rebuilt lazily by `update_buffer_matches`.
    pub(in crate::editor) search_matches: SearchMatches,
    /// Per-buffer setting overrides. `None` fields inherit from
    /// [`crate::editor::settings::EditorSettings`].
    pub(in crate::editor) overrides: BufferOverrides,
    /// Detected or explicitly set language identity (e.g. `rust`, `json`).
    /// `None` for unrecognised filetypes and scratch buffers.
    pub(crate) language: Option<LanguageId>,
    /// `true` when `language` was written by `:set buffer language=` or Steel's
    /// `set-buffer-option!`'s `"language"`, rather than by detection. `:reload-config`'s
    /// reset reads this (before clearing it) to restore the user's own
    /// assertion across the reload instead of letting re-detection silently
    /// pick something else. See `clear_languages_all`.
    pub(in crate::editor) language_explicit: bool,
    /// The text version most recently announced as an `on-text-changed`
    /// event. `Buffer` cannot reach the event queue (it holds no
    /// `EditorState`), so the hook is raised by diffing this against the
    /// text's version at a drain observation point (see
    /// `BufferStore::take_text_changed`) rather than at `install` itself.
    pub(in crate::editor) announced_version: TextVersion,
    /// Per-buffer tree-sitter syntax attachment: grammar identity, committed
    /// parse layers, generation bookkeeping, and in-flight state, all in one
    /// place. `None` when no grammar is attached or the buffer exceeds
    /// `syntax-highlight-max-bytes`.
    pub(in crate::editor) syntax: Option<Syntax>,
    /// When `true`, all forward text mutations are blocked at the `doc_ops`
    /// layer. Entering Insert mode is also refused. Read-only is orthogonal to
    /// language/syntax: a read-only buffer may still be highlighted.
    pub(in crate::editor) read_only: bool,
    /// Display name used for synthetic, path-less view buffers (e.g. `"[messages]"`).
    /// Shown in the statusline and `:ls` instead of `*scratch*`.
    pub(in crate::editor) label: Option<String>,
    /// The LSP server this buffer is attached to, if any. Set by
    /// `Editor::lsp_attach_buffer`; `None` for unnamed buffers, buffers with
    /// no registered server, before the open-time attach attempt runs, or
    /// after the attached server is detached.
    pub(in crate::editor) lsp_server: Option<hume_lsp::backend::ServerId>,
    /// BufferText mutations queued for `textDocument/didChange` conversion, in
    /// order. Recorded at the same chokepoint as tree-sitter's pending
    /// edits (`doc_ops.rs`'s five apply functions); drained by the LSP
    /// per-frame flush. Always empty when `lsp_server` is `None`.
    pub(in crate::editor) lsp_pending: Vec<super::lsp::sync::LspPendingChange>,
    /// True from chokepoint open (`lifecycle::open_buffer_and_notify`, or
    /// `lifecycle::close_buffer_and_notify`'s own `queue_open_announcement`
    /// call for the fresh scratch buffer a last-buffer close allocates)
    /// until `Editor::detect_pending_languages` fires this buffer's
    /// `OnBufferOpen`. Read by `lifecycle::close_buffer_and_notify`: a still-
    /// pending buffer (opened and closed before that drain ran, e.g. within
    /// one Steel eval) announced no open, so it must announce no close
    /// either. The startup buffer, built inline by `Editor::open` (which
    /// can't call the chokepoint, since it's what bootstraps the very
    /// `EngineView`/pane-state maps the chokepoint needs), is the one buffer
    /// that defaults to `false`: its close always announces.
    pub(in crate::editor) open_hook_pending: bool,
    /// The buffer's disk state as of the last check, set by
    /// `Editor::check_buffer_disk_state`, cleared to `InSync` by a reload or
    /// a successful write. Always `InSync` for scratch/synthetic buffers,
    /// which the check skips.
    pub(in crate::editor) disk_state: disk::DiskState,
}

/// How a new text relates to the one it replaces, for [`Buffer::install`].
enum Change<'a> {
    /// The new text is the old one with this change applied.
    Edit(&'a ChangeSet),
    /// The new text replaces the old one wholesale.
    Replace,
}

impl Buffer {
    /// Display name used for buffers that have no backing file.
    pub(in crate::editor) const SCRATCH_BUFFER_NAME: &'static str = "*scratch*";

    /// Create a new buffer holding `text` with a cursor on its first cluster.
    pub(crate) fn at_start(text: BufferText) -> Self {
        Self::new(EditState::at_text_start(text))
    }

    /// Create a new buffer from its text and initial selections.
    ///
    /// The selections are stored in the history root so `initial_sels()` can
    /// recover them for seeding `PaneBufferState` on first open or `:e!` reload.
    pub(crate) fn new(initial: EditState) -> Self {
        let history = History::new(initial.recorded(), initial.text().len_chars());
        let text = initial.text().clone();
        let saved_revision = Some(history.current_id());
        let announced_version = text.version();
        Self {
            text,
            history,
            saved_revision,
            path: None,
            display_path: None,
            file_meta: None,
            search_pattern: None,
            search_matches: SearchMatches::default(),
            overrides: BufferOverrides::default(),
            language: None,
            language_explicit: false,
            announced_version,
            syntax: None,
            read_only: false,
            label: None,
            lsp_server: None,
            lsp_pending: Vec::new(),
            open_hook_pending: false,
            disk_state: disk::DiskState::InSync,
        }
    }

    /// Load a file from disk, returning a ready-to-use `Buffer`.
    ///
    /// Sets `path` and `file_meta` from the resolved filesystem metadata;
    /// `set_path` derives a canonical-path-based `display_path` alongside it.
    /// Callers that resolved a user-typed path (`resolve_open_path`, save-as,
    /// ...) overwrite it with the typed-derived form afterwards, via
    /// `set_display_path`; this default only surfaces for opens with no typed
    /// path (Steel `open-buffer!`, `:tutor`, LSP goto). `search_pattern` and
    /// `search_matches` are left at their defaults (no active search).
    pub(in crate::editor::buffer) fn from_file(path: &Path) -> io::Result<Self> {
        let (content, meta) = hume_platform::io::read_file(path)?;
        let text = BufferText::from(content.as_str());
        let mut buf = Self::at_start(text);
        buf.set_path(Some(meta.resolved_path().to_path_buf()));
        buf.file_meta = Some(meta);
        Ok(buf)
    }

    /// Empty buffer bound to a path that doesn't exist on disk yet: `:e` on
    /// a missing file, matching Vim's `:e newfile` semantics: `:w` creates it
    /// (see `is_new_file`, `write_buffer_by_id`'s `file_meta.is_none()`
    /// branch). `file_meta` stays `None` until that first write; `path` is
    /// set so the buffer participates in `find_by_path` dedup and displays
    /// its intended name.
    pub(in crate::editor::buffer) fn new_file(path: PathBuf) -> Self {
        let mut buf = Self::at_start(BufferText::empty());
        buf.set_path(Some(path));
        buf
    }

    /// `true` for a buffer bound to a path with no backing file yet, opened
    /// via [`Self::new_file`], not yet written. `path.is_some()` alone isn't
    /// enough (a normal file has that too); `file_meta` is the SSOT for
    /// "has this buffer ever touched disk". See the field doc.
    pub(in crate::editor) fn is_new_file(&self) -> bool {
        self.path.is_some() && self.file_meta.is_none()
    }

    /// Load a file from disk, or open an empty [`Self::new_file`] buffer
    /// bound to `path` if it doesn't exist yet (Vim's `:e newfile`
    /// semantics: `:w` creates it). `cwd` feeds `Editor::resolve_buffer_path`
    /// on the missing-file branch, so the buffer's identity matches whatever
    /// form the caller resolved `path` to.
    ///
    /// The single decision point for "is a missing path openable", shared by
    /// every open chokepoint, so they can't diverge on tolerance.
    ///
    /// A path with no basename (`/`, `..`) still errors: `Buffer::set_path`
    /// would panic on it in debug.
    pub(in crate::editor) fn from_file_or_new(path: &Path, cwd: &Path) -> io::Result<Self> {
        match Self::from_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let resolved = crate::editor::Editor::resolve_buffer_path(path, cwd);
                if resolved.file_name().is_none() {
                    return Err(e);
                }
                Ok(Self::new_file(resolved))
            }
            other => other,
        }
    }

    /// Empty scratch buffer (single structural `\n`, no path, default overrides).
    ///
    /// Used when closing the last buffer to keep the "always ≥1 buffer open"
    /// invariant without leaving the editor in an invalid state.
    pub(in crate::editor) fn scratch() -> Self {
        Self::at_start(BufferText::empty())
    }

    /// Create a read-only view buffer from in-memory content.
    ///
    /// Used for `:messages`, `:ls`, and `:plugin-status`. The buffer has no
    /// backing file, no language detection, and blocks all user edits.
    pub(in crate::editor) fn read_only_view(text: BufferText, label: String) -> Self {
        let mut buf = Self::at_start(text);
        buf.read_only = true;
        buf.label = Some(label);
        buf
    }

    /// Replace the content of a read-only view buffer with new text.
    ///
    /// Resets history to a clean root and clears search state so the refreshed
    /// buffer is non-dirty and has no stale match data. This is a system
    /// refresh: it intentionally bypasses the `read_only` guard in `doc_ops`.
    pub(in crate::editor) fn set_view_content(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        content: &str,
    ) {
        let text = self.text.replaced_with(content);
        let text_len = text.len_chars();
        let undo_levels = self.history.undo_levels();
        self.history = History::new(EditState::at_text_start(text.clone()).recorded(), text_len);
        self.history.set_undo_levels(undo_levels);
        self.saved_revision = Some(self.history.current_id());
        self.search_pattern = None;
        self.search_matches = SearchMatches::default();
        self.install(id, stores, text, Change::Replace);
    }

    /// `true` when the buffer blocks user edits.
    pub(crate) fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// `true` for in-memory view buffers (e.g. `[messages]`, `[buffers]`).
    ///
    /// Synthetic buffers have no backing file (`path = None`) but carry a
    /// display label. Scratch buffers are path-less too, but have no label;
    /// that distinction is what this predicate captures.
    pub(in crate::editor) fn is_synthetic(&self) -> bool {
        self.path.is_none() && self.label.is_some()
    }

    /// Replace the buffer text and carry every stored position with it:
    /// through the edit's `ChangeSet`, or, for a replacement no change
    /// describes, by resetting them to the buffer's initial state. All text-mutating paths go through here,
    /// each with a text of a new version, which is how
    /// `reparse_stale_buffers` and the `on-text-changed` announcer see that
    /// the text moved.
    fn install(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        text: BufferText,
        change: Change<'_>,
    ) {
        debug_assert!(
            text.version().is_later_than(self.text.version()),
            "install: a replacement text must be a later version of the buffer's text"
        );
        let before = std::mem::replace(&mut self.text, text);
        match change {
            Change::Edit(changes) => {
                let change = TextChange::new(&before, &self.text, changes);
                if let Some(syn) = self.syntax.as_mut() {
                    syn.record_edit(&change);
                }
                stores.carry(id, &change);
            }
            Change::Replace => stores.reset(id, || crate::editor::pane_state::fresh_from_buf(self)),
        }
    }

    /// Set the buffer's file path, enforcing the "path has a basename"
    /// invariant, and derive `display_path` alongside it so the two can never
    /// drift out of pairing. Pass `None` to clear both (scratch buffer).
    ///
    /// Why: `display_name()` falls back to `*scratch*` when `path.file_name()`
    /// is `None`, so pathological paths like `/` or `..` would collide with a
    /// real scratch buffer in `:ls` and make `:b *scratch*` ambiguous. Rejecting
    /// at the boundary keeps the collision truly unreachable.
    ///
    /// The derived `display_path` is only a default: callers with a
    /// user-typed path overwrite it afterwards via `set_display_path` (see
    /// `Buffer::from_file`).
    pub(in crate::editor) fn set_path(&mut self, path: Option<PathBuf>) {
        if let Some(ref p) = path {
            debug_assert!(
                p.file_name().is_some(),
                "Buffer::set_path: path must have a basename, got {}",
                p.display()
            );
        }
        self.display_path = path.as_deref().map(hume_platform::path::display_form);
        self.path = path;
    }

    /// Canonical backing-file path, or `None` for scratch buffers.
    pub(in crate::editor) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Set the display-ready path string (see field doc). Pass `None` to clear.
    pub(in crate::editor) fn set_display_path(&mut self, display: Option<String>) {
        self.display_path = display;
    }

    /// The fully display-ready path string, or `None` for scratch/synthetic
    /// buffers. Print verbatim, no further transforms needed.
    pub(crate) fn display_path(&self) -> Option<&str> {
        self.display_path.as_deref()
    }

    /// First line of buffer content, capped at 64 bytes, stopping at the
    /// line's `\n`.
    /// Returns `None` when the first line is empty. Used for shebang detection.
    /// Iterates codepoints, not grapheme clusters: safe because shebang lines are ASCII-only.
    pub(in crate::editor) fn first_line(&self) -> Option<String> {
        const CAP: usize = 64;
        let mut out = String::with_capacity(CAP);
        for ch in self.text.rope().chars() {
            if ch == '\n' {
                break;
            }
            if out.len() + ch.len_utf8() > CAP {
                break;
            }
            out.push(ch);
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// The initial selections stored at the history root.
    ///
    /// Used to seed `PaneBufferState.selections` when a pane first views this
    /// buffer or when `:e!` reloads it from disk.
    pub(in crate::editor) fn initial_sels(&self) -> SelectionSet {
        self.history
            .initial_sels()
            .clone()
            .refit(self.text.clone())
            .into_selections()
    }

    /// The name shown in the UI: label for view buffers, basename for named
    /// buffers, `*scratch*` for unnamed ones.
    pub(crate) fn display_name(&self) -> String {
        if let Some(ref label) = self.label {
            return label.clone();
        }
        self.path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| Self::SCRATCH_BUFFER_NAME.to_owned())
    }

    /// Replace `self.text` with `new_text`, recording the swap as a single
    /// revision in the existing history so `u` reverts to the pre-reload state.
    ///
    /// This is the history-preserving reload path. Unlike
    /// [`set_view_content`](Self::set_view_content), which resets history,
    /// this treats the reload as an ordinary edit: `u` after `:e!` shows the
    /// pre-reload buffer with its full undo tree intact beneath, and
    /// `Ctrl-r` re-applies the reload.
    ///
    /// `pre_sels` (stored on the inverse transaction, restored by undo) and
    /// `post_sels` (stored on the forward transaction, restored by redo) are
    /// both caller-computed. `post_sels` is typically the grapheme-snapped,
    /// clamped cursor the reload UI wants visible.
    ///
    /// The `ChangeSet` pair is line-diff-derived ([`changesets_from_line_diff`])
    /// so the inverse carries only the changed lines, not a full-buffer
    /// delete-all + insert-all. `saved_revision` is bumped after recording so
    /// the reloaded buffer is `!is_dirty()`.
    ///
    /// The reload is an edit: `install` carries every stored position through
    /// its line diff. The history revision records `focused`'s selections for
    /// the buffer before and after.
    ///
    /// Returns whether the text changed (`install` ran, the version moved),
    /// `false` for an identical-to-disk no-op.
    pub(in crate::editor::buffer) fn reload_from_text(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        new_text: BufferText,
        focused: PaneId,
    ) -> bool {
        // Reloading from disk is, by definition, catching up to whatever is
        // there now, so clear regardless of which branch below runs.
        self.disk_state = disk::DiskState::InSync;

        // Build the CS pair from immutable borrows of both texts, before
        // `install` mutates `self.text`. The helper takes `&BufferText` on both
        // sides; `new_text` is still owned by us here so the borrow is fine.
        let (forward, inverse) = changesets_from_line_diff(&self.text, &new_text);

        // Reload of identical-to-disk content: `self.text` already equals
        // `new_text`, so skip `install` entirely rather than change the version
        // (and fire `on-text-changed` plus a spurious tree-sitter reparse) for
        // a no-op. Just re-anchor `saved_revision`: the buffer now matches
        // disk. Nothing is recorded, as there is nothing to undo to.
        if forward.is_identity() {
            self.saved_revision = Some(self.history.current_id());
            return false;
        }

        // Applying the diff keeps the buffer's lineage, so the reload is the
        // next generation of the same document rather than a new one.
        // `install` does NOT reset history (`set_view_content` is the only
        // writer that resets history).
        let reloaded = forward
            .apply(&self.text)
            .expect("a line diff of the current text applies to it");
        debug_assert_eq!(
            reloaded, new_text,
            "reload: the line diff must reproduce the file"
        );
        let recorded = |text: &BufferText, stores: &PositionStores<'_>| {
            EditState::bind(text, stores.panes[focused][id].selections().clone()).recorded()
        };
        let pre_sels = recorded(&self.text, stores);
        self.install(
            id,
            stores,
            reloaded.with_line_ending(new_text.line_ending()),
            Change::Edit(&forward),
        );
        let post_sels = recorded(&self.text, stores);
        self.record_revision(forward, inverse, pre_sels, post_sels);
        self.saved_revision = Some(self.history.current_id());
        true
    }

    /// `true` if the buffer has unsaved changes.
    ///
    /// Comparing revision IDs means undoing back to the save point correctly
    /// reports a clean buffer, which a simple `dirty: bool` flag cannot do.
    /// `saved_revision == None` (saved state evicted by promotion) always
    /// reads dirty.
    ///
    /// Tracks revision identity, not byte equality with the saved text: a
    /// history walk that lands on a *different* revision whose text happens
    /// to be byte-identical to the saved one (e.g. undoing an insert and its
    /// own later delete, past the save point) still reads dirty. Making this
    /// content-truthful would mean hashing the buffer on every dirty query
    /// (the statusline makes one every frame) for a case that self-corrects
    /// on the next real edit or save.
    pub(crate) fn is_dirty(&self) -> bool {
        self.saved_revision != Some(self.history.current_id())
    }

    /// Record the current revision as the saved state.
    ///
    /// Call this immediately after a successful file write.
    pub(in crate::editor) fn mark_saved(&mut self) {
        self.saved_revision = Some(self.history.current_id());
        self.disk_state = disk::DiskState::InSync;
    }

    /// `true` if the last disk-state check found the backing file changed or
    /// vanished and the user has not yet acted on it (reloaded or written).
    /// Test-only: `:w`'s write guard stats the file fresh instead of trusting
    /// this (see `stale_write_block`). This remains for tests that assert
    /// on the reported/warned state itself, not on write behavior.
    #[cfg(test)]
    pub(in crate::editor) fn is_disk_stale(&self) -> bool {
        !matches!(self.disk_state, disk::DiskState::InSync)
    }

    /// Set the `undo-levels` cap on this buffer's history. `0` means unlimited.
    pub(in crate::editor) fn set_undo_levels(&mut self, levels: usize) {
        self.history.set_undo_levels(levels);
    }

    /// Record a revision, remapping `saved_revision` if `undo-levels`
    /// trimming just promoted it into the new root, and invalidating
    /// `saved_revision` if trimming instead overwrote the root's state out
    /// from under it.
    ///
    /// A revision ID that gets merely evicted (not promoted) needs no
    /// handling: `RevisionId`s are never reused, so `is_dirty()`'s equality
    /// check against a stale `saved_revision` correctly stays `true`
    /// forever. Promotion is the one case that needs explicit handling,
    /// since the promoted node's state is still reachable: it's now what
    /// the root represents. But promotion also *overwrites* the root's
    /// previous state, so a `saved_revision` that pointed at `History::ROOT`
    /// (the buffer was opened, never saved since) no longer names the saved
    /// state at all. It must become `None`, not silently keep pointing at
    /// ROOT's new (different) content.
    fn record_revision(
        &mut self,
        forward: ChangeSet,
        inverse: ChangeSet,
        pre_sels: RecordedSelections,
        post_sels: RecordedSelections,
    ) {
        if let Some(promoted) = self.history.record(forward, inverse, pre_sels, post_sels) {
            if self.saved_revision == Some(promoted) {
                self.saved_revision = Some(History::ROOT);
            } else if self.saved_revision == Some(History::ROOT) {
                self.saved_revision = None;
            }
        }
    }

    /// `cmd`'s edit of `state`, which must be an edit of this buffer's text.
    ///
    /// # Panics
    /// Panics if `cmd` returns an edit of another text: its changeset and
    /// selections would be installed against the wrong one.
    fn run_edit(&self, state: EditState, cmd: impl FnOnce(EditState) -> Edited) -> Edited {
        let edited = cmd(state);
        assert_eq!(
            edited.base(),
            self.text.version(),
            "an edit of another text was applied to this buffer"
        );
        edited
    }

    /// Apply an edit and record it in the undo history.
    ///
    /// Takes `sels` (the acting pane's current selections) by value and returns
    /// the post-edit selections + the forward `ChangeSet` (for the syntax and
    /// LSP streams); `install` carries every stored position through it.
    pub(crate) fn apply_edit(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        sels: SelectionSet,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (SelectionSet, ChangeSet) {
        let pre = EditState::bind(&self.text, sels);
        let pre_sels = pre.recorded();
        let (post, cs) = self.run_edit(pre, cmd).into_parts();

        // An identity `cs` moved no bytes: recording it would litter the undo
        // tree with a no-op revision, and a new text version would fire
        // `on-text-changed` for a mutation that never happened.
        if cs.is_identity() {
            return (post.into_selections(), cs);
        }

        // self.text is still pre-edit here, so it's safe to call invert.
        let inverse_cs = cs.invert(&self.text);
        self.record_revision(cs.clone(), inverse_cs, pre_sels, post.recorded());
        self.install(id, stores, post.text().clone(), Change::Edit(&cs));
        (post.into_selections(), cs)
    }

    /// Apply an edit within the current open group, composing its CS into the
    /// group accumulator rather than recording a history revision.
    ///
    /// Caller must have called `begin_edit_group` and still hold its result.
    /// There is no `None` case to guard here, since the caller already
    /// proved a group is open by having an `&mut EditGroup` at all.
    pub(in crate::editor) fn apply_edit_grouped(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        sels: SelectionSet,
        group: &mut EditGroup,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (SelectionSet, ChangeSet) {
        let (post, cs) = self
            .run_edit(EditState::bind(&self.text, sels), cmd)
            .into_parts();
        let new_text = post.text().clone();
        let new_sels = post.into_selections();

        // An identity `cs` moved no bytes: composing it into the group
        // accumulator would still be a no-op, but a new text version would
        // fire `on-text-changed` for one. Leave the accumulator untouched:
        // a group whose every edit was identity commits nothing.
        if cs.is_identity() {
            return (new_sels, cs);
        }

        group.cs = Some(match group.cs.take() {
            None => cs.clone(),
            Some(acc) => acc.compose(cs.clone()),
        });

        self.install(id, stores, new_text, Change::Edit(&cs));
        (new_sels, cs)
    }

    /// Re-paste from the paste-session snapshot, replacing the accumulated CS.
    ///
    /// Always starts from `group.snapshot`, so every
    /// cycle cleanly discards the previous paste output (including added lines).
    /// Returns the new selections and a propagation CS mapping the current
    /// buffer text to the new text, which `install` carries every stored
    /// position through.
    ///
    /// Caller must have called `begin_edit_group` and still hold its result,
    /// the same contract as [`Buffer::apply_edit_grouped`].
    pub(in crate::editor) fn apply_edit_regrouped(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        group: &mut EditGroup,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (SelectionSet, ChangeSet) {
        let snapshot = group.snapshot.clone();
        let (post, new_cs) = cmd(snapshot).into_parts();
        let new_text = post.text().clone();
        let mut new_sels = post.into_selections();

        // Build the propagation CS: maps current buffer text → new_text.
        // On the first paste group.cs is None, meaning current == snapshot,
        // so propagation CS == new_cs.
        let propagation_cs = match &group.cs {
            None => new_cs.clone(),
            Some(prev_cs) => prev_cs
                .invert(group.snapshot.text())
                .compose(new_cs.clone()),
        };

        group.cs = Some(new_cs);
        // `propagation_cs` maps the live text onto content equal to
        // `new_text`, so the result stays in the live text's lineage and the
        // selections, tagged for `new_text`, are carried across to it. An
        // identity `propagation_cs` returns the live text itself, which
        // needs no install.
        let landed = propagation_cs
            .apply(&self.text)
            .expect("the propagation changeset maps the live text");
        rebind_to_same_content(&mut new_sels, &new_text, &landed);
        if !propagation_cs.is_identity() {
            self.install(id, stores, landed, Change::Edit(&propagation_cs));
        }
        (new_sels, propagation_cs)
    }

    /// Open an edit group. Snapshots the current text with `base`, the
    /// selections the group's edits start from, so `commit_edit_group` can
    /// invert the composed CS and record one revision, which undoes to
    /// `undo`, the selections the command that opened the group was made
    /// from.
    ///
    /// Returns a fresh [`EditGroup`] rather than writing through a pointer.
    /// The caller (`doc_ops::begin_edit_group`) is the one that knows whether
    /// a session is already open, since it owns `EditorState::active_session`;
    /// that check belongs there, not here.
    pub(in crate::editor) fn begin_edit_group(
        &self,
        base: SelectionSet,
        undo: SelectionSet,
    ) -> EditGroup {
        EditGroup {
            snapshot: EditState::bind(&self.text, base),
            undo_sels: EditState::bind(&self.text, undo).recorded(),
            cs: None,
        }
    }

    /// Close the current edit group and record it as a single undo step.
    ///
    /// If no edits were applied since `begin_edit_group` (empty group), or the
    /// composed `ChangeSet` cancelled out to the identity transform (e.g. type
    /// a char, then backspace it), no revision is recorded. Takes `group` by
    /// value: the caller already `.take()`n it from `EditorState::active_session`.
    pub(in crate::editor) fn commit_edit_group(
        &mut self,
        group: EditGroup,
        post_sels: SelectionSet,
    ) {
        if let Some(cs) = group.cs {
            // An identity `cs` moved no bytes: recording it would put a no-op
            // revision on the undo stack, and undoing that revision would
            // still consume a step and move `current`. `undo_n`/`redo_n`
            // don't inspect the transaction they're walking, only whether a
            // parent/child link exists, so `u` would consume a step, change
            // no text, and print no exhaustion message either. See
            // `apply_transactions`'s doc for why its own identity guard is a
            // different one, protecting a different case.
            if cs.is_identity() {
                return;
            }
            let inverse_cs = cs.invert(group.snapshot.text());
            let pre_sels = group.undo_sels;
            let post_sels = EditState::bind(&self.text, post_sels).recorded();
            self.record_revision(cs, inverse_cs, pre_sels, post_sels);
        }
    }

    /// Apply an ordered Transaction list (from `History::undo_n`/`redo_n`/
    /// `goto_revision`) as one composed transform: fold every ChangeSet
    /// together with `ChangeSet::compose_all` (sound because each Transaction
    /// in the list maps the state the previous one produced, see
    /// `History::goto_revision`'s doc) and apply the result once. Returns a
    /// [`HistoryWalkResult`]: the restored selections, the net ChangeSet,
    /// and how many steps `txns` held: short of the caller's requested
    /// count when the walk hit the root/leaf, so the caller can tell
    /// exhaustion apart from a full walk. `None` when `txns` is empty
    /// (nothing to do, already at the target).
    ///
    /// Always validates the landing selections via `Transaction::apply`
    /// (length + bounds check, `merge_overlapping_in_place`), but skips
    /// `install` when the composed ChangeSet is identity: a walk that
    /// undoes an insert and its own later delete nets to no text change, so
    /// the version doesn't move for a mutation that never happened. A
    /// different guard from `apply_edit`/`commit_edit_group`'s, not the same
    /// one: those two skip recording a revision at all, so a no-op edit
    /// never enters history; this walk's revision move already happened in
    /// `History::undo_n`/`redo_n`/`goto_revision` before this is even
    /// called, so all that's left to guard here is the text mutation.
    fn apply_transactions(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        txns: Vec<Transaction>,
    ) -> HistoryWalkResult {
        let steps = txns.len();
        let landing_sels = txns.last()?.selection().clone();
        let css = txns.into_iter().map(Transaction::into_changes);
        let txn = Transaction::new(
            ChangeSet::compose_all(css).expect("txns non-empty: the `?` above already returned"),
            landing_sels,
        );
        let landed = txn
            .apply(&self.text)
            .expect("composed history transaction failed: history is corrupt");
        let new_text = landed.text().clone();
        let new_sels = landed.into_selections();
        let cs = txn.into_changes();
        if !cs.is_identity() {
            self.install(id, stores, new_text, Change::Edit(&cs));
        }
        Some((new_sels, cs, steps))
    }

    /// Undo up to `count` steps as one composed transform: the production
    /// path for `5u` and an age-resolved `:earlier`, so a multi-step travel
    /// pays for one `install`/`finish_edit` cycle instead of `count` of
    /// them. See [`Self::apply_transactions`] for the return contract.
    pub(crate) fn undo_n(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        count: usize,
    ) -> HistoryWalkResult {
        let txns = self.history.undo_n(count);
        self.apply_transactions(id, stores, txns)
    }

    /// Redo up to `count` steps forward as one composed transform. See
    /// `undo_n`'s doc: same contract, redo direction.
    pub(crate) fn redo_n(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        count: usize,
    ) -> HistoryWalkResult {
        let txns = self.history.redo_n(count);
        self.apply_transactions(id, stores, txns)
    }

    /// The current buffer contents.
    pub(crate) fn text(&self) -> &BufferText {
        &self.text
    }

    /// The current revision in the undo history.
    #[cfg(test)]
    pub(in crate::editor) fn revision_id(&self) -> RevisionId {
        self.history.current_id()
    }

    /// Test-only: production does not branch on this. `undo_n`/`redo_n`
    /// clamp at the root themselves and report `taken < requested` instead
    /// of checking `can_undo` up front.
    #[cfg(test)]
    pub(in crate::editor) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Undo steps back to the state as of `age` ago, for `:earlier`: a narrow
    /// delegate so the typed layer never touches `History` itself. See
    /// `History::undo_steps_older_than`'s own doc for the `Result` contract.
    pub(in crate::editor) fn undo_steps_older_than(&self, age: Duration) -> Result<usize, usize> {
        self.history.undo_steps_older_than(age)
    }

    /// Redo steps forward to the state as of `age` ago, for `:later`.
    pub(in crate::editor) fn redo_steps_newer_than(&self, age: Duration) -> Result<usize, usize> {
        self.history.redo_steps_newer_than(age)
    }

    /// Jump to an arbitrary revision in the undo tree.
    #[cfg(test)]
    pub(in crate::editor) fn goto_revision(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        sels: &mut SelectionSet,
        target: hume_editing::history::RevisionId,
    ) {
        if let Some(transactions) = self.history.goto_revision(target)
            && let Some((new_sels, _cs, _steps)) = self.apply_transactions(id, stores, transactions)
        {
            *sels = new_sels;
        }
    }
}

/// Retag `sels`, computed for `from`, as selections of `to`, another version
/// of the same content.
fn rebind_to_same_content(sels: &mut SelectionSet, from: &BufferText, to: &BufferText) {
    debug_assert!(
        from.rope() == to.rope(),
        "rebind: the two texts must hold the same content"
    );
    sels.translate(&TextChange::new(
        from,
        to,
        &ChangeSet::identity(to.len_chars()),
    ));
}

#[cfg(test)]
mod tests;
