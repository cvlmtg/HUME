use crate::buffer::Buffer;
use crate::changeset::ChangeSet;
use crate::history::History;
use crate::selection::SelectionSet;

// ── IntoApplyResult ───────────────────────────────────────────────────────────

/// Converts a closure's return value into the canonical `(Buffer, SelectionSet,
/// ChangeSet, Vec<String>)` quad that `Document::apply_edit` needs.
///
/// Implemented for both the 3-tuple `(Buffer, SelectionSet, ChangeSet)` that
/// plain edit functions return, and the 4-tuple
/// `(Buffer, SelectionSet, ChangeSet, Vec<String>)` that paste functions return.
/// This lets `apply_edit` accept either without wrapping at the call site.
pub(crate) trait IntoApplyResult {
    fn into_apply_result(self) -> (Buffer, SelectionSet, ChangeSet, Vec<String>);
}

impl IntoApplyResult for (Buffer, SelectionSet, ChangeSet) {
    fn into_apply_result(self) -> (Buffer, SelectionSet, ChangeSet, Vec<String>) {
        (self.0, self.1, self.2, vec![])
    }
}

impl IntoApplyResult for (Buffer, SelectionSet, ChangeSet, Vec<String>) {
    fn into_apply_result(self) -> (Buffer, SelectionSet, ChangeSet, Vec<String>) {
        self
    }
}

// ── Document ──────────────────────────────────────────────────────────────────

/// A document: the current buffer, cursor state, and undo history — together.
///
/// `Document` is the core "open file" abstraction. All text edits go through
/// [`apply_edit`], which handles undo bookkeeping automatically: before
/// applying the edit, the inverse ChangeSet is computed against the pre-edit
/// buffer, and both the forward and inverse Transactions are recorded in the
/// undo tree.
///
/// ## Undo timing
///
/// [`ChangeSet::invert`] must be called against the buffer *before* the edit
/// is applied (it reads deleted text from the original buffer). `Document`
/// handles this invariant internally: it clones the buffer before passing it
/// to the edit command, so `self.buf` still holds the pre-edit content when
/// `invert` is called.
///
/// ## Buffer cloning
///
/// `buf.clone()` is O(log n) — Ropey uses Arc-based structural sharing, so
/// cloning a Rope shares the underlying data. This makes the Document approach
/// cheap: we don't snapshot the buffer for undo (we use changeset inversion),
/// but cloning for the edit call is affordable.
///
/// ## Edit groups
///
/// `begin_edit_group` / `commit_edit_group` bracket a series of edits that
/// should undo as a single step. During an open group, `apply_edit_grouped`
/// updates `self.buf` and `self.sels` normally (so the user sees their edits)
/// but composes changesets into an accumulator instead of recording individual
/// history entries. `commit_edit_group` inverts the composed changeset against
/// the pre-group buffer snapshot and records exactly one revision. Used by
/// insert mode so that an entire insert session undoes as one step.
pub(crate) struct Document {
    buf: Buffer,
    sels: SelectionSet,
    history: History,
    /// Non-`None` while an edit group is open (i.e. while in insert mode).
    group: Option<EditGroup>,
}

/// Accumulated state for an open edit group.
struct EditGroup {
    /// Buffer snapshot taken at `begin_edit_group` — used by `invert` in `commit_edit_group`.
    buf_snapshot: Buffer,
    /// Selection snapshot taken at `begin_edit_group` — restored by undo.
    sels_snapshot: SelectionSet,
    /// Running composition of all forward changesets applied since the group opened.
    /// `None` means no edits have been applied yet (empty group).
    cs: Option<ChangeSet>,
}

impl Document {
    /// Create a new document from a buffer and initial selection state.
    pub(crate) fn new(buf: Buffer, sels: SelectionSet) -> Self {
        let buf_len = buf.len_chars();
        let history = History::new(sels.clone(), buf_len);
        Self { buf, sels, history, group: None }
    }

