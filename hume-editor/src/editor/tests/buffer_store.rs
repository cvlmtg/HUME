use super::*;
use pretty_assertions::assert_eq;

// ── BufferStore + buffer choke-points ─────────────────────────────────────────

use crate::editor::commands::open_pane;
use crate::editor::doc_ops;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_scripting::host::CommandHost;

/// `open_buffer` allocates a new BufferId, seeds pane_state, and tracks MRU.
#[test]
fn p6_open_buffer_seeds_pane_state() {
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("hello\n"),
        SelectionSet::default(),
    ));
    let initial_bid = ed.focused_buffer_id();
    let doc2 = Buffer::new(BufferText::from("world\n"), SelectionSet::default());
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
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("alpha\n"),
        SelectionSet::default(),
    ));
    let bid_alpha = ed.focused_buffer_id();
    let doc_beta = Buffer::new(BufferText::from("beta\n"), SelectionSet::default());
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
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("only\n"),
        SelectionSet::default(),
    ));
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
        BufferText::from("old content\n"),
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
    let replacement = Buffer::new(BufferText::from("new content\n"), SelectionSet::default());
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
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("a\n"),
        SelectionSet::default(),
    ));
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("b\n"),
        SelectionSet::default(),
    ));
    let bid_c = ed.open_buffer(Buffer::new(
        BufferText::from("c\n"),
        SelectionSet::default(),
    ));
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

/// `goto-next-buffer`/`goto-prev-buffer` cycle through buffers in open-order —
/// the mappable, key-bindable siblings of `:bnext`/`:bprev`.
#[test]
fn goto_next_prev_buffer_cycle() {
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("a\n"),
        SelectionSet::default(),
    ));
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("b\n"),
        SelectionSet::default(),
    ));
    let bid_c = ed.open_buffer(Buffer::new(
        BufferText::from("c\n"),
        SelectionSet::default(),
    ));
    // Still focused on a. goto-next-buffer → b.
    live_host!(ed)
        .run_command_sync("goto-next-buffer", Some(1), false, None)
        .expect("goto-next-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_b, "advances to b");
    live_host!(ed)
        .run_command_sync("goto-next-buffer", Some(1), false, None)
        .expect("goto-next-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_c, "advances to c");
    live_host!(ed)
        .run_command_sync("goto-next-buffer", Some(1), false, None)
        .expect("goto-next-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_a, "wraps to a");
    // goto-prev-buffer from a → c.
    live_host!(ed)
        .run_command_sync("goto-prev-buffer", Some(1), false, None)
        .expect("goto-prev-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_c, "wraps to c");
    live_host!(ed)
        .run_command_sync("goto-prev-buffer", Some(1), false, None)
        .expect("goto-prev-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_b, "back to b");
}

/// Both directions are registered as mappable, key-bindable commands that
/// record a jump-list entry on switch (mirrors
/// `goto_alternate_buffer_is_registered_as_jump`).
#[test]
fn goto_next_prev_buffer_registered_as_jump() {
    let reg = super::super::registry::CommandRegistry::with_defaults();
    for name in ["goto-next-buffer", "goto-prev-buffer"] {
        let cmd = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("{name} must be registered"));
        assert!(cmd.meta().is_jump, "{name} must have jump:true");
    }
}

/// `:bd` closes the current buffer.
#[test]
fn p6_bd_closes_focused_buffer() {
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("first\n"),
        SelectionSet::default(),
    ));
    let bid_first = ed.focused_buffer_id();
    let bid_second = ed.open_buffer(Buffer::new(
        BufferText::from("second\n"),
        SelectionSet::default(),
    ));
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
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("clean\n"),
        SelectionSet::default(),
    ));
    let bid_clean = ed.focused_buffer_id();
    let bid_dirty = ed.open_buffer(Buffer::new(
        BufferText::from("dirty\n"),
        SelectionSet::default(),
    ));
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

/// `close_buffer` redirects ALL panes viewing the closed buffer to the MRU alternative.
///
/// The `:bd` tests verify the single-pane path. This test targets the multi-pane
/// redirect branch: both the focused and a non-focused pane must be redirected.
#[test]
fn p6_close_buffer_redirects_all_panes_to_mru() {
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("a\n"),
        SelectionSet::default(),
    ));
    let bid_a = ed.focused_buffer_id();
    // open_buffer seeds pane_state for the focused pane but doesn't switch the pane view.
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("b\n"),
        SelectionSet::default(),
    ));

    let pid_1 = ed.state.focused_pane_id;
    // Second pane also views A.
    let pid_2 = open_pane(&mut ed.state, &mut ed.view, bid_a);

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

