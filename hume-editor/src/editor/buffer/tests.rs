use super::*;
use hume_ops::edit::{
    delete_char_backward, delete_char_forward, delete_selection, insert_char, paste_after,
    paste_before, repeat_edit,
};
use hume_ops::register::yank_selections;
use hume_test_fixtures::testing::{parse_state, serialize_state};
use pretty_assertions::assert_eq;

// ── DocHelper ─────────────────────────────────────────────────────────────
//
// Thin test wrapper that keeps a `SelectionSet` alongside the `Buffer` so
// tests get an ergonomic API instead of passing sels at every call site.

struct DocHelper {
    buf: Buffer,
    sels: SelectionSet,
    edit_group: Option<EditGroup>,
}

impl DocHelper {
    fn apply_edit(
        &mut self,
        cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
    ) {
        let sels = std::mem::take(&mut self.sels);
        let (new_sels, _cs) = self.buf.apply_edit(sels, cmd);
        self.sels = new_sels;
    }

    fn apply_edit_grouped(
        &mut self,
        cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
    ) {
        let sels = std::mem::take(&mut self.sels);
        let (new_sels, _cs) = self.buf.apply_edit_grouped(sels, &mut self.edit_group, cmd);
        self.sels = new_sels;
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

    fn goto_revision(&mut self, target: hume_editing::history::RevisionId) {
        self.buf.goto_revision(&mut self.sels, target);
    }

    /// Reload the buffer text in place, preserving history. The current
    /// selections become the stored `pre_sels` (undo restores them);
    /// `post_sels` becomes both the stored post-reload selection and the
    /// helper's live `self.sels`.
    fn reload_from(&mut self, new_text: BufferText, post_sels: SelectionSet) {
        let pre_sels = self.sels.clone();
        self.buf
            .reload_from_text(new_text, pre_sels, post_sels.clone());
        self.sels = post_sels;
    }

    fn text(&self) -> &BufferText {
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
    fn set_undo_levels(&mut self, levels: usize) {
        self.buf.set_undo_levels(levels);
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
    assert_eq!(state(&d), "h-[XY]>ello\n");
    d.undo();
    assert_eq!(state(&d), "-[h]>ello\n");
}

// ── paste_before ──────────────────────────────────────────────────────────

#[test]
fn undo_paste_before() {
    let mut d = doc("-[h]>ello\n");
    d.apply_edit(|b, s| paste_before(b, s, &["XY".to_string()]));
    assert_eq!(state(&d), "-[XY]>hello\n");
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
        use hume_editing::changeset::ChangeSetBuilder;
        let mut csb = ChangeSetBuilder::new(27);
        csb.retain(6);
        csb.delete(6);
        csb.retain_rest();
        let cs = csb.finish();
        let new_text = cs.apply(&b).unwrap();
        use hume_editing::selection::{Selection, SelectionSet};
        let new_sels = SelectionSet::single(Selection::collapsed(6));
        (new_text, new_sels, cs)
    });
    let b1_id = d.buf.history.current_id();
    assert_eq!(d.text().to_string(), "Lorem dolor sit amet\n");

    d.apply_edit(|b, _s| {
        use hume_editing::changeset::ChangeSetBuilder;
        let mut csb = ChangeSetBuilder::new(21);
        csb.retain(6);
        csb.delete(5);
        csb.insert("foo");
        csb.retain_rest();
        let cs = csb.finish();
        let new_text = cs.apply(&b).unwrap();
        use hume_editing::selection::{Selection, SelectionSet};
        let new_sels = SelectionSet::single(Selection::collapsed(6));
        (new_text, new_sels, cs)
    });
    assert_eq!(d.text().to_string(), "Lorem foo sit amet\n");

    d.apply_edit(|b, _s| {
        use hume_editing::changeset::ChangeSetBuilder;
        let mut csb = ChangeSetBuilder::new(19);
        csb.retain(10);
        csb.delete(3);
        csb.insert("bar");
        csb.retain_rest();
        let cs = csb.finish();
        let new_text = cs.apply(&b).unwrap();
        use hume_editing::selection::{Selection, SelectionSet};
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
        use hume_editing::changeset::ChangeSetBuilder;
        let mut csb = ChangeSetBuilder::new(21);
        csb.retain(6);
        csb.delete(6);
        csb.retain_rest();
        let cs = csb.finish();
        let new_text = cs.apply(&b).unwrap();
        use hume_editing::selection::{Selection, SelectionSet};
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
    d.goto_revision(hume_editing::history::History::ROOT);
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

// ── undo-levels ───────────────────────────────────────────────────────────

#[test]
fn promotion_remaps_saved_revision_to_root() {
    // The saved revision is exactly the one `undo-levels` trimming promotes
    // into the root. Undoing past the next edit must still read as saved.
    // Fail oracle: skip the saved_revision remap in record_revision and
    // is_dirty() stays true forever after the promotion.
    let mut d = doc("-[h]>ello\n");
    d.apply_edit(|b, s| insert_char(b, s, 'x'));
    let saved_state = state(&d);
    d.mark_saved();
    d.set_undo_levels(1);
    d.apply_edit(|b, s| insert_char(b, s, 'y'));
    assert!(d.is_dirty());

    d.undo();
    assert_eq!(state(&d), saved_state);
    assert!(!d.is_dirty());
}

#[test]
fn promotion_overwriting_root_invalidates_saved_revision() {
    // The buffer is opened and never saved since (saved_revision == ROOT).
    // A promotion overwrites ROOT's content with a later revision's, so the
    // saved id must stop reading as clean even though it's still `ROOT` —
    // ROOT no longer represents the state it was saved at.
    // Fail oracle: if record_revision only remapped saved_revision when it
    // equals the promoted node (never invalidating a saved-at-ROOT id),
    // undoing back to the new root would wrongly read clean here even
    // though the buffer's text ('xhello') differs from the saved state
    // ('hello').
    let mut d = doc("-[h]>ello\n");
    d.set_undo_levels(1);
    d.apply_edit(|b, s| insert_char(b, s, 'x'));
    d.apply_edit(|b, s| insert_char(b, s, 'y'));

    d.undo();
    assert!(d.is_dirty());
}

#[test]
fn evicted_saved_revision_stays_dirty() {
    // The saved revision sits inside a branch that gets evicted outright
    // (not promoted) when a sibling branch grows past the cap. Since
    // RevisionIds are never reused, the buffer must never spontaneously
    // read as clean again until an explicit mark_saved.
    // Fail oracle: if a future revision could reuse the evicted saved
    // revision's numeric id, is_dirty() would wrongly read false.
    let mut d = doc("-[h]>ello\n");
    d.apply_edit(|b, s| insert_char(b, s, 'a'));
    d.apply_edit(|b, s| insert_char(b, s, 'b'));
    d.mark_saved();
    d.undo();
    d.undo();
    d.apply_edit(|b, s| insert_char(b, s, 'c'));
    d.set_undo_levels(1);
    d.apply_edit(|b, s| insert_char(b, s, 'd'));
    assert!(d.is_dirty());

    d.apply_edit(|b, s| insert_char(b, s, 'e'));
    assert!(d.is_dirty());

    d.mark_saved();
    assert!(!d.is_dirty());
}

#[test]
fn undo_after_eviction_stops_at_new_root() {
    // Cap 2 promotes 'a' into the root once 'c' is recorded. Undoing twice
    // from 'c' must still reproduce a's post-edit text (b's and c's inverse
    // transactions are untouched by promotion) and land at the new root,
    // where a further undo is a safe no-op.
    // Fail oracle: if promotion corrupted the parent chain or an inverse
    // transaction, this would land on the wrong text or panic.
    let mut d = doc("-[h]>ello\n");
    d.set_undo_levels(2);
    d.apply_edit(|b, s| insert_char(b, s, 'a'));
    let state_after_a = state(&d);
    d.apply_edit(|b, s| insert_char(b, s, 'b'));
    d.apply_edit(|b, s| insert_char(b, s, 'c'));

    d.undo();
    d.undo();
    assert_eq!(state(&d), state_after_a);
    assert!(!d.can_undo());
    assert!(d.is_dirty()); // never saved; promotion overwrote ROOT's content

    d.undo(); // no-op at the new root, must not panic
    assert_eq!(state(&d), state_after_a);
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
    let mut b = Buffer::new(BufferText::empty(), SelectionSet::default());
    b.set_path(Some(PathBuf::from("/tmp/file.txt")));
    assert_eq!(b.display_name(), "file.txt");
}

#[test]
fn set_path_none_clears_path() {
    let mut b = Buffer::new(BufferText::empty(), SelectionSet::default());
    b.set_path(Some(PathBuf::from("/tmp/file.txt")));
    b.set_path(None);
    assert!(b.path().is_none());
    assert_eq!(b.display_name(), Buffer::SCRATCH_BUFFER_NAME);
}

#[test]
fn set_path_derives_display_path() {
    let mut b = Buffer::new(BufferText::empty(), SelectionSet::default());
    assert!(b.display_path().is_none());
    b.set_path(Some(PathBuf::from("/tmp/file.txt")));
    assert_eq!(
        b.display_path(),
        Some(hume_platform::path::display_form(Path::new("/tmp/file.txt")).as_str())
    );
}

#[test]
fn set_path_none_clears_display_path() {
    let mut b = Buffer::new(BufferText::empty(), SelectionSet::default());
    b.set_path(Some(PathBuf::from("/tmp/file.txt")));
    b.set_path(None);
    assert!(b.display_path().is_none());
}

#[test]
#[should_panic(expected = "path must have a basename")]
fn set_path_rejects_root() {
    let mut b = Buffer::new(BufferText::empty(), SelectionSet::default());
    b.set_path(Some(PathBuf::from("/")));
}

#[test]
#[should_panic(expected = "path must have a basename")]
fn set_path_rejects_dotdot() {
    let mut b = Buffer::new(BufferText::empty(), SelectionSet::default());
    b.set_path(Some(PathBuf::from("..")));
}

// ── text_gen ──────────────────────────────────────────────────────────────

#[test]
fn text_gen_starts_at_zero() {
    let b = Buffer::new(BufferText::empty(), SelectionSet::default());
    assert_eq!(b.text_gen, 0);
}

#[test]
fn text_gen_bumped_by_apply_edit() {
    let mut d = doc("-[h]>ello\n");
    let before = d.buf.text_gen;
    d.apply_edit(|b, s| insert_char(b, s, 'x'));
    assert_eq!(d.buf.text_gen, before + 1);
}

#[test]
fn text_gen_bumped_by_apply_edit_grouped() {
    let mut d = doc("-[h]>ello\n");
    d.begin_edit_group();
    let before = d.buf.text_gen;
    d.apply_edit_grouped(|b, s| insert_char(b, s, 'x'));
    assert_eq!(d.buf.text_gen, before + 1, "each grouped edit bumps gen");
}

#[test]
fn text_gen_bumped_by_undo() {
    let mut d = doc("-[h]>ello\n");
    d.apply_edit(|b, s| insert_char(b, s, 'x'));
    let before = d.buf.text_gen;
    d.undo();
    assert_eq!(d.buf.text_gen, before + 1);
}

#[test]
fn text_gen_bumped_by_redo() {
    let mut d = doc("-[h]>ello\n");
    d.apply_edit(|b, s| insert_char(b, s, 'x'));
    d.undo();
    let before = d.buf.text_gen;
    d.redo();
    assert_eq!(d.buf.text_gen, before + 1);
}

#[test]
fn text_gen_not_bumped_when_undo_at_root() {
    let mut d = doc("-[h]>ello\n");
    let before = d.buf.text_gen;
    d.undo(); // nothing to undo — no-op
    assert_eq!(d.buf.text_gen, before, "no-op undo must not bump gen");
}

// ── reload_from_text ────────────────────────────────────────────────────

#[test]
fn reload_from_text_keeps_buffer_not_dirty() {
    let mut d = doc("-[a]>lpha\nbeta\ngamma\n");
    assert!(!d.is_dirty());
    d.reload_from(BufferText::from("alpha\nBETA\ngamma\n"), SelectionSet::default());
    assert!(!d.is_dirty(), "freshly reloaded buffer is clean");
    assert_eq!(d.text().to_string(), "alpha\nBETA\ngamma\n");
}

#[test]
fn reload_from_text_is_undoable() {
    let mut d = doc("hel-[l]>o\n");
    let pre_state = state(&d);
    d.reload_from(BufferText::from("hello world\n"), SelectionSet::default());
    assert!(d.can_undo(), "reload recorded a revision");
    assert_eq!(d.text().to_string(), "hello world\n");

    d.undo();
    // Undo restores pre-reload text AND pre-reload selections.
    assert_eq!(state(&d), pre_state);
    assert!(!d.can_undo(), "undoing the reload reaches the root");
    assert!(d.is_dirty(), "undo past the saved reload revision is dirty");
}

#[test]
fn reload_from_text_redo_reapplies_reload() {
    let mut d = doc("hel-[l]>o\n");
    d.reload_from(BufferText::from("hello world\n"), SelectionSet::default());
    d.undo();
    assert_eq!(d.text().to_string(), "hello\n");
    d.redo();
    assert_eq!(d.text().to_string(), "hello world\n");
    assert!(!d.is_dirty(), "redo lands on the saved reload revision");
}

#[test]
fn reload_from_text_then_edit_branches_off_old_tree() {
    // edit → reload → undo → new edit: the new edit must branch as a
    // sibling of the reload (tree-monotonicity invariant), reachable via
    // `current_id`/redo. Mirrors `branching_preserves_old_path` in
    // history.rs.
    let mut d = doc("-[h]>ello\n");
    d.apply_edit(|b, s| insert_char(b, s, '1'));
    let after_first_edit = d.buf.history.current_id();
    d.reload_from(BufferText::from("hello world\n"), SelectionSet::default());
    let reload_rev = d.buf.history.current_id();
    assert_ne!(reload_rev, after_first_edit);

    // Undo the reload (back to the first-edit revision), then make a new
    // edit — it becomes a sibling of the reload.
    d.undo();
    assert_eq!(d.buf.history.current_id(), after_first_edit);
    d.apply_edit(|b, s| insert_char(b, s, '2'));
    let branched_rev = d.buf.history.current_id();
    assert_ne!(branched_rev, reload_rev);
    assert_eq!(d.buf.history.parent(branched_rev), Some(after_first_edit));

    // Redo from here jumps to the new branch, not the reload.
    d.undo();
    d.redo();
    assert_eq!(d.buf.history.current_id(), branched_rev);
}

#[test]
fn reload_from_text_noop_when_unchanged() {
    // Identical old/new → identity forward CS. Reload records NO revision
    // (a no-op `:e!` must not litter the undo tree) and leaves the buffer
    // clean and at the same revision it started on.
    let mut d = doc("-[s]>ame\ncontent\n");
    let before = d.buf.history.current_id();
    d.reload_from(BufferText::from("same\ncontent\n"), SelectionSet::default());
    assert!(!d.can_undo(), "no-op reload records no undo step");
    assert_eq!(d.buf.history.current_id(), before, "revision unchanged");
    assert!(!d.is_dirty());
    assert_eq!(d.text().to_string(), "same\ncontent\n");
}

#[test]
fn reload_from_text_inverse_is_fine_grained() {
    // Pin the "fine-grained, not coarse" property at the Buffer layer: a
    // single-line change's inverse re-inserts only the changed line, not a
    // full-buffer delete-all. The inverse is what `undo` returns.
    use hume_editing::changeset::Operation;
    let mut d = doc("-[a]>lpha\nbeta\ngamma\n");
    d.reload_from(BufferText::from("alpha\nBETA\ngamma\n"), SelectionSet::default());
    let (_, inv_cs) = d
        .buf
        .undo()
        .expect("undo after reload returns the inverse CS");
    let has_small_insert = inv_cs
        .ops()
        .iter()
        .any(|op| matches!(op, Operation::Insert(s) if s == "beta\n"));
    assert!(
        has_small_insert,
        "reload inverse should re-insert only the changed line, got {:?}",
        inv_cs.ops(),
    );
}
