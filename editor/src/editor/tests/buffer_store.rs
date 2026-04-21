use super::*;
use pretty_assertions::assert_eq;

// ── Phase 6 — BufferStore + buffer choke-points ───────────────────────────────

use crate::core::text::Text;
use crate::core::selection::SelectionSet;

/// `open_buffer` allocates a new BufferId, seeds pane_state, and tracks MRU.
#[test]
fn p6_open_buffer_seeds_pane_state() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("hello\n"), SelectionSet::default()));
    let initial_bid = ed.focused_buffer_id();
    let doc2 = Buffer::new(Text::from("world\n"), SelectionSet::default());
    let bid2 = ed.open_buffer(doc2);
    assert_ne!(bid2, initial_bid);
    // pane_state should be seeded for bid2 on the focused pane.
    assert!(ed.selections_for(ed.focused_pane_id, bid2).is_some(), "pane_state seeded for new buffer");
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
    assert_eq!(ed.focused_buffer_id(), bid_alpha, "focused pane redirected to alpha after closing beta");
    assert!(ed.buffers.try_get(bid_beta).is_none(), "beta slot freed from BufferStore");
}

/// `close_buffer` on the last buffer replaces it with scratch (Case C).
#[test]
fn p6_close_last_buffer_becomes_scratch() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("only\n"), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    ed.close_buffer(bid);
    // Buffer id stays valid but content is now scratch.
    assert_eq!(ed.focused_buffer_id(), bid, "same buffer id after scratch replacement");
    assert_eq!(ed.doc().text().to_string(), "\n", "scratch buffer has structural newline only");
}

/// `replace_buffer_in_place` reseeds selections and clears scrolls.
#[test]
fn p6_replace_buffer_in_place_reseeds() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("old content\n"), SelectionSet::default()));
    let bid = ed.focused_buffer_id();
    // Move the cursor somewhere non-zero.
    ed.apply_motion(|b, _sels| {
        let head = b.len_chars().saturating_sub(2);
        SelectionSet::single(crate::core::selection::Selection::collapsed(head))
    });
    let replacement = Buffer::new(Text::from("new content\n"), SelectionSet::default());
    ed.replace_buffer_in_place(bid, replacement);
    // Selections should be reset to initial (cursor at 0).
    let sels = ed.current_selections();
    assert_eq!(sels.primary().head, 0, "selections reset after replace_buffer_in_place");
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
    assert_eq!(ed.focused_buffer_id(), bid_first, "bd closed second, focused pane moved to first");
    assert!(ed.buffers.try_get(bid_second).is_none(), "second buffer freed");
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
    assert!(result.is_err(), ":bd on dirty buffer without force should fail");
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
    use crate::core::error::CommandError;
    for cmd in ["split", "vsplit", "sp", "vsp"] {
        let mut ed = editor_from("-[h]>ello\n");
        let err: CommandError = ed.execute_typed(cmd, None).unwrap_err();
        assert!(
            err.0.contains("not yet implemented"),
            ":{cmd} must report not-yet-implemented, got: {:?}", err.0,
        );
        // execute_typed also sets status_msg so the user sees the error.
        assert!(
            ed.status_msg.as_deref().unwrap_or("").contains("not yet implemented"),
            ":{cmd} must set error status: {:?}", ed.status_msg,
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

    let pid_1 = ed.focused_pane_id;
    // Second pane also views A.
    let pid_2 = ed.open_pane(bid_a);

    assert_eq!(ed.engine_view.panes[pid_1].buffer_id, bid_a, "sanity: pid_1 views A");
    assert_eq!(ed.engine_view.panes[pid_2].buffer_id, bid_a, "sanity: pid_2 views A");

    // Close A; mru_excluding(A) == B (B was opened last, so it's at the MRU tail).
    ed.close_buffer(bid_a);

    assert_eq!(ed.engine_view.panes[pid_1].buffer_id, bid_b, "focused pane redirected to B");
    assert_eq!(ed.engine_view.panes[pid_2].buffer_id, bid_b, "non-focused pane redirected to B");
    assert!(ed.buffers.try_get(bid_a).is_none(), "closed buffer freed from store");
}

/// `:e path` opens a new buffer when the file is not already open.
#[test]
#[cfg(not(windows))]
fn p6_edit_opens_new_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "file content\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    let initial_bid = ed.focused_buffer_id();
    let canonical = std::fs::canonicalize(&path).unwrap();
    let result = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(result.is_ok(), ":e should succeed for readable file");
    assert_ne!(ed.focused_buffer_id(), initial_bid, ":e opened a new buffer");
    assert_eq!(ed.doc().text().to_string(), "file content\n");
    // Path stored correctly.
    assert_eq!(ed.doc().path.as_deref().map(|p| p.as_path()), Some(canonical.as_path()));
}

/// `:e path` deduplicates: switching to an already-open file doesn't create a new buffer.
#[test]
#[cfg(not(windows))]
fn p6_edit_deduplicates_open_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup.txt");
    std::fs::write(&path, "dedup\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    // Open the file once.
    let r1 = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(r1.is_ok());
    let bid_first_open = ed.focused_buffer_id();
    let count_after_first = ed.buffers.len();
    // Switch back to scratch.
    let scratch_bid = ed.buffers.prev(bid_first_open);
    ed.switch_to_buffer_without_jump(scratch_bid);
    // Open the same file again — should switch to existing buffer, not create new.
    let r2 = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(r2.is_ok());
    assert_eq!(ed.focused_buffer_id(), bid_first_open, "dedup: switched to existing buffer");
    assert_eq!(ed.buffers.len(), count_after_first, "no new buffer created on dedup");
}

/// `:e!` reloads the current file even when dirty.
#[test]
#[cfg(not(windows))]
fn p6_edit_force_reloads_current_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reload.txt");
    std::fs::write(&path, "original\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
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
    assert_eq!(ed.doc().text().to_string(), "original\n", "reloaded from disk");
    assert!(!ed.doc().is_dirty(), "not dirty after reload");
}

