use super::*;
use super::super::buffer_store::set_cursor;

/// `:e path` opens a new buffer when the file is not already open.
#[test]
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

/// `:e!` reloads the current file even when dirty.
#[test]
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

/// `:e!` reload records an undoable revision: one `u` reverts to the
/// pre-reload buffer (with its prior edit intact beneath), and `Ctrl-r`
/// re-applies the reload.
#[test]
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
