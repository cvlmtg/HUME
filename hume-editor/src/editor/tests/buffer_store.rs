use super::*;
use pretty_assertions::assert_eq;

// ── Phase 6 — BufferStore + buffer choke-points ───────────────────────────────

use crate::editor::doc_ops;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;

/// `open_buffer` allocates a new BufferId, seeds pane_state, and tracks MRU.
#[test]
fn p6_open_buffer_seeds_pane_state() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("hello\n"), SelectionSet::default()));
    let initial_bid = ed.focused_buffer_id();
    let doc2 = Buffer::new(Text::from("world\n"), SelectionSet::default());
    let bid2 = ed.open_buffer(doc2);
    assert_ne!(bid2, initial_bid);
    // pane_state should be seeded for bid2 on the focused pane.
    assert!(
        ed.selections_for(ed.state.focused_pane_id, bid2).is_some(),
        "pane_state seeded for new buffer"
    );
}

/// `close_buffer` with one other buffer redirects panes and frees the slot.
#[test]
fn p6_close_buffer_redirects_to_mru() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("alpha\n"), SelectionSet::default()));
    let bid_alpha = ed.focused_buffer_id();
    let doc_beta = Buffer::new(Text::from("beta\n"), SelectionSet::default());
    let bid_beta = ed.open_buffer(doc_beta);
    ed.switch_to_buffer_with_jump(bid_beta);
    assert_eq!(ed.focused_buffer_id(), bid_beta);
    // Close beta — should redirect focused pane back to alpha.
    ed.close_buffer(bid_beta);
    assert_eq!(
        ed.focused_buffer_id(),
        bid_alpha,
        "focused pane redirected to alpha after closing beta"
    );
    assert!(
        ed.state.buffers.try_get(bid_beta).is_none(),
        "beta slot freed from BufferStore"
    );
}

/// `close_buffer` on the last buffer replaces it with scratch (Case C).
#[test]
fn p6_close_last_buffer_becomes_scratch() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("only\n"), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    ed.close_buffer(bid);
    // Buffer id stays valid but content is now scratch.
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "same buffer id after scratch replacement"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "scratch buffer has structural newline only"
    );
}

/// `replace_buffer_in_place` reseeds selections and clears scrolls.
#[test]
fn p6_replace_buffer_in_place_reseeds() {
    let mut ed = Editor::for_testing(Buffer::new(
        Text::from("old content\n"),
        SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    // Move the cursor somewhere non-zero.
    let focused = ed.state.focused_pane_id;
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |b, _sels| {
            let head = b.len_chars().saturating_sub(2);
            SelectionSet::single(hume_editing::selection::Selection::collapsed(head))
        },
    );
    let replacement = Buffer::new(Text::from("new content\n"), SelectionSet::default());
    ed.replace_buffer_in_place(bid, replacement);
    // Selections should be reset to initial (cursor at 0).
    let sels = ed.current_selections();
    assert_eq!(
        sels.primary().head(),
        0,
        "selections reset after replace_buffer_in_place"
    );
    assert_eq!(ed.doc().text().to_string(), "new content\n");
}

/// `:bnext` / `:bprev` cycle through buffers in open-order.
#[test]
fn p6_bnext_bprev_cycle() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("a\n"), SelectionSet::default()));
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::new(Text::from("b\n"), SelectionSet::default()));
    let bid_c = ed.open_buffer(Buffer::new(Text::from("c\n"), SelectionSet::default()));
    // Still focused on a. bnext → b.
    let _ = ed.execute_typed("bn", None);
    assert_eq!(ed.focused_buffer_id(), bid_b, "bnext advances to b");
    let _ = ed.execute_typed("bn", None);
    assert_eq!(ed.focused_buffer_id(), bid_c, "bnext advances to c");
    let _ = ed.execute_typed("bn", None);
    assert_eq!(ed.focused_buffer_id(), bid_a, "bnext wraps to a");
    // bprev from a → c.
    let _ = ed.execute_typed("bp", None);
    assert_eq!(ed.focused_buffer_id(), bid_c, "bprev wraps to c");
    let _ = ed.execute_typed("bp", None);
    assert_eq!(ed.focused_buffer_id(), bid_b, "bprev to b");
}

