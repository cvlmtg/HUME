use std::io;
use std::path::{Path, PathBuf};

use super::search::{SearchMatches, SearchPattern};
use crate::editor::pane_state::EditGroup;
use crate::settings::BufferOverrides;
use hume_editing::changeset::{ChangeSet, changesets_from_line_diff};
use hume_editing::history::{History, RevisionId};
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_platform::io::FileMeta;

mod disk;
// Production callers reach both only via `super::disk::*` from sibling
// buffer submodules (`file_open::enter_buffer_with_jump`) or `is_disk_stale()`
// — these re-exports exist only so test code (a different module tree) can
// name `DiskCheckTrigger`/`DiskState` directly.
#[cfg(test)]
pub(crate) use disk::{DiskCheckTrigger, DiskState};
mod file_open;
pub(crate) mod lifecycle;
pub(crate) mod store;
use hume_treesitter::registry::LanguageId;
use hume_treesitter::syntax::Syntax;

// ── LastInsert ────────────────────────────────────────────────────────────────

/// The span(s) typed during the most recently completed insert session,
/// stamped with the buffer's `text_gen` at capture time.
///
/// `mii` (`select-last-insertion`) compares `text_gen` against the buffer's
/// live generation before using `spans` — any intervening mutation (an edit,
/// undo, or redo, all of which bump `text_gen`) invalidates it rather than
/// trying to remap positions through the change.
pub(crate) struct LastInsert {
    /// One inclusive `(start, end)` char range per selection that was active
    /// during the session, sorted by `start`.
    pub(crate) spans: Vec<(usize, usize)>,
    pub(crate) text_gen: u64,
}

// ── Buffer ────────────────────────────────────────────────────────────────────

