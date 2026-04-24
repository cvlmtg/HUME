use std::io;
use std::path::{Path, PathBuf};

use crate::core::changeset::ChangeSet;
use crate::core::history::{History, RevisionId};
use crate::core::search_state::{SearchMatches, SearchPattern};
use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::editor::pane_state::EditGroup;
use crate::os::io::FileMeta;
use crate::settings::BufferOverrides;

// ── IntoApplyResult ───────────────────────────────────────────────────────────

/// Converts a closure's return value into the canonical 4-tuple that
/// `Buffer::apply_edit` needs internally.
///
/// Implemented for the 3-tuple `(Text, SelectionSet, ChangeSet)` (plain edits)
/// and the 4-tuple `(Text, SelectionSet, ChangeSet, Vec<String>)` (paste, which
/// also captures displaced text). `None` is returned for the `Vec<String>` on
/// the common path — no allocation.
pub(crate) trait IntoApplyResult {
    fn into_apply_result(self) -> (Text, SelectionSet, ChangeSet, Option<Vec<String>>);
}

impl IntoApplyResult for (Text, SelectionSet, ChangeSet) {
    fn into_apply_result(self) -> (Text, SelectionSet, ChangeSet, Option<Vec<String>>) {
        (self.0, self.1, self.2, None)
    }
}