    /// Apply an edit command and record it in the undo history.
    ///
    /// The closure receives `(Buffer, SelectionSet)` and may return either a
    /// 3-tuple `(Buffer, SelectionSet, ChangeSet)` (plain edits) or a 4-tuple
    /// `(Buffer, SelectionSet, ChangeSet, Vec<String>)` (paste, which also
    /// captures displaced text). Both are accepted via [`IntoApplyResult`].
    ///
    /// Returns the displaced text — empty for non-paste edits, populated for
    /// paste so the caller can write it to a register.
    ///
    /// ## Undo bookkeeping
    ///
    /// `apply_edit` is the single place where the undo invariant is enforced:
    ///
    /// 1. The pre-edit buffer clone is passed to the closure.
    /// 2. `self.buf` is still the pre-edit buffer when `invert` is called.
    /// 3. Both forward and inverse Transactions are recorded in `self.history`.
    /// 4. `self.buf` and `self.sels` are updated to the post-edit state.
    ///
    /// Calling this method means "this edit is one undo step". If the caller
    /// uses [`crate::edit::repeat_edit`] inside the closure, all N iterations
    /// are composed into one ChangeSet, so the whole repetition undoes in one
    /// step.
    pub(crate) fn apply_edit<R: IntoApplyResult>(
        &mut self,
        cmd: impl FnOnce(Buffer, SelectionSet) -> R,
    ) -> Vec<String> {
        let old_sels = self.sels.clone();
        // Clone the buffer for the edit. O(log n) — Ropey structural sharing.
        let (new_buf, new_sels, cs, captured) =
            cmd(self.buf.clone(), self.sels.clone()).into_apply_result();

        // self.buf is still the pre-edit buffer here — safe to call invert.
        // invert() needs the original content to reconstruct deleted text.
        let inverse_cs = cs.invert(&self.buf);

        self.history.record(cs, inverse_cs, old_sels, new_sels.clone());
        self.buf = new_buf;
        self.sels = new_sels;
        captured
    }

    /// Open an edit group. All subsequent `apply_edit_grouped` calls will be
    /// accumulated and recorded as a single undo step when `commit_edit_group`
    /// is called. Snapshots the current buffer and selections.
    ///
    /// Calling `begin_edit_group` while a group is already open is a logic
    /// error; it asserts in debug builds and replaces the snapshot in release.
    pub(crate) fn begin_edit_group(&mut self) {
        debug_assert!(self.group.is_none(), "begin_edit_group called with group already open");
        self.group = Some(EditGroup {
            buf_snapshot: self.buf.clone(),
            sels_snapshot: self.sels.clone(),
            cs: None,
        });
    }

    /// Apply an edit within the current open group.
    ///
    /// Identical to `apply_edit` except the changeset is composed into the
    /// group accumulator rather than recorded directly in the undo history.
    /// `self.buf` and `self.sels` are updated so the user sees the edit.
    ///
    /// Panics if called without an open group (i.e. `begin_edit_group` was not
    /// called first).
    pub(crate) fn apply_edit_grouped<R: IntoApplyResult>(
        &mut self,
        cmd: impl FnOnce(Buffer, SelectionSet) -> R,
    ) -> Vec<String> {
        let group = self.group.as_mut().expect("apply_edit_grouped called without an open group");

        let (new_buf, new_sels, cs, captured) =
            cmd(self.buf.clone(), self.sels.clone()).into_apply_result();

        // Compose this changeset into the accumulator.
        group.cs = Some(match group.cs.take() {
            None => cs,
            Some(acc) => acc.compose(cs),
        });

        self.buf = new_buf;
        self.sels = new_sels;
        captured
    }

    /// Close the current edit group and record it as a single undo step.
    ///
    /// If no edits were applied since `begin_edit_group` (empty group), no
    /// revision is recorded. Clears the group state regardless.
    ///
    /// Panics if called without an open group.
    pub(crate) fn commit_edit_group(&mut self) {
        let group = self.group.take().expect("commit_edit_group called without an open group");

        // Only record a revision if something was actually edited.
        if let Some(cs) = group.cs {
            // invert() needs the pre-group buffer — that's exactly the snapshot.
            let inverse_cs = cs.invert(&group.buf_snapshot);
            self.history.record(cs, inverse_cs, group.sels_snapshot, self.sels.clone());
        }
    }

    /// Undo the last edit. No-op at the root (nothing to undo).
    pub(crate) fn undo(&mut self) {
        if let Some(txn) = self.history.undo() {
            let (new_buf, new_sels) = txn
                .apply(&self.buf)
                .expect("inverse transaction failed — history is corrupt");
            self.buf = new_buf;
            self.sels = new_sels;
        }
    }

