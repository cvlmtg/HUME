use super::*;
use pretty_assertions::assert_eq;

// ── BufferStore + buffer choke-points ─────────────────────────────────────────

use crate::editor::commands::open_pane_in_layout;
use crate::editor::doc_ops;
use hume_editing::text::BufferText;
use hume_scripting::host::CommandHost;

/// `open_buffer` allocates a new BufferId and tracks MRU, but seeds no
/// pane's `pane_state` yet. No pane shows the buffer until something
/// switches to it, and that switch is what seeds it (lazily, on first
/// visit; see `lifecycle::open_buffer`'s doc).
#[test]
fn p6_open_buffer_defers_pane_state_seeding() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("hello\n")));
    let initial_bid = ed.focused_buffer_id();
    let doc2 = Buffer::at_start(BufferText::from("world\n"));
    let bid2 = ed.open_buffer(doc2);
    assert_ne!(bid2, initial_bid);
    assert!(
        ed.selections_for(ed.state.focus.id(), bid2).is_none(),
        "pane_state not yet seeded for a buffer no pane shows"
    );

    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.state), bid2);
    assert!(
        ed.selections_for(ed.state.focus.id(), bid2).is_some(),
        "pane_state seeded once the focused pane switches to it"
    );
}

/// `close_buffer` with one other buffer redirects panes and frees the slot.
#[test]
fn p6_close_buffer_redirects_to_mru() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("alpha\n")));
    let bid_alpha = ed.focused_buffer_id();
    let doc_beta = Buffer::at_start(BufferText::from("beta\n"));
    let bid_beta = ed.open_buffer(doc_beta);
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.state), bid_beta);
    assert_eq!(ed.focused_buffer_id(), bid_beta);
    // Close beta: should redirect focused pane back to alpha.
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

/// `close_buffer` on the last buffer frees its slot and opens a fresh
/// scratch buffer under a new `BufferId` (Case C), not a content swap
/// under the closed id, which a captured `bid` could then alias.
#[test]
fn p6_close_last_buffer_becomes_scratch() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("only\n")));
    let bid = ed.focused_buffer_id();
    ed.close_buffer(bid);
    assert!(
        ed.state.buffers.try_get(bid).is_none(),
        "the closed buffer's own slot must be freed, not reused"
    );
    assert_ne!(
        ed.focused_buffer_id(),
        bid,
        "the fresh scratch buffer must have its own, different BufferId"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "scratch buffer has structural newline only"
    );
}

/// A bid captured before `:bd` on the last buffer must read as dead
/// afterward. A same-slot in-place replace would defeat every
/// `LiveBid`-checked builtin here, since the closed id would still `try_get`
/// successfully against unrelated scratch content.
///
/// If `close_buffer`'s last-buffer branch reused `id` in place,
/// `get-buffer-option` would succeed against the scratch buffer
/// instead of raising "invalid buffer id".
#[test]
fn p6_bid_captured_before_last_buffer_close_reads_dead_afterward() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("only\n")));
    let tmp = safe_tempdir();
    // `bid` is the typed command's own injected leading param, the only
    // buffer, so it's the one `close-buffer!` below closes. Reusing that
    // same captured Scheme value afterward is exactly "a bid captured
    // before the close, read after".
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "close-and-probe" "" (lambda (bid)
             (close-buffer! bid)
             (get-buffer-option bid "tab-width")))"#,
    );
    type_cmd(&mut ed, ":close-and-probe");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("invalid buffer id"),
        "a get-buffer-option on the just-closed bid must raise, not read the scratch \
         replacement's tab-width: {log:?}"
    );
}