/// `:bd` closes the current buffer.
#[test]
fn p6_bd_closes_focused_buffer() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("first\n"), SelectionSet::default()));
    let bid_first = ed.focused_buffer_id();
    let bid_second = ed.open_buffer(Buffer::new(Text::from("second\n"), SelectionSet::default()));
    ed.switch_to_buffer_with_jump(bid_second);
    let _ = ed.execute_typed("bd", None);
    assert_eq!(
        ed.focused_buffer_id(),
        bid_first,
        "bd closed second, focused pane moved to first"
    );
    assert!(
        ed.state.buffers.try_get(bid_second).is_none(),
        "second buffer freed"
    );
}

/// `:bd!` closes a dirty buffer without error.
#[test]
fn p6_bd_force_closes_dirty_buffer() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("clean\n"), SelectionSet::default()));
    let bid_clean = ed.focused_buffer_id();
    let bid_dirty = ed.open_buffer(Buffer::new(Text::from("dirty\n"), SelectionSet::default()));
    ed.switch_to_buffer_with_jump(bid_dirty);
    // Make it dirty by inserting a character.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer should be dirty after edit");
    // :bd without force should fail.
    let result = ed.execute_typed("bd", None);
    assert!(
        result.is_err(),
        ":bd on dirty buffer without force should fail"
    );
    // :bd! should succeed.
    let result = ed.execute_typed("bd!", None);
    assert!(result.is_ok(), ":bd! should close dirty buffer");
    assert_eq!(ed.focused_buffer_id(), bid_clean);
}

/// `:split`, `:vsplit`, and their aliases `:sp`/`:vsp` are M9+ stubs.
///
/// Locks the error contract so the stubs can't accidentally become no-ops
/// or panics when the feature isn't yet wired.
#[test]
fn colon_split_vsplit_are_stubs() {
    use crate::editor::error::CommandError;
    for cmd in ["split", "vsplit", "sp", "vsp"] {
        let mut ed = editor_from("-[h]>ello\n");
        let err: CommandError = ed.execute_typed(cmd, None).unwrap_err();
        assert!(
            err.message().contains("not yet implemented"),
            ":{cmd} must report not-yet-implemented, got: {:?}",
            err.message().to_owned(),
        );
        // execute_typed also sets status_msg so the user sees the error.
        assert!(
            ed.state
                .status_msg
                .as_deref()
                .unwrap_or("")
                .contains("not yet implemented"),
            ":{cmd} must set error status: {:?}",
            ed.state.status_msg,
        );
    }
}

/// `close_buffer` redirects ALL panes viewing the closed buffer to the MRU alternative.
///
/// The `:bd` tests verify the single-pane path. This test targets the multi-pane
/// redirect branch: both the focused and a non-focused pane must be redirected.
#[test]
fn p6_close_buffer_redirects_all_panes_to_mru() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("a\n"), SelectionSet::default()));
    let bid_a = ed.focused_buffer_id();
    // open_buffer seeds pane_state for the focused pane but doesn't switch the pane view.
    let bid_b = ed.open_buffer(Buffer::new(Text::from("b\n"), SelectionSet::default()));

    let pid_1 = ed.state.focused_pane_id;
    // Second pane also views A.
    let pid_2 = ed.open_pane(bid_a);

    assert_eq!(
        ed.view.panes[pid_1].buffer_id, bid_a,
        "sanity: pid_1 views A"
    );
    assert_eq!(
        ed.view.panes[pid_2].buffer_id, bid_a,
        "sanity: pid_2 views A"
    );

    // Close A; mru_excluding(A) == B (B was opened last, so it's at the MRU tail).
    ed.close_buffer(bid_a);

    assert_eq!(
        ed.view.panes[pid_1].buffer_id, bid_b,
        "focused pane redirected to B"
    );
    assert_eq!(
        ed.view.panes[pid_2].buffer_id, bid_b,
        "non-focused pane redirected to B"
    );
    assert!(
        ed.state.buffers.try_get(bid_a).is_none(),
        "closed buffer freed from store"
    );
}

/// `:e path` opens a new buffer when the file is not already open.
#[test]
#[cfg(not(windows))]
fn p6_edit_opens_new_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "file content\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(
        Text::from("scratch\n"),
        SelectionSet::default(),
    ));
    let initial_bid = ed.focused_buffer_id();
    let canonical = std::fs::canonicalize(&path).unwrap();
    let result = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(result.is_ok(), ":e should succeed for readable file");
    assert_ne!(
        ed.focused_buffer_id(),
        initial_bid,
        ":e opened a new buffer"
    );
    assert_eq!(ed.doc().text().to_string(), "file content\n");
    // Path stored correctly.
    assert_eq!(ed.doc().path(), Some(canonical.as_path()));
}

