use crate::buffer::Buffer;
use crate::changeset::ChangeSet;
use crate::history::History;
use crate::selection::SelectionSet;

/// A document: the current buffer, cursor state, and undo history — together.
///
/// `Document` is the core "open file" abstraction. All text edits go through
/// [`apply_edit`] or [`apply_paste`], which handle undo bookkeeping
/// automatically: before applying the edit, the inverse ChangeSet is computed
/// against the pre-edit buffer, and both the forward and inverse Transactions
/// are recorded in the undo tree.
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
pub(crate) struct Document {
    buf: Buffer,
    sels: SelectionSet,
    history: History,
}

impl Document {
    /// Create a new document from a buffer and initial selection state.
    pub(crate) fn new(buf: Buffer, sels: SelectionSet) -> Self {
        let buf_len = buf.len_chars();
        let history = History::new(sels.clone(), buf_len);
        Self { buf, sels, history }
    }

    /// Apply an edit command and record it in the undo history.
    ///
    /// The closure receives `(Buffer, SelectionSet)` and must return
    /// `(Buffer, SelectionSet, ChangeSet)`. This is the return type of all
    /// public edit functions in [`crate::edit`] (after the ChangeSet refactor).
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
    pub(crate) fn apply_edit(
        &mut self,
        cmd: impl FnOnce(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet),
    ) {
        let old_sels = self.sels.clone();
        // Clone the buffer for the edit. O(log n) — Ropey structural sharing.
        let (new_buf, new_sels, cs) = cmd(self.buf.clone(), self.sels.clone());

        // self.buf is still the pre-edit buffer here — safe to call invert.
        // invert() needs the original content to reconstruct deleted text.
        let inverse_cs = cs.invert(&self.buf);

        self.history.record(cs, inverse_cs, old_sels, new_sels.clone());
        self.buf = new_buf;
        self.sels = new_sels;
    }

    /// Apply a paste command (which returns captured displaced text) and record
    /// it in the undo history.
    ///
    /// Returns the displaced text (`replaced[i]` = text that was overwritten by
    /// selection `i`; empty for cursor selections). The caller can write this
    /// back to a register.
    ///
    /// Identical undo semantics as [`apply_edit`].
    pub(crate) fn apply_paste(
        &mut self,
        cmd: impl FnOnce(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet, Vec<String>),
    ) -> Vec<String> {
        let old_sels = self.sels.clone();
        let (new_buf, new_sels, cs, replaced) = cmd(self.buf.clone(), self.sels.clone());
        let inverse_cs = cs.invert(&self.buf);
        self.history.record(cs, inverse_cs, old_sels, new_sels.clone());
        self.buf = new_buf;
        self.sels = new_sels;
        replaced
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
        d.apply_paste(|b, s| paste_after(b, s, &["XY".to_string()]));
        assert_eq!(state(&d), "hX-[Y]>ello\n");
        d.undo();
        assert_eq!(state(&d), "-[h]>ello\n");
    }

    // ── paste_before ──────────────────────────────────────────────────────────

    #[test]
    fn undo_paste_before() {
        let mut d = doc("-[h]>ello\n");
        d.apply_paste(|b, s| paste_before(b, s, &["XY".to_string()]));
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

    // ── apply_paste returns displaced text ───────────────────────────────────

    #[test]
    fn apply_paste_returns_replaced_text() {
        let mut d = doc("-[hell]>o\n");
        let replaced = d.apply_paste(|b, s| paste_after(b, s, &["XY".to_string()]));
        // Multi-char selection was replaced; displaced text = "hell".
        assert_eq!(replaced, vec!["hell"]);
    }

    // ── yank + paste roundtrip ────────────────────────────────────────────────

    #[test]
    fn yank_paste_undo() {
        let mut d = doc("-[hell]>o\n");
        let yanked = yank_selections(d.buf(), d.sels());
        d.apply_paste(|b, s| paste_after(b, s, &yanked));
        // "hell" pasted after the selection: "hell" + "hell" + "o".
        d.undo();
        assert_eq!(state(&d), "-[hell]>o\n");
    }
}