/// Closing the last buffer (the scratch-replacement branch of
/// `buffer::lifecycle::close_buffer`) reseeds selections back to the start,
/// not just the scratch content `p6_close_last_buffer_becomes_scratch`
/// already pins.
#[test]
fn p6_close_last_buffer_reseeds_selections() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("old content\n")));
    let bid = ed.focused_buffer_id();
    // Move the cursor somewhere non-zero.
    let focused = ed.state.focus.id();
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |st| {
            let cursor =
                test_fixtures::testing::cursor(st.text(), st.text().len_chars().saturating_sub(2));
            st.with_selections(vec![cursor], 0)
        },
    );
    ed.close_buffer(bid);
    // Selections should be reset to initial (cursor at 0).
    let sels = ed.current_view();
    assert_eq!(
        sels.primary().head().offset(),
        co(0),
        "selections reset after closing the last buffer"
    );
    assert_eq!(ed.doc().text().to_string(), "\n");
}

/// `:bnext` / `:bprev` cycle through buffers in open-order.
#[test]
fn p6_bnext_bprev_cycle() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("a\n")));
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::at_start(BufferText::from("b\n")));
    let bid_c = ed.open_buffer(Buffer::at_start(BufferText::from("c\n")));
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

/// `goto-next-buffer`/`goto-prev-buffer` cycle through buffers in open-order:
/// the mappable, key-bindable siblings of `:bnext`/`:bprev`.
#[test]
fn goto_next_prev_buffer_cycle() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("a\n")));
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::at_start(BufferText::from("b\n")));
    let bid_c = ed.open_buffer(Buffer::at_start(BufferText::from("c\n")));
    // Still focused on a. goto-next-buffer → b.
    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("goto-next-buffer", pane, Some(1), false, None)
        .expect("goto-next-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_b, "advances to b");
    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("goto-next-buffer", pane, Some(1), false, None)
        .expect("goto-next-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_c, "advances to c");
    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("goto-next-buffer", pane, Some(1), false, None)
        .expect("goto-next-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_a, "wraps to a");
    // goto-prev-buffer from a → c.
    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("goto-prev-buffer", pane, Some(1), false, None)
        .expect("goto-prev-buffer must not error");
    assert_eq!(ed.focused_buffer_id(), bid_c, "wraps to c");
    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("goto-prev-buffer", pane, Some(1), false, None)
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
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("first\n")));
    let bid_first = ed.focused_buffer_id();
    let bid_second = ed.open_buffer(Buffer::at_start(BufferText::from("second\n")));
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.state), bid_second);
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
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("clean\n")));
    let bid_clean = ed.focused_buffer_id();
    let bid_dirty = ed.open_buffer(Buffer::at_start(BufferText::from("dirty\n")));
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.state), bid_dirty);
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
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("a\n")));
    let bid_a = ed.focused_buffer_id();
    // open_buffer seeds pane_state for the focused pane but doesn't switch the pane view.
    let bid_b = ed.open_buffer(Buffer::at_start(BufferText::from("b\n")));

    let pid_1 = ed.state.focus.id();
    // Second pane also views A.
    let pid_2 = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_1,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

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
    // Five content lines: line 0..4, each "lineN\n" (6 chars).
    let content = "line0\nline1\nline2\nline3\nline4\n";
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from(content)));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focus.id();

    // Place cursor at line 2, col 3 (char offset = 6+6+3 = 15).
    let expected_head = co(15);
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |st| {
            let cursor = test_fixtures::testing::cursor(st.text(), expected_head.index());
            st.with_selections(vec![cursor], 0)
        },
    );

    // Reload with identical content.
    let replacement = Buffer::at_start(BufferText::from(content));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    assert_eq!(
        ed.current_view().primary().head().offset(),
        expected_head,
        "cursor preserved at same position after reload with identical content",
    );
}