/// `:e path` deduplicates: switching to an already-open file doesn't create a new buffer.
#[test]
#[cfg(not(windows))]
fn p6_edit_deduplicates_open_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup.txt");
    std::fs::write(&path, "dedup\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(
        Text::from("scratch\n"),
        SelectionSet::default(),
    ));
    // Open the file once.
    let r1 = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(r1.is_ok());
    let bid_first_open = ed.focused_buffer_id();
    let count_after_first = ed.state.buffers.len();
    // Switch back to scratch.
    let scratch_bid = ed.state.buffers.prev(bid_first_open);
    ed.switch_to_buffer_without_jump(scratch_bid);
    // Open the same file again — should switch to existing buffer, not create new.
    let r2 = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(r2.is_ok());
    assert_eq!(
        ed.focused_buffer_id(),
        bid_first_open,
        "dedup: switched to existing buffer"
    );
    assert_eq!(
        ed.state.buffers.len(),
        count_after_first,
        "no new buffer created on dedup"
    );
}

// ── reload_buffer_in_place cursor-preservation tests ─────────────────────────

/// `reload_buffer_in_place` preserves the primary cursor line/column when the
/// reloaded content is identical.
#[test]
fn p6_reload_preserves_cursor_same_content() {
    use hume_editing::selection::Selection;

    // Five content lines: line 0..4, each "lineN\n" (6 chars).
    let content = "line0\nline1\nline2\nline3\nline4\n";
    let mut ed = Editor::for_testing(Buffer::new(Text::from(content), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;

    // Place cursor at line 2, col 3 (char offset = 6+6+3 = 15).
    let expected_head = 15usize;
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |_, _| SelectionSet::single(Selection::collapsed(expected_head)),
    );

    // Reload with identical content.
    let replacement = Buffer::new(Text::from(content), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    assert_eq!(
        ed.current_selections().primary().head(),
        expected_head,
        "cursor preserved at same position after reload with identical content",
    );
}

/// `reload_buffer_in_place` clamps the cursor to the last line when the
/// reloaded file has fewer lines than the original cursor position.
#[test]
fn p6_reload_clamps_cursor_to_last_line() {
    use hume_editing::selection::Selection;

    // Five content lines; cursor on line 4.
    let content = "line0\nline1\nline2\nline3\nline4\n";
    let mut ed = Editor::for_testing(Buffer::new(Text::from(content), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;

    // line 4 starts at char 24.
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |_, _| SelectionSet::single(Selection::collapsed(24)),
    );

    // Reload with a 1-line file.
    let replacement = Buffer::new(Text::from("short\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    // last_line=0, target_line=0, col=0 → head=0.
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "cursor clamped to line 0 after reload with fewer lines",
    );
}

/// `reload_buffer_in_place` clamps a col that exceeds the new line length to
/// the line's terminating `\n`.
#[test]
fn p6_reload_clamps_col_to_line_end() {
    use hume_editing::selection::Selection;

    let mut ed = Editor::for_testing(Buffer::new(
        Text::from("hello world\n"),
        SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;

    // Cursor at col 10 ('d' in "hello world\n"). head=10.
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |_, _| SelectionSet::single(Selection::collapsed(10)),
    );

    // Reload with a shorter line "hi\n" (h=0,i=1,\n=2).
    let replacement = Buffer::new(Text::from("hi\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    // line_end=2 (\n), target=(0+10).min(2)=2 → head=2.
    assert_eq!(
        ed.current_selections().primary().head(),
        2,
        "cursor clamped to \\n when col exceeds new line length",
    );
}

/// `reload_buffer_in_place` snaps a col that lands inside a grapheme cluster
/// back to the cluster's start.
#[test]
fn p6_reload_snaps_col_to_grapheme_boundary() {
    use hume_editing::selection::Selection;

    // "caf" + é (U+0065 U+0301, two chars) + "\n" → len_chars=6.
    // Grapheme boundaries: 0,1,2,3,5,6 — é occupies chars 3..5.
    let content = "caf\u{0065}\u{0301}\n";
    let mut ed = Editor::for_testing(Buffer::new(Text::from(content), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;

    // Place cursor mid-cluster at char 4 (the combining acute U+0301).
    // Normal motions won't do this; set directly.
    ed.state.panes.state[focused][bid].selections = SelectionSet::single(Selection::collapsed(4));

    // Reload with identical content — col=4 is mid-cluster.
    let replacement = Buffer::new(Text::from(content), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    // snap_to_grapheme_boundary(text, 0, 4) should land at 3 (start of é).
    assert_eq!(
        ed.current_selections().primary().head(),
        3,
        "cursor snapped back to grapheme cluster start",
    );
}

/// `reload_buffer_in_place` collapses multi-cursor selections to the primary.
#[test]
fn p6_reload_collapses_multi_selection_to_primary() {
    use hume_editing::selection::Selection;

    // "line0\nline1\nline2\n": line 0 starts at 0, line 1 at 6, line 2 at 12.
    let content = "line0\nline1\nline2\n";
    let mut ed = Editor::for_testing(Buffer::new(Text::from(content), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;

    // Two selections: primary at line 1 (head=6), secondary at line 2 (head=12).
    ed.state.panes.state[focused][bid].selections = SelectionSet::from_vec(
        vec![Selection::collapsed(6), Selection::collapsed(12)],
        0, // primary index
    );
    assert_eq!(
        ed.current_selections().len(),
        2,
        "sanity: two selections set"
    );

    let replacement = Buffer::new(Text::from(content), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    let sels = ed.current_selections();
    assert_eq!(
        sels.len(),
        1,
        "multi-selection collapsed to single after reload"
    );
    assert_eq!(
        sels.primary().head(),
        6,
        "primary cursor preserved at line 1 col 0",
    );
}

/// `:e!` reloads the current file even when dirty.
#[test]
#[cfg(not(windows))]
fn p6_edit_force_reloads_current_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reload.txt");
    std::fs::write(&path, "original\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(
        Text::from("scratch\n"),
        SelectionSet::default(),
    ));
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());
    // :e without force should fail.
    let r = ed.execute_typed("e", None);
    assert!(r.is_err(), ":e on dirty buffer should fail without !");
    // :e! should reload.
    let r = ed.execute_typed("e!", None);
    assert!(r.is_ok(), ":e! should reload");
    assert_eq!(
        ed.doc().text().to_string(),
        "original\n",
        "reloaded from disk"
    );
    assert!(!ed.doc().is_dirty(), "not dirty after reload");
}

// ── :e! undo-retention (RELOAD.md) ────────────────────────────────────────────
//
// `:e!` now records the reload as an ordinary edit in the existing undo tree
// instead of discarding history. `u` reverts to the pre-reload buffer (full
// prior tree intact beneath); `Ctrl-r` re-applies the reload.

/// `:e!` reload records an undoable revision: one `u` reverts to the
/// pre-reload buffer (with its prior edit intact beneath), and `Ctrl-r`
/// re-applies the reload.
#[test]
#[cfg(not(windows))]
fn p6_e_bang_undo_restores_pre_reload_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("undo.txt");
    std::fs::write(&path, "original\n").unwrap();

    let mut ed = editor_from("-[o]>riginal\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    assert_eq!(ed.doc().text().to_string(), "original\n");
    assert!(!ed.doc().is_dirty());

    // Edit the buffer (insert "X" at the cursor).
    ed.handle_key(key('i'));
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Xoriginal\n");
    assert!(ed.doc().is_dirty());

    // Change the file on disk, then `:e!` reload.
    std::fs::write(&path, "changed\n").unwrap();
    ed.execute_typed("e!", None).unwrap();
    assert_eq!(ed.doc().text().to_string(), "changed\n");
    assert!(!ed.doc().is_dirty(), "reload marks the buffer clean");

    // Single undo restores the pre-reload buffer ("Xoriginal\n") — NOT the
    // disk version "original\n" — so the prior edit's undo tree is intact
    // beneath the reload.
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "Xoriginal\n",
        "undo after :e! restores the pre-reload buffer, not the disk file",
    );
    assert!(
        ed.doc().is_dirty(),
        "undoing the reload re-dirties the buffer",
    );

    // One more undo reverts the insert back to the root (original file content).
    // The root is dirty here: the reload moved `saved_revision` to the reload
    // revision, and "original\n" no longer matches the on-disk "changed\n".
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "original\n");
    assert!(
        ed.doc().is_dirty(),
        "root content differs from disk after reload"
    );
    assert!(!ed.doc().can_undo(), "back at the root after two undos");

    // Redo re-applies the reload.
    ed.handle_key(key_ctrl('r')); // → "Xoriginal\n"
    ed.handle_key(key_ctrl('r')); // → "changed\n"
    assert_eq!(ed.doc().text().to_string(), "changed\n");
    assert!(
        !ed.doc().is_dirty(),
        "redo lands on the saved reload revision"
    );
}

/// `:e!` → `u` → new edit branches off the old tree: the new edit becomes a
/// sibling of the reload (tree-monotonicity). Redo from the branch-point goes
/// to the new edit, and the reload revision survives as a reachable sibling.
#[test]
#[cfg(not(windows))]
fn p6_e_bang_undo_then_edit_branches_off_old_tree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("branch.txt");
    std::fs::write(&path, "base\n").unwrap();

    let mut ed = editor_from("-[b]>ase\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    // E1: insert "X" at the start.
    set_cursor(&mut ed, 0);
    ed.handle_key(key('i'));
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Xbase\n");
    let r_e1 = ed.doc().revision_id();

    // R: reload from disk → new revision, child of E1.
    std::fs::write(&path, "changed\n").unwrap();
    ed.execute_typed("e!", None).unwrap();
    let r_reload = ed.doc().revision_id();
    assert_ne!(r_reload, r_e1);
    assert_eq!(ed.doc().text().to_string(), "changed\n");

    // Undo back to E1, then redo — last child is the reload.
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().revision_id(), r_e1);
    ed.handle_key(key_ctrl('r'));
    assert_eq!(ed.doc().revision_id(), r_reload);

    // Undo back to E1 and make a NEW edit (E2). It must branch as a sibling
    // of the reload, not overwrite it.
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().revision_id(), r_e1);
    set_cursor(&mut ed, 0);
    ed.handle_key(key('i'));
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    let r_e2 = ed.doc().revision_id();
    assert_ne!(r_e2, r_reload, "new edit is not the reload revision");
    assert_eq!(ed.doc().text().to_string(), "YXbase\n");

    // From E1, redo must go to the new edit (last child), not the reload.
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().revision_id(), r_e1);
    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.doc().revision_id(),
        r_e2,
        "redo goes to the new branch, not the old reload",
    );

    // The reload revision survives as a reachable sibling (tree-monotonicity).
    let bid = ed.focused_buffer_id();
    let mut sels = ed.current_selections().clone();
    ed.doc_mut().goto_revision(&mut sels, r_reload);
    assert_eq!(
        ed.doc().text().to_string(),
        "changed\n",
        "old reload branch is still reachable via goto_revision",
    );
    // Restore pane selections so the editor state is consistent post-test.
    let focused = ed.state.focused_pane_id;
    ed.state.panes.state[focused][bid].selections = sels;
}

/// The reload inverse `ChangeSet` is fine-grained, not a coarse delete-all +
/// insert-all: after a single-line reload, `undo`'s inverse re-inserts only the
/// changed line, not the whole document.
#[test]
#[cfg(not(windows))]
fn p6_e_bang_inverse_is_fine_grained() {
    use hume_editing::changeset::Operation;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fine.txt");
    std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();

    let mut ed = editor_from("-[a]>lpha\nbeta\ngamma\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    // Single-line change on disk.
    std::fs::write(&path, "alpha\nBETA\ngamma\n").unwrap();
    ed.execute_typed("e!", None).unwrap();
    assert_eq!(ed.doc().text().to_string(), "alpha\nBETA\ngamma\n");

    // Pull the inverse CS straight from the buffer's undo path. The coarse
    // (buffer-swap) reload would have produced an inverse of
    // `Delete(whole new) | Insert(whole old)`; the line-diff path produces a
    // small `Insert("beta\n")` instead.
    let (_, inv_cs) = ed.doc_mut().undo().expect("undo returns the inverse CS");
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

/// Move the focused pane's primary cursor to `head` for the focused buffer.
fn set_cursor(ed: &mut Editor, head: usize) {
    use hume_editing::selection::Selection;
    let focused = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |_, _| SelectionSet::single(Selection::collapsed(head)),
    );
}
