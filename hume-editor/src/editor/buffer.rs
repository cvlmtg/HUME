use std::io;
use std::path::{Path, PathBuf};

use super::search_state::{SearchMatches, SearchPattern};
use crate::editor::pane_state::EditGroup;
use crate::settings::BufferOverrides;
use hume_editing::changeset::{ChangeSet, changesets_from_line_diff};
use hume_editing::history::{History, RevisionId};
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_platform::io::FileMeta;
use hume_treesitter::registry::BufferSyntax;

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
    saved_revision: RevisionId,
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
    /// Detected or explicitly set language identity (e.g. `"rust"`, `"json"`).
    /// `None` for unrecognised filetypes and scratch buffers.
    pub(crate) language: Option<String>,
    /// Monotonically increasing counter, bumped on every text mutation.
    /// `reparse_stale_buffers` skips a buffer when this equals
    /// `syntax.parsed_gen`.
    pub(crate) text_gen: u64,
    /// Per-buffer tree-sitter syntax attachment. Holds grammar identity and the
    /// `text_gen` of the last installed tree. `None` when no grammar is attached
    /// or the buffer exceeds `syntax-highlight-max-bytes`.
    pub(crate) syntax: Option<BufferSyntax>,
    /// When `true`, all forward text mutations are blocked at the `doc_ops`
    /// layer. Entering Insert mode is also refused. Read-only is orthogonal to
    /// language/syntax — a read-only buffer may still be highlighted.
    pub(crate) read_only: bool,
    /// Display name used for synthetic, path-less view buffers (e.g. `"[messages]"`).
    /// Shown in the statusline and `:ls` instead of `*scratch*`.
    pub(crate) label: Option<String>,
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
        let saved_revision = history.current_id();
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
            text_gen: 0,
            syntax: None,
            read_only: false,
            label: None,
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
        self.history = History::new(SelectionSet::default(), text_len);
        self.saved_revision = self.history.current_id();
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
    /// the full `Buffer` swap performed by `ops::replace_buffer_in_place`
    /// (which discards history), this treats the reload as an ordinary edit:
    /// `u` after `:e!` shows the pre-reload buffer with its full prior undo
    /// tree intact beneath, and `Ctrl-r` re-applies the reload.
    ///
    /// `pre_sels` is the cursor state before the reload (stored on the inverse
    /// transaction — undo restores it); `post_sels` is the cursor state after
    /// the reload (stored on the forward transaction — redo restores it). Both
    /// are caller-computed; `post_sels` is typically the grapheme-snapped
    /// clamped cursor the reload UI wants visible.
    ///
    /// The `ChangeSet` pair is line-diff-derived
    /// ([`changesets_from_line_diff`]) so the inverse carries only the
    /// changed lines, not a full-buffer delete-all + insert-all. After
    /// recording, `saved_revision` is bumped to the new revision so the
    /// freshly-reloaded buffer is `!is_dirty()` — matching the old
    /// buffer-swap behaviour where the fresh-from-disk doc was clean.
    pub(crate) fn reload_from_text(
        &mut self,
        new_text: Text,
        pre_sels: SelectionSet,
        post_sels: SelectionSet,
    ) {
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
            self.saved_revision = self.history.current_id();
            return;
        }

        self.history.record(forward, inverse, pre_sels, post_sels);
        self.saved_revision = self.history.current_id();
    }

    /// `true` if the buffer has unsaved changes.
    ///
    /// Comparing revision IDs means undoing back to the save point correctly
    /// reports a clean buffer — a simple `dirty: bool` flag cannot do this.
    pub(crate) fn is_dirty(&self) -> bool {
        self.history.current_id() != self.saved_revision
    }

    /// Record the current revision as the saved state.
    ///
    /// Call this immediately after a successful file write.
    pub(crate) fn mark_saved(&mut self) {
        self.saved_revision = self.history.current_id();
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
        self.history
            .record(cs.clone(), inverse_cs, sels, new_sels.clone());
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
            self.history
                .record(cs, inverse_cs, group.pre_sels, post_sels);
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