/// `reload_buffer_in_place` clamps the cursor to the last line when the
/// reloaded file has fewer lines than the original cursor position.
#[test]
fn p6_reload_clamps_cursor_to_last_line() {
    // Five content lines; cursor on line 4.
    let content = "line0\nline1\nline2\nline3\nline4\n";
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from(content)));
    let bid = ed.focused_buffer_id();
    let focused = ed.state.focus.id();

    // line 4 starts at char 24.
    doc_ops::apply_doc_motion(
        &ed.state.buffers,
        &mut ed.state.panes.state,
        focused,
        bid,
        |st| {
            let cursor = test_fixtures::testing::cursor(st.text(), 24);
            st.with_selections(vec![cursor], 0)
        },
    );

    // Reload with a 1-line file.
    let replacement = Buffer::at_start(BufferText::from("short\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    // last_line=0, target_line=0, col=0 → head=0.
    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(0),
        "cursor clamped to line 0 after reload with fewer lines",
    );
}

/// A reload is an edit: the cursor follows its text through the line diff,
/// so a line added above leaves it on the same characters.
#[test]
fn p6_reload_cursor_follows_its_line_when_a_line_is_added_above() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("abc\ndef\n")));
    set_cursor(&mut ed, 5);

    let replacement = Buffer::at_start(BufferText::from("new\nabc\ndef\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    assert_eq!(state(&ed), "new\nabc\nd-[e]>f\n");
}

/// A cursor on a line the reload rewrote lands where that line starts.
#[test]
fn p6_reload_cursor_on_a_rewritten_line_lands_at_its_start() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("abc\nhello world\n")));
    set_cursor(&mut ed, 14);

    let replacement = Buffer::at_start(BufferText::from("abc\nhi\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    assert_eq!(state(&ed), "abc\n-[h]>i\n");
}

/// Every selection follows its text, not only the primary.
#[test]
fn p6_reload_keeps_every_selection() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("line0\nline1\nline2\n")));
    select(&mut ed, &[(6, 6), (12, 12)], 0);

    let replacement = Buffer::at_start(BufferText::from("top\nline0\nline1\nline2\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    assert_eq!(state(&ed), "top\nline0\n-{l}>ine1\n-[l]>ine2\n");
}

// ── :e! undo-retention ──────────────────────────────────────────────────────
//
// `:e!` records the reload as an ordinary edit in the existing undo tree
// instead of discarding history. `u` reverts to the pre-reload buffer (full
// prior tree intact beneath); `Ctrl-r` re-applies the reload.

// ── find_by_path — Windows `\\?\` verbatim-prefix normalization ───────────────
//
// Stored buffer paths are always `fs::canonicalize` output: `\\?\C:\…` on
// Windows. Most lookups also canonicalize and match as-is, but the `:b <name>`
// fallback for a deleted backing file (`typed_buffer.rs`) uses
// `std::path::absolute`, which never carries the verbatim prefix. Without
// normalizing both sides, that lookup dedup-misses an already-open buffer.

#[cfg(windows)]
#[test]
fn find_by_path_matches_verbatim_prefixed_stored_path_against_a_plain_query() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("hello\n")));
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
    // to a plain `\\server\share\…` form; strip_unc_prefix
    // leaves it untouched, so these two remain distinct buffers.
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("hello\n")));
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

/// `(buffer-live? bid)`: the non-raising idiom for a timer/debounce/async
/// continuation to check its captured `bid` before acting on it. `#t` for
/// an open buffer, `#f` for one that closed since. Never raises either
/// way, unlike a `LiveBid`-checked builtin.
#[test]
fn buffer_live_reflects_open_and_closed_state() {
    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("a\n")));
    // A second buffer so the probe's own close below frees its slot
    // outright rather than hitting the last-buffer scratch-replacement
    // branch.
    ed.open_buffer(Buffer::at_start(BufferText::from("b\n")));

    let tmp = safe_tempdir();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (log! 'trace (string-append "before: " (to-string (buffer-live? bid))))
             (close-buffer! bid)
             (log! 'trace (string-append "after: " (to-string (buffer-live? bid))))))"#,
    );
    type_cmd(&mut ed, ":probe");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("before: #t"),
        "an open buffer must read as live: {log:?}"
    );
    assert!(
        log.contains("after: #f"),
        "the just-closed buffer must read as dead, not raise: {log:?}"
    );
}