/// Content-only document: text, undo history, search state, and per-buffer overrides.
///
/// `Buffer` is the SSOT for everything intrinsic to an open file and shared
/// across all panes viewing it. It does **not** own:
/// - selections (per-(pane, buffer) — live on `PaneBufferState`)
/// - viewport / scroll (per-pane — live on engine `Pane`)
/// - per-pane search cursor (live on `PaneBufferState`)
/// - edit groups / insert sessions (per-(pane, buffer) — live on `PaneBufferState`)
///
/// ## Edit API
///
/// All text mutations go through [`apply_edit`] or [`apply_edit_grouped`].
/// Both take the acting pane's `SelectionSet` as a parameter, return the
/// post-edit `SelectionSet` + a `ChangeSet` for propagation to non-acting panes,
/// and handle undo bookkeeping internally.
pub(crate) struct Buffer {
    text: Text,
    history: History,
    /// The revision at which the buffer was last saved (or first opened).
    /// `None` means the saved state was overwritten by an `undo-levels`
    /// promotion and no longer exists anywhere in the tree — the buffer is
    /// dirty until the next save.
    saved_revision: Option<RevisionId>,
    /// Canonical file path (after symlink resolution). `None` for scratch buffers.
    pub(super) path: Option<PathBuf>,
    /// Absolute path as supplied by the user (symlinks NOT resolved). Display-only;
    /// `path` is the canonical identity for dedup and I/O. `None` when the buffer
    /// was not opened via a user-typed path (scratch, synthetic, startup arg without
    /// recording). `FilePath` statusline element falls back to `path` when absent.
    pub(super) display_path: Option<PathBuf>,
    /// File metadata captured at open/save time (permissions, uid/gid).
    /// `None` for scratch buffers; populated after a successful save.
    pub(crate) file_meta: Option<FileMeta>,
    /// Active search pattern shared by all panes viewing this buffer.
    /// `None` when no search is active. A present `SearchPattern` is always
    /// fully-valid — invalid regexes leave this as `None`.
    pub(crate) search_pattern: Option<SearchPattern>,
    /// Cached match list for `search_pattern`. Invalidated by revision change
    /// or pattern change; rebuilt lazily by `update_buffer_matches`.
    pub(crate) search_matches: SearchMatches,
    /// Per-buffer setting overrides. `None` fields inherit from
    /// [`crate::settings::EditorSettings`].
    pub(crate) overrides: BufferOverrides,
    /// Detected or explicitly set language identity (e.g. `rust`, `json`).
    /// `None` for unrecognised filetypes and scratch buffers.
    pub(crate) language: Option<LanguageId>,
    /// `true` when `language` was written by `:set buffer language=` or Steel's
    /// `set-buffer-language!`, rather than by detection. `:reload-config`'s
    /// reset reads this (before clearing it) to restore the user's own
    /// assertion across the reload instead of letting re-detection silently
    /// pick something else — see `clear_languages_all`.
    pub(crate) language_explicit: bool,
    /// Monotonically increasing counter, bumped on every text mutation.
    /// `reparse_stale_buffers` skips a buffer when this equals
    /// `syntax.parsed_gen()`.
    pub(crate) text_gen: u64,
    /// Per-buffer tree-sitter syntax attachment: grammar identity, committed
    /// parse layers, generation bookkeeping, and in-flight state, all in one
    /// place. `None` when no grammar is attached or the buffer exceeds
    /// `syntax-highlight-max-bytes`.
    pub(crate) syntax: Option<Syntax>,
    /// When `true`, all forward text mutations are blocked at the `doc_ops`
    /// layer. Entering Insert mode is also refused. Read-only is orthogonal to
    /// language/syntax — a read-only buffer may still be highlighted.
    pub(crate) read_only: bool,
    /// Display name used for synthetic, path-less view buffers (e.g. `"[messages]"`).
    /// Shown in the statusline and `:ls` instead of `*scratch*`.
    pub(crate) label: Option<String>,
    /// The LSP server this buffer is attached to, if any (set once by
    /// `Editor::lsp_attach_buffer`; `None` for unnamed buffers, buffers with
    /// no registered server, or before the open-time attach attempt runs).
    pub(crate) lsp_server: Option<hume_lsp::backend::ServerId>,
    /// Text mutations queued for `textDocument/didChange` conversion, in
    /// order. Recorded at the same chokepoint as tree-sitter's pending
    /// edits (`doc_ops.rs`'s five apply functions); drained by the LSP
    /// per-frame flush. Always empty when `lsp_server` is `None`.
    pub(crate) lsp_pending: Vec<super::lsp::sync::LspPendingChange>,
    /// Spans typed during the most recently completed insert session, for
    /// `mii` (`select-last-insertion`). `None` before any session completes,
    /// or once `text_gen` has moved past the stamp (see [`LastInsert`]).
    pub(crate) last_insert: Option<LastInsert>,
    /// True from chokepoint open (`lifecycle::open_buffer_and_notify`) until
    /// `Editor::detect_pending_languages` fires this buffer's `OnBufferOpen`.
    /// Read by `lifecycle::close_buffer_and_notify`: a still-pending buffer
    /// (opened and closed before that drain ran — e.g. within one Steel eval)
    /// announced no open, so it must announce no close either. Buffers
    /// created outside the chokepoint (the startup buffer, the last-buffer
    /// scratch replacement) default to `false` — their close always
    /// announces.
    pub(crate) open_hook_pending: bool,
    /// The buffer's disk state as of the last check — set by
    /// `Editor::check_buffer_disk_state`, cleared to `InSync` by a reload or
    /// a successful write. Always `InSync` for scratch/synthetic buffers,
    /// which the check skips.
    pub(crate) disk_state: disk::DiskState,
    /// Bumped by [`lifecycle::replace_buffer_in_place`] — the only path that
    /// swaps a `BufferId`'s content without a close/open pair (the
    /// last-buffer scratch replacement in `close_buffer`). A versioned
    /// slotmap key alone can't distinguish "the same buffer that was open
    /// before `:reload-config` started" from "a fresh scratch buffer that
    /// happens to have reused that same key in place": `replace_buffer_in_place`
    /// mutates the existing slot rather than freeing and reinserting it, so
    /// the key's own version never changes. `reset_config_state` pairs each
    /// pre-reload `BufferId` with this stamp; `resync_config_state` and the
    /// explicit-language restore treat a bid whose current stamp has moved
    /// on as a different buffer, not the one the snapshot meant.
    pub(crate) replace_stamp: u64,
}