    /// Jump to an arbitrary revision in the undo tree.
    ///
    /// Applies the necessary inverse/forward transactions sequentially to
    /// transform the buffer from the current state to the target state.
    /// No-op if `target` is the current revision or out of bounds.
    pub(crate) fn goto_revision(&mut self, target: crate::history::RevisionId) {
        if let Some(transactions) = self.history.goto_revision(target) {
            for txn in transactions {
                let (new_buf, new_sels) = txn
                    .apply(&self.buf)
                    .expect("goto_revision transaction failed — history is corrupt");
                self.buf = new_buf;
                self.sels = new_sels;
            }
        }
    }

    /// Redo the most recent undone edit. No-op if at the latest revision.
    pub(crate) fn redo(&mut self) {
        if let Some(txn) = self.history.redo() {
            let (new_buf, new_sels) = txn
                .apply(&self.buf)
                .expect("forward transaction failed — history is corrupt");
            self.buf = new_buf;
            self.sels = new_sels;
        }
    }

    /// The current buffer contents.
    pub(crate) fn buf(&self) -> &Buffer {
        &self.buf
    }

    /// The current selection state.
    pub(crate) fn sels(&self) -> &SelectionSet {
        &self.sels
    }

    /// Replace the current selection state without recording an undo entry.
    ///
    /// Use this for motion commands: they move the cursor but do not modify
    /// the buffer, so there is nothing to record in the undo history.
    /// Edits that also change the buffer must go through [`apply_edit`].
    pub(crate) fn set_selections(&mut self, sels: SelectionSet) {
        self.sels = sels;
    }

    /// True if there is at least one edit to undo.
    pub(crate) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// True if there is at least one undone edit to redo.
    pub(crate) fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{
        delete_char_backward, delete_char_forward, delete_selection, insert_char, paste_after,
        paste_before, repeat_edit,
    };
    use crate::register::yank_selections;
    use crate::testing::{parse_state, serialize_state};
    use pretty_assertions::assert_eq;

    // ── Helper ────────────────────────────────────────────────────────────────

    fn state(doc: &Document) -> String {
        serialize_state(doc.buf(), doc.sels())
    }

    fn doc(input: &str) -> Document {
        let (buf, sels) = parse_state(input);
        Document::new(buf, sels)
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
        // Back to initial state.
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── delete_char_forward ───────────────────────────────────────────────────

    #[test]
    fn undo_delete_char_forward() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| delete_char_forward(b, s));
        assert_eq!(state(&d), "-[e]>llo\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── delete_char_backward ──────────────────────────────────────────────────

    #[test]
    fn undo_delete_char_backward() {
        let mut d = doc("hel-[l]>o\n");
        d.apply_edit(|b, s| delete_char_backward(b, s));
        assert_eq!(state(&d), "he-[l]>o\n");
        d.undo();
        assert_eq!(state(&d), "hel-[l]>o\n");
    }

    // ── delete_selection ──────────────────────────────────────────────────────

    #[test]
    fn undo_delete_selection() {
        let mut d = doc("-[hell]>o\n");
        d.apply_edit(|b, s| delete_selection(b, s));
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
        // Start with a forward selection; after delete it collapses; undo restores it.
        let mut d = doc("-[hell]>o\n");
        d.apply_edit(|b, s| delete_char_forward(b, s));
        d.undo();
        // Selection should be restored exactly (anchor=0, head=3).
        assert_eq!(state(&d), "-[hell]>o\n");
    }

    #[test]
    fn undo_restores_backward_selection() {
        let mut d = doc("<[hell]-o\n");
        d.apply_edit(|b, s| delete_char_forward(b, s));
        d.undo();
        assert_eq!(state(&d), "<[hell]-o\n");
    }

    // ── multi-cursor ──────────────────────────────────────────────────────────

    #[test]
    fn undo_multi_cursor_delete() {
        let mut d = doc("-[h]>el-[l]>o\n");
        d.apply_edit(|b, s| delete_char_forward(b, s));
        // Both 'h' and second 'l' deleted.
        assert_eq!(state(&d), "-[e]>l-[o]>\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>el-[l]>o\n");
    }

    // ── repeat_edit produces single undo step ─────────────────────────────────

    #[test]
    fn repeat_edit_is_single_undo_step() {
        let mut d = doc("-[h]>ello\n");
        // Delete 3 chars forward as one undo step.
        d.apply_edit(|b, s| repeat_edit(3, b, s, delete_char_forward));
        assert_eq!(state(&d), "-[l]>o\n");
        // One undo should restore the full pre-repeat state.
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo()); // only one step was recorded
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
        d.undo(); // should not panic
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    #[test]
    fn redo_at_latest_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        d.redo(); // no children yet — should not panic
        assert_eq!(state(&d), "x-[h]>ello\n");
    }