// ── reload_buffer_in_place cursor-preservation tests ─────────────────────────

/// `reload_buffer_in_place` preserves the primary cursor line/column when the
/// reloaded content is identical.
#[test]
fn p6_reload_preserves_cursor_same_content() {
    use hume_editing::selection::Selection;

    // Five content lines: line 0..4, each "lineN\n" (6 chars).
    let content = "line0\nline1\nline2\nline3\nline4\n";
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from(content),
        SelectionSet::default(),
    ));
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
    let replacement = Buffer::new(BufferText::from(content), SelectionSet::default());
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
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from(content),
        SelectionSet::default(),
    ));
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
    let replacement = Buffer::new(BufferText::from("short\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    // last_line=0, target_line=0, col=0 → head=0.
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "cursor clamped to line 0 after reload with fewer lines",
    );
}

/// `reload_buffer_in_place` clamps a char col that exceeds the new line
/// length to the line's last content character (the vim/helix
/// stick-to-content convention `place_char_column` uses) — never onto the
/// line's terminating `\n`.
#[test]
fn p6_reload_clamps_char_col_to_line_end() {
    use hume_editing::selection::Selection;

    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("hello world\n"),
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
    let replacement = Buffer::new(BufferText::from("hi\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    // last content char='i' (char 1); overshooting col 10 clamps there, not
    // onto the '\n' at char 2.
    assert_eq!(
        ed.current_selections().primary().head(),
        1,
        "cursor clamped to the last content char when col exceeds new line length",
    );
}

/// `reload_buffer_in_place` snaps a char col that lands inside a grapheme
/// cluster back to the cluster's start.
#[test]
fn p6_reload_snaps_char_col_to_grapheme_boundary() {
    use hume_editing::selection::Selection;

    // "caf" + é (U+0065 U+0301, two chars) + "\n" → len_chars=6.
    // Grapheme boundaries: 0,1,2,3,5,6 — é occupies chars 3..5.
    let content = "caf\u{0065}\u{0301}\n";
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from(content),
        SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;

    // Place cursor mid-cluster at char 4 (the combining acute U+0301).
    // Normal motions won't do this; set directly.
    ed.state.panes.state[focused][bid].selections = SelectionSet::single(Selection::collapsed(4));

    // Reload with identical content — col=4 is mid-cluster.
    let replacement = Buffer::new(BufferText::from(content), SelectionSet::default());
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
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from(content),
        SelectionSet::default(),
    ));
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

    let replacement = Buffer::new(BufferText::from(content), SelectionSet::default());
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

// ── :e! undo-retention ──────────────────────────────────────────────────────
//
// `:e!` records the reload as an ordinary edit in the existing undo tree
// instead of discarding history. `u` reverts to the pre-reload buffer (full
// prior tree intact beneath); `Ctrl-r` re-applies the reload.

// ── find_by_path — Windows `\\?\` verbatim-prefix normalization ───────────────
//
// Stored buffer paths are always `fs::canonicalize` output — `\\?\C:\…` on
// Windows. Most lookups also canonicalize and match as-is, but the `:b <name>`
// fallback for a deleted backing file (`typed_buffer.rs`) uses
// `std::path::absolute`, which never carries the verbatim prefix. Without
// normalizing both sides, that lookup dedup-misses an already-open buffer.

#[cfg(windows)]
#[test]
fn find_by_path_matches_verbatim_prefixed_stored_path_against_a_plain_query() {
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("hello\n"),
        SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(std::path::PathBuf::from(r"\\?\C:\tmp\foo.txt")));

    let found = ed
        .state
        .buffers
        .find_by_path(std::path::Path::new(r"C:\tmp\foo.txt"));
    assert_eq!(
        found,
        Some(bid),
        "a plain-form query must dedup-match a \\\\?\\-stored path"
    );
}

#[cfg(windows)]
#[test]
fn find_by_path_leaves_verbatim_unc_paths_alone() {
    // `\\?\UNC\…` (verbatim network share) must NOT be treated as equivalent
    // to a plain `\\server\share\…` form — strip_unc_prefix deliberately
    // leaves it untouched, so these two remain distinct buffers.
    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("hello\n"),
        SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(std::path::PathBuf::from(
            r"\\?\UNC\server\share\foo.txt",
        )));

    let found = ed
        .state
        .buffers
        .find_by_path(std::path::Path::new(r"\\server\share\foo.txt"));
    assert_eq!(
        found, None,
        "a verbatim UNC path must not match its plain-UNC form"
    );
}

/// Move the focused pane's primary cursor to `head` for the focused buffer.
#[cfg(unix)]
pub(super) fn set_cursor(ed: &mut Editor, head: usize) {
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