impl IntoApplyResult for (Text, SelectionSet, ChangeSet, Vec<String>) {
    fn into_apply_result(self) -> (Text, SelectionSet, ChangeSet, Option<Vec<String>>) {
        (self.0, self.1, self.2, Some(self.3))
    }
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
    saved_revision: RevisionId,
    /// Canonical file path (after symlink resolution). `None` for scratch buffers.
    pub(super) path: Option<PathBuf>,
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
            file_meta: None,
            search_pattern: None,
            search_matches: SearchMatches::default(),
            overrides: BufferOverrides::default(),
        }
    }

    /// Load a file from disk, returning a ready-to-use `Buffer`.
    ///
    /// Sets `path` and `file_meta` from the resolved filesystem metadata.
    /// `search_pattern` and `search_matches` are left at their defaults
    /// (no active search) — caller contract for `replace_buffer_in_place`.
    pub(crate) fn from_file(path: &Path) -> io::Result<Self> {
        let (content, meta) = crate::os::io::read_file(path)?;
        let text = Text::from(content.as_str());
        let sels = SelectionSet::default();
        let mut buf = Self::new(text, sels);
        buf.set_path(Some(meta.resolved_path.clone()));
        buf.file_meta = Some(meta);
        Ok(buf)
    }

    /// Empty scratch buffer (single structural `\n`, no path, default overrides).
    ///
    /// Used when closing the last buffer to keep the "always ≥1 buffer open"
    /// invariant without leaving the editor in an invalid state.
    #[allow(dead_code)] // used when closing the last buffer in multi-buffer
    pub(crate) fn scratch() -> Self {
        Self::new(Text::empty(), SelectionSet::default())
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

    /// The initial selections stored at the history root.
    ///
    /// Used to seed `PaneBufferState.selections` when a pane first views this
    /// buffer or when `:e!` reloads it from disk.
    pub(crate) fn initial_sels(&self) -> SelectionSet {
        self.history.initial_sels().clone()
    }

    /// The name shown in the UI: basename for named buffers, `*scratch*` for unnamed ones.
    pub(crate) fn display_name(&self) -> String {
        self.path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| Self::SCRATCH_BUFFER_NAME.to_owned())
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
    /// the post-edit selections + an optional displaced-text Vec (non-`None`
    /// only for paste operations) + the forward `ChangeSet` (for propagation to
    /// non-acting panes via `propagate_cs_to_panes`).
    ///
    /// The closure receives `(Text, SelectionSet)` and may return either a
    /// 3-tuple `(Text, SelectionSet, ChangeSet)` (plain edits) or a 4-tuple
    /// `(Text, SelectionSet, ChangeSet, Vec<String>)` (paste). Both are accepted
    /// via [`IntoApplyResult`].
    pub(crate) fn apply_edit<R: IntoApplyResult>(
        &mut self,
        sels: SelectionSet,
        cmd: impl FnOnce(Text, SelectionSet) -> R,
    ) -> (SelectionSet, Option<Vec<String>>, ChangeSet) {
        // Clone the buffer for the edit — O(log n) via ropey structural sharing.
        let (new_text, new_sels, cs, displaced) =
            cmd(self.text.clone(), sels.clone()).into_apply_result();

        // self.text is still pre-edit here — safe to call invert.
        let inverse_cs = cs.invert(&self.text);
        self.history
            .record(cs.clone(), inverse_cs, sels, new_sels.clone());
        self.text = new_text;
        (new_sels, displaced, cs)
    }

    /// Apply an edit within the current open group, composing its CS into the
    /// group accumulator rather than recording a history revision.
    ///
    /// `edit_group` must be `Some` — caller must have called `begin_edit_group`
    /// first. Panics (debug) if `None`.
    pub(crate) fn apply_edit_grouped<R: IntoApplyResult>(
        &mut self,
        sels: SelectionSet,
        edit_group: &mut Option<EditGroup>,
        cmd: impl FnOnce(Text, SelectionSet) -> R,
    ) -> (SelectionSet, Option<Vec<String>>, ChangeSet) {
        let group = edit_group
            .as_mut()
            .expect("apply_edit_grouped called without an open group");

        let (new_text, new_sels, cs, displaced) = cmd(self.text.clone(), sels).into_apply_result();

        group.cs = Some(match group.cs.take() {
            None => cs.clone(),
            Some(acc) => acc.compose(cs.clone()),
        });

        self.text = new_text;
        (new_sels, displaced, cs)
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
        self.text = new_text;
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
        self.text = new_text;
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

    /// `true` if there is at least one edit to undo.
    #[cfg(test)]
    pub(crate) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Jump to an arbitrary revision in the undo tree.
    #[allow(dead_code)]
    pub(crate) fn goto_revision(
        &mut self,
        sels: &mut SelectionSet,
        target: crate::core::history::RevisionId,
    ) {
        if let Some(transactions) = self.history.goto_revision(target) {
            for txn in transactions {
                let (new_text, new_sels) = txn
                    .apply(&self.text)
                    .expect("goto_revision transaction failed — history is corrupt");
                self.text = new_text;
                *sels = new_sels;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::edit::{
        delete_char_backward, delete_char_forward, delete_selection, insert_char, paste_after,
        paste_before, repeat_edit,
    };
    use crate::ops::register::yank_selections;
    use crate::testing::{parse_state, serialize_state};
    use pretty_assertions::assert_eq;

    // ── DocHelper ─────────────────────────────────────────────────────────────
    //
    // Thin test wrapper that keeps a `SelectionSet` alongside the `Buffer` so
    // the tests can use the old ergonomic API without the new buffer requiring
    // sels to be passed at every call site.

    struct DocHelper {
        buf: Buffer,
        sels: SelectionSet,
        edit_group: Option<EditGroup>,
    }

    impl DocHelper {
        fn apply_edit<R: IntoApplyResult>(
            &mut self,
            cmd: impl FnOnce(Text, SelectionSet) -> R,
        ) -> Option<Vec<String>> {
            let sels = std::mem::take(&mut self.sels);
            let (new_sels, displaced, _cs) = self.buf.apply_edit(sels, cmd);
            self.sels = new_sels;
            displaced
        }

        fn apply_edit_grouped<R: IntoApplyResult>(
            &mut self,
            cmd: impl FnOnce(Text, SelectionSet) -> R,
        ) -> Option<Vec<String>> {
            let sels = std::mem::take(&mut self.sels);
            let (new_sels, displaced, _cs) =
                self.buf.apply_edit_grouped(sels, &mut self.edit_group, cmd);
            self.sels = new_sels;
            displaced
        }

        fn begin_edit_group(&mut self) {
            let pre_sels = self.sels.clone();
            self.buf.begin_edit_group(&mut self.edit_group, pre_sels);
        }

        fn commit_edit_group(&mut self) {
            let post_sels = self.sels.clone();
            self.buf.commit_edit_group(&mut self.edit_group, post_sels);
        }

        fn undo(&mut self) {
            if let Some((new_sels, _cs)) = self.buf.undo() {
                self.sels = new_sels;
            }
        }

        fn redo(&mut self) {
            if let Some((new_sels, _cs)) = self.buf.redo() {
                self.sels = new_sels;
            }
        }

        fn goto_revision(&mut self, target: crate::core::history::RevisionId) {
            self.buf.goto_revision(&mut self.sels, target);
        }

        fn text(&self) -> &Text {
            self.buf.text()
        }
        fn sels(&self) -> &SelectionSet {
            &self.sels
        }
        fn is_dirty(&self) -> bool {
            self.buf.is_dirty()
        }
        fn mark_saved(&mut self) {
            self.buf.mark_saved();
        }
        fn can_undo(&self) -> bool {
            self.buf.can_undo()
        }
    }

    fn state(d: &DocHelper) -> String {
        serialize_state(d.text(), d.sels())
    }

    fn doc(input: &str) -> DocHelper {
        let (text, sels) = parse_state(input);
        let buf = Buffer::new(text, sels.clone());
        DocHelper {
            buf,
            sels,
            edit_group: None,
        }
    }

    // ── insert_char ───────────────────────────────────────────────────────────

    #[test]
    fn undo_insert_char() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        assert_eq!(state(&d), "x-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    #[test]
    fn redo_insert_char() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        d.undo();
        d.redo();
        assert_eq!(state(&d), "x-[h]>ello\n");
    }

    #[test]
    fn undo_redo_is_identity() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        d.undo();
        d.redo();
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── delete_char_forward ───────────────────────────────────────────────────

    #[test]
    fn undo_delete_char_forward() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(delete_char_forward);
        assert_eq!(state(&d), "-[e]>llo\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── delete_char_backward ──────────────────────────────────────────────────

    #[test]
    fn undo_delete_char_backward() {
        let mut d = doc("hel-[l]>o\n");
        d.apply_edit(delete_char_backward);
        assert_eq!(state(&d), "he-[l]>o\n");
        d.undo();
        assert_eq!(state(&d), "hel-[l]>o\n");
    }

    // ── delete_selection ──────────────────────────────────────────────────────

    #[test]
    fn undo_delete_selection() {
        let mut d = doc("-[hell]>o\n");
        d.apply_edit(delete_selection);
        assert_eq!(state(&d), "-[o]>\n");
        d.undo();
        assert_eq!(state(&d), "-[hell]>o\n");
    }

    // ── paste_after ───────────────────────────────────────────────────────────

    #[test]
    fn undo_paste_after() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| paste_after(b, s, &["XY".to_string()]));
        assert_eq!(state(&d), "hX-[Y]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── paste_before ──────────────────────────────────────────────────────────

    #[test]
    fn undo_paste_before() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| paste_before(b, s, &["XY".to_string()]));
        assert_eq!(state(&d), "X-[Y]>hello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── selection restoration ─────────────────────────────────────────────────

    #[test]
    fn undo_restores_selection_anchor_and_head() {
        let mut d = doc("-[hell]>o\n");
        d.apply_edit(delete_char_forward);
        d.undo();
        assert_eq!(state(&d), "-[hell]>o\n");
    }

    #[test]
    fn undo_restores_backward_selection() {
        let mut d = doc("<[hell]-o\n");
        d.apply_edit(delete_char_forward);
        d.undo();
        assert_eq!(state(&d), "<[hell]-o\n");
    }

    // ── multi-cursor ──────────────────────────────────────────────────────────

    #[test]
    fn undo_multi_cursor_delete() {
        let mut d = doc("-[h]>el-[l]>o\n");
        d.apply_edit(delete_char_forward);
        assert_eq!(state(&d), "-[e]>l-[o]>\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>el-[l]>o\n");
    }

    // ── repeat_edit produces single undo step ─────────────────────────────────

    #[test]
    fn repeat_edit_is_single_undo_step() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| repeat_edit(3, b, s, delete_char_forward));
        assert_eq!(state(&d), "-[l]>o\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo());
    }

    // ── multiple edits and sequential undo/redo ───────────────────────────────

    #[test]
    fn sequential_undo_multiple_edits() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a'));
        d.apply_edit(|b, s| insert_char(b, s, 'b'));
        d.apply_edit(|b, s| insert_char(b, s, 'c'));
        assert_eq!(state(&d), "abc-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "ab-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "a-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo());
    }

    #[test]
    fn undo_at_root_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    #[test]
    fn redo_at_latest_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        d.redo();
        assert_eq!(state(&d), "x-[h]>ello\n");
    }

    // ── branching ─────────────────────────────────────────────────────────────

    #[test]
    fn branching_undo_then_new_edit() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a'));
        d.undo();
        d.apply_edit(|b, s| insert_char(b, s, 'b'));
        assert_eq!(state(&d), "b-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        d.redo();
        assert_eq!(state(&d), "b-[h]>ello\n");
    }

    // ── goto_revision ─────────────────────────────────────────────────────────

    #[test]
    fn goto_revision_same_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        let buf_before = state(&d);
        d.goto_revision(d.buf.history.current_id());
        assert_eq!(state(&d), buf_before);
    }

    #[test]
    fn goto_revision_across_branches_restores_buffer() {
        let mut d = doc("-[L]>orem ipsum dolor sit amet\n");

        d.apply_edit(|b, _s| {
            use crate::core::changeset::ChangeSetBuilder;
            let mut csb = ChangeSetBuilder::new(27);
            csb.retain(6);
            csb.delete(6);
            csb.retain_rest();
            let cs = csb.finish();
            let new_text = cs.apply(&b).unwrap();
            use crate::core::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::collapsed(6));
            (new_text, new_sels, cs)
        });
        let b1_id = d.buf.history.current_id();
        assert_eq!(d.text().to_string(), "Lorem dolor sit amet\n");

        d.apply_edit(|b, _s| {
            use crate::core::changeset::ChangeSetBuilder;
            let mut csb = ChangeSetBuilder::new(21);
            csb.retain(6);
            csb.delete(5);
            csb.insert("foo");
            csb.retain_rest();
            let cs = csb.finish();
            let new_text = cs.apply(&b).unwrap();
            use crate::core::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::collapsed(6));
            (new_text, new_sels, cs)
        });
        assert_eq!(d.text().to_string(), "Lorem foo sit amet\n");

        d.apply_edit(|b, _s| {
            use crate::core::changeset::ChangeSetBuilder;
            let mut csb = ChangeSetBuilder::new(19);
            csb.retain(10);
            csb.delete(3);
            csb.insert("bar");
            csb.retain_rest();
            let cs = csb.finish();
            let new_text = cs.apply(&b).unwrap();
            use crate::core::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::collapsed(10));
            (new_text, new_sels, cs)
        });
        let b3_id = d.buf.history.current_id();
        assert_eq!(d.text().to_string(), "Lorem foo bar amet\n");

        d.undo();
        d.undo();
        assert_eq!(d.buf.history.current_id(), b1_id);
        assert_eq!(d.text().to_string(), "Lorem dolor sit amet\n");

        d.apply_edit(|b, _s| {
            use crate::core::changeset::ChangeSetBuilder;
            let mut csb = ChangeSetBuilder::new(21);
            csb.retain(6);
            csb.delete(6);
            csb.retain_rest();
            let cs = csb.finish();
            let new_text = cs.apply(&b).unwrap();
            use crate::core::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::collapsed(6));
            (new_text, new_sels, cs)
        });
        assert_eq!(d.text().to_string(), "Lorem sit amet\n");

        d.goto_revision(b3_id);
        assert_eq!(d.text().to_string(), "Lorem foo bar amet\n");
        assert_eq!(d.buf.history.current_id(), b3_id);
    }

    #[test]
    fn goto_revision_then_edit_creates_new_branch() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a'));
        d.apply_edit(|b, s| insert_char(b, s, 'b'));
        let rev2 = d.buf.history.current_id();

        d.undo();
        d.undo();

        d.apply_edit(|b, s| insert_char(b, s, 'x'));

        d.goto_revision(rev2);
        assert!(d.text().to_string().starts_with("ab"));

        let before_new_edit = d.buf.history.current_id();
        d.apply_edit(|b, s| insert_char(b, s, 'z'));
        let new_rev = d.buf.history.current_id();
        assert_ne!(new_rev, before_new_edit);
        assert_eq!(d.buf.history.parent(new_rev), Some(rev2));
    }

    #[test]
    fn goto_root_from_deep_branch() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a'));
        d.apply_edit(|b, s| insert_char(b, s, 'b'));
        d.apply_edit(|b, s| insert_char(b, s, 'c'));
        let initial = "-[h]>ello\n";
        d.goto_revision(crate::core::history::RevisionId(0));
        assert_eq!(state(&d), initial);
    }

    // ── edit groups ───────────────────────────────────────────────────────────

    #[test]
    fn grouped_edits_single_undo_step() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'a'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'b'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'c'));
        d.commit_edit_group();
        assert_eq!(state(&d), "abc-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo());
    }

    #[test]
    fn empty_group_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.commit_edit_group();
        assert!(!d.can_undo());
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    #[test]
    fn grouped_edits_with_backspace() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'a'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'b'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'x'));
        d.apply_edit_grouped(delete_char_backward);
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'c'));
        d.commit_edit_group();
        assert_eq!(state(&d), "abc-[h]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo());
    }

    #[test]
    fn grouped_then_normal_edit_two_steps() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'a'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'b'));
        d.commit_edit_group();
        assert_eq!(state(&d), "ab-[h]>ello\n");

        d.apply_edit(|b, s| insert_char(b, s, 'z'));
        assert_eq!(state(&d), "abz-[h]>ello\n");

        d.undo();
        assert_eq!(state(&d), "ab-[h]>ello\n");

        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo());
    }

    #[test]
    fn grouped_edits_redo() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'a'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'b'));
        d.commit_edit_group();
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        d.redo();
        assert_eq!(state(&d), "ab-[h]>ello\n");
    }

    // ── dirty tracking ───────────────────────────────────────────────────────

    #[test]
    fn fresh_doc_is_not_dirty() {
        let d = doc("-[h]>ello\n");
        assert!(!d.is_dirty());
    }

    #[test]
    fn edit_makes_dirty() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        assert!(d.is_dirty());
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        assert!(d.is_dirty());
        d.mark_saved();
        assert!(!d.is_dirty());
    }

    #[test]
    fn undo_to_saved_revision_is_clean() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        d.mark_saved();
        d.apply_edit(|b, s| insert_char(b, s, 'y'));
        assert!(d.is_dirty());
        d.undo();
        assert!(!d.is_dirty());
    }

    #[test]
    fn undo_past_saved_revision_is_dirty() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        d.mark_saved();
        d.undo();
        assert!(d.is_dirty());
    }

    #[test]
    fn grouped_edit_makes_dirty() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'a'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'b'));
        d.commit_edit_group();
        assert!(d.is_dirty());
    }

    // ── apply_edit returns displaced text for paste ───────────────────────────

    #[test]
    fn apply_edit_paste_returns_replaced_text() {
        let mut d = doc("-[hell]>o\n");
        let replaced = d.apply_edit(|b, s| paste_after(b, s, &["XY".to_string()]));
        assert_eq!(replaced, Some(vec!["hell".to_string()]));
    }

    // ── yank + paste roundtrip ────────────────────────────────────────────────

    #[test]
    fn yank_paste_undo() {
        let mut d = doc("-[hell]>o\n");
        let yanked = yank_selections(d.text(), d.sels());
        d.apply_edit(|b, s| paste_after(b, s, &yanked));
        d.undo();
        assert_eq!(state(&d), "-[hell]>o\n");
    }

    // ── set_path invariant ────────────────────────────────────────────────────

    #[test]
    fn set_path_accepts_paths_with_basename() {
        let mut b = Buffer::new(Text::empty(), SelectionSet::default());
        b.set_path(Some(PathBuf::from("/tmp/file.txt")));
        assert_eq!(b.display_name(), "file.txt");
    }

    #[test]
    fn set_path_none_clears_path() {
        let mut b = Buffer::new(Text::empty(), SelectionSet::default());
        b.set_path(Some(PathBuf::from("/tmp/file.txt")));
        b.set_path(None);
        assert!(b.path.is_none());
        assert_eq!(b.display_name(), Buffer::SCRATCH_BUFFER_NAME);
    }

    #[test]
    #[should_panic(expected = "path must have a basename")]
    fn set_path_rejects_root() {
        let mut b = Buffer::new(Text::empty(), SelectionSet::default());
        b.set_path(Some(PathBuf::from("/")));
    }

    #[test]
    #[should_panic(expected = "path must have a basename")]
    fn set_path_rejects_dotdot() {
        let mut b = Buffer::new(Text::empty(), SelectionSet::default());
        b.set_path(Some(PathBuf::from("..")));
    }
}