    // ── branching ─────────────────────────────────────────────────────────────

    #[test]
    fn branching_undo_then_new_edit() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a')); // branch A
        d.undo(); // back to root
        d.apply_edit(|b, s| insert_char(b, s, 'b')); // branch B

        // Current state is branch B.
        assert_eq!(state(&d), "b-[h]>ello\n");

        // Undo goes back to root.
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");

        // Redo goes to the most recent branch (B).
        d.redo();
        assert_eq!(state(&d), "b-[h]>ello\n");
    }

    // ── goto_revision ─────────────────────────────────────────────────────────

    #[test]
    fn goto_revision_same_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'x'));
        let buf_before = state(&d);
        d.goto_revision(d.history.current_id());
        assert_eq!(state(&d), buf_before);
    }

    #[test]
    fn goto_revision_across_branches_restores_buffer() {
        // Reproduce the __undo__.txt scenario and jump C2 → B3.
        //
        // B0: "Lorem ipsum dolor sit amet\n"
        // B1: delete "ipsum " → "Lorem dolor sit amet\n"
        // B2: change "dolor" → "foo" → "Lorem foo sit amet\n"
        // B3: change "sit" → "bar" → "Lorem foo bar amet\n"
        // Undo to B1, then:
        // C2: delete "dolor " → "Lorem sit amet\n"
        //
        // From C2, goto B3 → buffer should be "Lorem foo bar amet\n".
        let mut d = doc("-[L]>orem ipsum dolor sit amet\n");

        // B1: delete "ipsum "
        d.apply_edit(|b, _s| {
            use crate::changeset::ChangeSetBuilder;
            // "Lorem ipsum dolor sit amet\n" is 27 chars.
            // Delete chars 6..12 ("ipsum ") → "Lorem dolor sit amet\n"
            let mut csb = ChangeSetBuilder::new(27);
            csb.retain(6);
            csb.delete(6); // "ipsum "
            csb.retain_rest();
            let cs = csb.finish();
            let new_buf = cs.apply(&b).unwrap();
            // Cursor at pos 6 (on 'd').
            use crate::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::cursor(6));
            (new_buf, new_sels, cs)
        });
        let b1_id = d.history.current_id();
        assert_eq!(d.buf().to_string(), "Lorem dolor sit amet\n");

        // B2: change "dolor" → "foo"
        d.apply_edit(|b, _s| {
            use crate::changeset::ChangeSetBuilder;
            // "Lorem dolor sit amet\n" is 21 chars. "dolor" at 6..11.
            let mut csb = ChangeSetBuilder::new(21);
            csb.retain(6);
            csb.delete(5); // "dolor"
            csb.insert("foo");
            csb.retain_rest();
            let cs = csb.finish();
            let new_buf = cs.apply(&b).unwrap();
            use crate::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::cursor(6));
            (new_buf, new_sels, cs)
        });
        assert_eq!(d.buf().to_string(), "Lorem foo sit amet\n");

        // B3: change "sit" → "bar"
        d.apply_edit(|b, _s| {
            use crate::changeset::ChangeSetBuilder;
            // "Lorem foo sit amet\n" is 19 chars. "sit" at 10..13.
            let mut csb = ChangeSetBuilder::new(19);
            csb.retain(10);
            csb.delete(3); // "sit"
            csb.insert("bar");
            csb.retain_rest();
            let cs = csb.finish();
            let new_buf = cs.apply(&b).unwrap();
            use crate::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::cursor(10));
            (new_buf, new_sels, cs)
        });
        let b3_id = d.history.current_id();
        assert_eq!(d.buf().to_string(), "Lorem foo bar amet\n");

        // Undo twice to B1.
        d.undo();
        d.undo();
        assert_eq!(d.history.current_id(), b1_id);
        assert_eq!(d.buf().to_string(), "Lorem dolor sit amet\n");

        // C2: delete "dolor " → "Lorem sit amet\n"
        d.apply_edit(|b, _s| {
            use crate::changeset::ChangeSetBuilder;
            // "Lorem dolor sit amet\n" is 21 chars. "dolor " at 6..12.
            let mut csb = ChangeSetBuilder::new(21);
            csb.retain(6);
            csb.delete(6); // "dolor "
            csb.retain_rest();
            let cs = csb.finish();
            let new_buf = cs.apply(&b).unwrap();
            use crate::selection::{Selection, SelectionSet};
            let new_sels = SelectionSet::single(Selection::cursor(6));
            (new_buf, new_sels, cs)
        });
        assert_eq!(d.buf().to_string(), "Lorem sit amet\n");

        // Jump from C2 to B3.
        d.goto_revision(b3_id);
        assert_eq!(d.buf().to_string(), "Lorem foo bar amet\n");
        assert_eq!(d.history.current_id(), b3_id);
    }

    #[test]
    fn goto_revision_then_edit_creates_new_branch() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a')); // rev1
        d.apply_edit(|b, s| insert_char(b, s, 'b')); // rev2
        let rev2 = d.history.current_id();

        d.undo();
        d.undo(); // back to root

        d.apply_edit(|b, s| insert_char(b, s, 'x')); // rev3 (branch from root)

        // Jump to rev2.
        d.goto_revision(rev2);
        assert!(d.buf().to_string().starts_with("ab"));

        // Make a new edit from rev2 — should create a new branch.
        let before_new_edit = d.history.current_id();
        d.apply_edit(|b, s| insert_char(b, s, 'z'));
        let new_rev = d.history.current_id();
        assert_ne!(new_rev, before_new_edit);
        // Parent of new_rev should be rev2.
        assert_eq!(d.history.parent(new_rev), Some(rev2));
    }

    #[test]
    fn goto_root_from_deep_branch() {
        let mut d = doc("-[h]>ello\n");
        d.apply_edit(|b, s| insert_char(b, s, 'a'));
        d.apply_edit(|b, s| insert_char(b, s, 'b'));
        d.apply_edit(|b, s| insert_char(b, s, 'c'));
        let initial = "-[h]>ello\n";

        d.goto_revision(crate::history::RevisionId(0));
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

        // Only one undo step was recorded.
        assert!(!d.can_undo());
    }

    #[test]
    fn empty_group_is_noop() {
        let mut d = doc("-[h]>ello\n");
        d.begin_edit_group();
        d.commit_edit_group();

        // No revision was recorded.
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
        d.apply_edit_grouped(|b, s| delete_char_backward(b, s)); // fix typo
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'c'));
        d.commit_edit_group();

        assert_eq!(state(&d), "abc-[h]>ello\n");

        // Single undo restores all the way back.
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
        assert!(!d.can_undo());
    }

    #[test]
    fn grouped_then_normal_edit_two_steps() {
        let mut d = doc("-[h]>ello\n");

        // Insert mode session (grouped): types "ab", cursor ends at position 2.
        d.begin_edit_group();
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'a'));
        d.apply_edit_grouped(|b, s| insert_char(b, s, 'b'));
        d.commit_edit_group();
        assert_eq!(state(&d), "ab-[h]>ello\n");

        // Normal mode edit (ungrouped): inserts at cursor position 2.
        d.apply_edit(|b, s| insert_char(b, s, 'z'));
        assert_eq!(state(&d), "abz-[h]>ello\n");

        // First undo removes only the normal-mode 'z'.
        d.undo();
        assert_eq!(state(&d), "ab-[h]>ello\n");

        // Second undo removes the entire insert session.
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

    // ── apply_edit returns displaced text for paste ───────────────────────────

    #[test]
    fn apply_edit_paste_returns_replaced_text() {
        let mut d = doc("-[hell]>o\n");
        let replaced = d.apply_edit(|b, s| paste_after(b, s, &["XY".to_string()]));
        // Multi-char selection was replaced; displaced text = "hell".
        assert_eq!(replaced, vec!["hell"]);
    }

    // ── yank + paste roundtrip ────────────────────────────────────────────────

    #[test]
    fn yank_paste_undo() {
        let mut d = doc("-[hell]>o\n");
        let yanked = yank_selections(d.buf(), d.sels());
        d.apply_edit(|b, s| paste_after(b, s, &yanked));
        // "hell" pasted after the selection: "hell" + "hell" + "o".
        d.undo();
        assert_eq!(state(&d), "-[hell]>o\n");
    }
}