impl Buffer {
    /// Display name used for buffers that have no backing file.
    pub(crate) const SCRATCH_BUFFER_NAME: &'static str = "*scratch*";

    /// Create a new buffer from text and an initial selection state.
    ///
    /// `initial_sels` are stored in the history root so `initial_sels()` can
    /// recover them for seeding `PaneBufferState` on first open or `:e!` reload.
    pub(crate) fn new(text: Text, initial_sels: SelectionSet) -> Self {
        let text_len = text.len_chars();
        let history = History::new(initial_sels, text_len);
        let saved_revision = Some(history.current_id());
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
            text_gen: 0,
            syntax: None,
            read_only: false,
            label: None,
            lsp_server: None,
            lsp_pending: Vec::new(),
            last_insert: None,
            open_hook_pending: false,
            disk_state: disk::DiskState::InSync,
            replace_stamp: 0,
        }
    }

    /// Load a file from disk, returning a ready-to-use `Buffer`.
    ///
    /// Sets `path` and `file_meta` from the resolved filesystem metadata.
    /// `search_pattern` and `search_matches` are left at their defaults
    /// (no active search) — caller contract for `replace_buffer_in_place`.
    pub(crate) fn from_file(path: &Path) -> io::Result<Self> {
        let (content, meta) = hume_platform::io::read_file(path)?;
        let text = Text::from(content.as_str());
        let sels = SelectionSet::default();
        let mut buf = Self::new(text, sels);
        buf.set_path(Some(meta.resolved_path().to_path_buf()));
        buf.file_meta = Some(meta);
        Ok(buf)
    }

    /// Empty scratch buffer (single structural `\n`, no path, default overrides).
    ///
    /// Used when closing the last buffer to keep the "always ≥1 buffer open"
    /// invariant without leaving the editor in an invalid state.
    pub(crate) fn scratch() -> Self {
        Self::new(Text::empty(), SelectionSet::default())
    }

    /// Create a read-only view buffer from in-memory content.
    ///
    /// Used for `:messages`, `:ls`, and `:plugin-status`. The buffer has no
    /// backing file, no language detection, and blocks all user edits.
    pub(crate) fn read_only_view(text: Text, label: String) -> Self {
        let mut buf = Self::new(text, SelectionSet::default());
        buf.read_only = true;
        buf.label = Some(label);
        buf
    }

    /// Replace the content of a read-only view buffer with new text.
    ///
    /// Resets history to a clean root and clears search state so the refreshed
    /// buffer is non-dirty and has no stale match data. This is a system
    /// refresh — it intentionally bypasses the `read_only` guard in `doc_ops`.
    pub(crate) fn set_view_content(&mut self, text: Text) {
        let text_len = text.len_chars();
        let undo_levels = self.history.undo_levels();
        self.history = History::new(SelectionSet::default(), text_len);
        self.history.set_undo_levels(undo_levels);
        self.saved_revision = Some(self.history.current_id());
        self.search_pattern = None;
        self.search_matches = SearchMatches::default();
        self.set_text(text);
    }

    /// `true` when the buffer blocks user edits.
    pub(crate) fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// `true` for in-memory view buffers (e.g. `[messages]`, `[buffers]`).
    ///
    /// Synthetic buffers have no backing file (`path = None`) but carry a
    /// display label. Scratch buffers are path-less too, but have no label —
    /// that distinction is what this predicate captures.
    pub(crate) fn is_synthetic(&self) -> bool {
        self.path.is_none() && self.label.is_some()
    }

    /// Replace the buffer text and bump `text_gen` so `reparse_stale_buffers`
    /// knows a new parse is needed. All text-mutating paths go through here.
    fn set_text(&mut self, text: Text) {
        self.text = text;
        self.text_gen += 1;
    }

    /// Set the buffer's file path, enforcing the "path has a basename"
    /// invariant. Pass `None` to clear (scratch buffer).
    ///
    /// Why: `display_name()` falls back to `*scratch*` when `path.file_name()`
    /// is `None`, so pathological paths like `/` or `..` would collide with a
    /// real scratch buffer in `:ls` and make `:b *scratch*` ambiguous. Rejecting
    /// at the boundary keeps the collision truly unreachable.
    pub(crate) fn set_path(&mut self, path: Option<PathBuf>) {
        if let Some(ref p) = path {
            debug_assert!(
                p.file_name().is_some(),
                "Buffer::set_path: path must have a basename, got {}",
                p.display()
            );
        }
        self.path = path;
    }

    /// Canonical backing-file path, or `None` for scratch buffers.
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Set the display path (user-supplied, symlinks unresolved). Pass `None` to clear.
    pub(crate) fn set_display_path(&mut self, path: Option<PathBuf>) {
        self.display_path = path;
    }

    /// Absolute path as supplied by the user (symlinks NOT resolved), or `None` when
    /// the buffer was not opened via a typed path. The `FilePath` statusline element
    /// falls back to `self.path()` when this returns `None`.
    pub(crate) fn display_path(&self) -> Option<&Path> {
        self.display_path.as_deref()
    }

    /// First line of buffer content, capped at 64 bytes, stripped of trailing newlines.
    /// Returns `None` when the first line is empty. Used for shebang detection.
    /// Iterates codepoints, not grapheme clusters — safe because shebang lines are ASCII-only.
    pub(crate) fn first_line(&self) -> Option<String> {
        const CAP: usize = 64;
        let mut out = String::with_capacity(CAP);
        for ch in self.text.rope().chars() {
            if ch == '\n' || ch == '\r' {
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
    pub(crate) fn initial_sels(&self) -> SelectionSet {
        self.history.initial_sels().clone()
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
    /// This is the history-preserving reload path used by `:e!`. Unlike
    /// [`set_view_content`](Self::set_view_content) (which resets history) or
    /// the full `Buffer` swap in `lifecycle::replace_buffer_in_place` (which
    /// discards history), this treats the reload as an ordinary edit: `u`
    /// after `:e!` shows the pre-reload buffer with its full undo tree intact
    /// beneath, and `Ctrl-r` re-applies the reload.
    ///
    /// `pre_sels` (stored on the inverse transaction, restored by undo) and
    /// `post_sels` (stored on the forward transaction, restored by redo) are
    /// both caller-computed — `post_sels` is typically the grapheme-snapped,
    /// clamped cursor the reload UI wants visible.
    ///
    /// The `ChangeSet` pair is line-diff-derived ([`changesets_from_line_diff`])
    /// so the inverse carries only the changed lines, not a full-buffer
    /// delete-all + insert-all. `saved_revision` is bumped after recording so
    /// the reloaded buffer is `!is_dirty()`.
    pub(crate) fn reload_from_text(
        &mut self,
        new_text: Text,
        pre_sels: SelectionSet,
        post_sels: SelectionSet,
    ) {
        // Reloading from disk is, by definition, catching up to whatever is
        // there now — clear regardless of which branch below runs.
        self.disk_state = disk::DiskState::InSync;

        // Build the CS pair from immutable borrows of both texts, before
        // `set_text` mutates `self.text`. The helper takes `&Text` on both
        // sides; `new_text` is still owned by us here so the borrow is fine.
        let (forward, inverse) = changesets_from_line_diff(&self.text, &new_text);

        // `set_text` only bumps `text_gen`; it does NOT reset history
        // (`set_view_content` is the only writer that resets history).
        self.set_text(new_text);

        // Reload of identical-to-disk content: don't litter the undo tree with a
        // no-op revision. Just re-anchor `saved_revision` to the current node so
        // the buffer reads clean (it now matches disk). `pre_sels`/`post_sels`
        // are dropped — there is nothing to undo to.
        if forward.is_identity() {
            self.saved_revision = Some(self.history.current_id());
            return;
        }

        self.record_revision(forward, inverse, pre_sels, post_sels);
        self.saved_revision = Some(self.history.current_id());
    }

    /// `true` if the buffer has unsaved changes.
    ///
    /// Comparing revision IDs means undoing back to the save point correctly
    /// reports a clean buffer — a simple `dirty: bool` flag cannot do this.
    /// `saved_revision == None` (saved state evicted by promotion) always
    /// reads dirty.
    pub(crate) fn is_dirty(&self) -> bool {
        self.saved_revision != Some(self.history.current_id())
    }

    /// Record the current revision as the saved state.
    ///
    /// Call this immediately after a successful file write.
    pub(crate) fn mark_saved(&mut self) {
        self.saved_revision = Some(self.history.current_id());
        self.disk_state = disk::DiskState::InSync;
    }

    /// `true` if the last disk-state check found the backing file changed or
    /// vanished and the user has not yet acted on it (reloaded or written).
    pub(crate) fn is_disk_stale(&self) -> bool {
        !matches!(self.disk_state, disk::DiskState::InSync)
    }

    /// Set the `undo-levels` cap on this buffer's history. `0` means unlimited.
    pub(crate) fn set_undo_levels(&mut self, levels: usize) {
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
    /// since the promoted node's state is still reachable — it's now what
    /// the root represents. But promotion also *overwrites* the root's
    /// previous state, so a `saved_revision` that pointed at `History::ROOT`
    /// (the buffer was opened, never saved since) no longer names the saved
    /// state at all — it must become `None`, not silently keep pointing at
    /// ROOT's new (different) content.
    fn record_revision(
        &mut self,
        forward: ChangeSet,
        inverse: ChangeSet,
        pre_sels: SelectionSet,
        post_sels: SelectionSet,
    ) {
        if let Some(promoted) = self.history.record(forward, inverse, pre_sels, post_sels) {
            if self.saved_revision == Some(promoted) {
                self.saved_revision = Some(History::ROOT);
            } else if self.saved_revision == Some(History::ROOT) {
                self.saved_revision = None;
            }
        }
    }

    /// Apply an edit and record it in the undo history.
    ///
    /// Takes `sels` (the acting pane's current selections) by value and returns
    /// the post-edit selections + the forward `ChangeSet` (for propagation to
    /// non-acting panes via `propagate_cs_to_panes`).
    pub(crate) fn apply_edit(
        &mut self,
        sels: SelectionSet,
        cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
    ) -> (SelectionSet, ChangeSet) {
        // Clone the buffer for the edit — O(log n) via ropey structural sharing.
        let (new_text, new_sels, cs) = cmd(self.text.clone(), sels.clone());

        // self.text is still pre-edit here — safe to call invert.
        let inverse_cs = cs.invert(&self.text);
        self.record_revision(cs.clone(), inverse_cs, sels, new_sels.clone());
        self.set_text(new_text);
        (new_sels, cs)
    }

    /// Apply an edit within the current open group, composing its CS into the
    /// group accumulator rather than recording a history revision.
    ///
    /// `edit_group` must be `Some` — caller must have called `begin_edit_group`
    /// first. Panics (debug) if `None`.
    pub(crate) fn apply_edit_grouped(
        &mut self,
        sels: SelectionSet,
        edit_group: &mut Option<EditGroup>,
        cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
    ) -> (SelectionSet, ChangeSet) {
        let group = edit_group
            .as_mut()
            .expect("apply_edit_grouped called without an open group");

        let (new_text, new_sels, cs) = cmd(self.text.clone(), sels);

        group.cs = Some(match group.cs.take() {
            None => cs.clone(),
            Some(acc) => acc.compose(cs.clone()),
        });

        self.set_text(new_text);
        (new_sels, cs)
    }

    /// Re-paste from the paste-session snapshot, replacing the accumulated CS.
    ///
    /// Always starts from `group.text_snapshot` / `group.pre_sels`, so every
    /// cycle cleanly discards the previous paste output (including added lines).
    /// Returns the new selections and a propagation CS mapping the current buffer
    /// text → the new text (for `propagate_cs_to_panes`).
    ///
    /// `edit_group` must be `Some` — caller must have called `begin_edit_group`
    /// first. Panics if `None`.
    pub(crate) fn apply_edit_regrouped(
        &mut self,
        edit_group: &mut Option<EditGroup>,
        cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
    ) -> (SelectionSet, ChangeSet) {
        let group = edit_group
            .as_mut()
            .expect("apply_edit_regrouped called without an open group");

        let (new_text, new_sels, new_cs) = cmd(group.text_snapshot.clone(), group.pre_sels.clone());

        // Build the propagation CS: maps current buffer text → new_text.
        // On the first paste group.cs is None, meaning current == snapshot,
        // so propagation CS == new_cs.
        let propagation_cs = match &group.cs {
            None => new_cs.clone(),
            Some(prev_cs) => prev_cs.invert(&group.text_snapshot).compose(new_cs.clone()),
        };

        group.cs = Some(new_cs);
        self.set_text(new_text);
        (new_sels, propagation_cs)
    }

    /// Open an edit group. Snapshots the current text and the provided `pre_sels`
    /// so `commit_edit_group` can invert the composed CS and record one revision.
    ///
    /// Panics (debug) if a group is already open.
    pub(crate) fn begin_edit_group(
        &self,
        edit_group: &mut Option<EditGroup>,
        pre_sels: SelectionSet,
    ) {
        debug_assert!(
            edit_group.is_none(),
            "begin_edit_group called with group already open"
        );
        *edit_group = Some(EditGroup {
            text_snapshot: self.text.clone(),
            pre_sels,
            cs: None,
        });
    }

    /// Close the current edit group and record it as a single undo step.
    ///
    /// If no edits were applied since `begin_edit_group` (empty group), no
    /// revision is recorded. Panics if no group is open.
    pub(crate) fn commit_edit_group(
        &mut self,
        edit_group: &mut Option<EditGroup>,
        post_sels: SelectionSet,
    ) {
        let group = edit_group
            .take()
            .expect("commit_edit_group called without an open group");

        if let Some(cs) = group.cs {
            let inverse_cs = cs.invert(&group.text_snapshot);
            self.record_revision(cs, inverse_cs, group.pre_sels, post_sels);
        }
    }

    /// Undo the last edit. Returns `(restored_sels, inverse_cs)` on success,
    /// or `None` if already at the root.
    ///
    /// The returned CS maps post-edit positions → pre-edit positions — pass it
    /// to `propagate_cs_to_panes` so non-acting panes' cursors ride the undo.
    pub(crate) fn undo(&mut self) -> Option<(SelectionSet, ChangeSet)> {
        let txn = self.history.undo()?;
        let (new_text, new_sels) = txn
            .apply(&self.text)
            .expect("inverse transaction failed — history is corrupt");
        self.set_text(new_text);
        Some((new_sels, txn.into_changes()))
    }

    /// Redo the most recent undone edit. Returns `(restored_sels, forward_cs)`.
    ///
    /// The returned CS maps pre-edit positions → post-edit positions.
    pub(crate) fn redo(&mut self) -> Option<(SelectionSet, ChangeSet)> {
        let txn = self.history.redo()?;
        let (new_text, new_sels) = txn
            .apply(&self.text)
            .expect("forward transaction failed — history is corrupt");
        self.set_text(new_text);
        Some((new_sels, txn.into_changes()))
    }

    /// The current buffer contents.
    pub(crate) fn text(&self) -> &Text {
        &self.text
    }

    /// The current revision in the undo history.
    pub(crate) fn revision_id(&self) -> RevisionId {
        self.history.current_id()
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Jump to an arbitrary revision in the undo tree.
    #[cfg(test)]
    pub(crate) fn goto_revision(
        &mut self,
        sels: &mut SelectionSet,
        target: hume_editing::history::RevisionId,
    ) {
        if let Some(transactions) = self.history.goto_revision(target) {
            for txn in transactions {
                let (new_text, new_sels) = txn
                    .apply(&self.text)
                    .expect("goto_revision transaction failed — history is corrupt");
                self.set_text(new_text);
                *sels = new_sels;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
