// A buffer's text changes only together with every position stored against
// it: the selections of each pane that has shown the buffer, snapshots taken
// by an open prompt, and the rest. Each test changes the text while one such
// store is out of view, then reads the store back.

use super::*;
use crate::editor::buffer::Buffer;

/// A pane that showed a buffer and moved to another one keeps its selections
/// for the buffer. A reload while it is away carries them to the new text,
/// the same as the focused pane's, so coming back never lands past the end.
#[test]
fn reload_carries_the_selections_of_a_pane_showing_another_buffer() {
    let mut ed = editor_from("line0\nline1\nline2\nline3\n-[l]>ine4\n");
    let bid = ed.focused_buffer_id();

    ed.execute_typed("split", None).unwrap();
    let away = ed.state.focus.id();
    select(&mut ed, &[(24, 24)], 0);
    ed.open_buffer(Buffer::at_start(BufferText::from("other\n")));
    ed.execute_typed("bn", None).unwrap();
    assert_ne!(
        ed.view.panes[away].buffer_id, bid,
        "setup: the pane shows another buffer"
    );

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "setup: back on the reloading pane"
    );
    let replacement = Buffer::at_start(BufferText::from("short\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.state.focus.id(), away);
    ed.execute_typed("bp", None).unwrap();
    assert_eq!(ed.focused_buffer_id(), bid);
    assert_eq!(
        state(&ed),
        "-[s]>hort\n",
        "the pane's cursor lands where the reload replaced its line"
    );
}

/// A prompt's snapshot names the buffer it was taken from. Closing that
/// buffer drops the snapshot, so the prompt keeps working on the buffer the
/// pane moved to instead of indexing the pane state removed with the buffer.
#[test]
fn closing_the_buffer_a_search_prompt_snapshotted_leaves_the_prompt_usable() {
    let mut ed = editor_from("-[h]>ello\n");
    let closed = ed.focused_buffer_id();
    ed.open_buffer(Buffer::at_start(BufferText::from("other\n")));
    ed.handle_key(key('/'));
    assert_eq!(ed.state.mode(), Mode::Search, "setup: search is open");

    ed.close_buffer(closed);
    ed.handle_key(key('o'));
    ed.handle_key(key_esc());

    assert_ne!(ed.focused_buffer_id(), closed);
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.current_view().check(),
        Ok(()),
        "the pane's selections fit the buffer it moved to"
    );
}

/// Sift snapshots the selections it will restore on cancel. An edit that
/// lands while the prompt is open (a workspace edit from a language server)
/// carries the snapshot to the new text, so cancelling restores the same
/// characters rather than stale offsets.
#[test]
fn sift_cancel_after_an_out_of_band_edit_restores_the_carried_selection() {
    let mut ed = editor_from("abc -[defgh]>\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.state.mode(), Mode::Sift, "setup: sift is open");

    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    let delete_prefix = lsp_types::TextEdit {
        range: lsp_types::Range::new(
            lsp_types::Position::new(0, 0),
            lsp_types::Position::new(0, 4),
        ),
        new_text: String::new(),
    };
    crate::editor::lsp::edits::apply_text_edits(
        &mut ed.state,
        &ed.view.panes,
        pid,
        bid,
        vec![delete_prefix],
        None,
        hume_rope::position_encoding::PositionEncoding::Utf8,
    )
    .expect("the edit applies");
    assert_eq!(
        ed.doc().text().to_string(),
        "defgh\n",
        "setup: the edit landed"
    );

    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(state(&ed), "-[defgh]>\n");
}

/// A smart paste of the text a selection already holds lands after it, and
/// undo returns to the selection the paste was made from, whole, including
/// a last cluster that is a letter with a combining accent.
#[test]
fn undoing_a_repeated_smart_paste_restores_the_selection_it_was_made_from() {
    let mut ed = editor_from("-[ae\u{301}]>\n");
    ed.state.kill_ring.push(vec!["ae\u{301}".to_string()]);
    ed.feed_key(key('p'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ae\u{301}ae\u{301}\n",
        "setup: the repeat appended"
    );
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), "-[ae\u{301}]>\n");
}

/// An undo walk that nets to no change gives the text back the version its
/// selections were saved for. A pane showing the buffer from elsewhere keeps
/// selections that pair with it.
#[test]
fn a_net_identity_undo_walk_keeps_another_panes_selections_paired() {
    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let root = ed.doc().text().version();
    ed.execute_typed("split", None).unwrap();
    let away = ed.state.focus.id();
    select(&mut ed, &[(3, 3)], 0);
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_ne!(ed.state.focus.id(), away, "setup: back on the editing pane");

    for k in [
        key('i'),
        key('x'),
        key_esc(),
        key('a'),
        key_backspace(),
        key_esc(),
    ] {
        ed.feed_key(k);
    }
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\n",
        "setup: the edits cancel out"
    );
    ed.feed_key(key('2'));
    ed.feed_key(key('u'));

    let text = ed.doc().text();
    assert_eq!(text.to_string(), "hello\n");
    assert_eq!(text.version(), root);
    let away_head = ed.state.panes.state[away][bid].view(text).primary().head();
    assert_eq!(away_head.offset().index(), 3);
}

// ── Tracked positions ───────────────────────────────────────────────────────

/// A position a script asked the editor to remember follows the text.
#[test]
fn a_tracked_position_follows_an_insertion_above_it() {
    let mut ed = editor_from("abc\nd-[e]>f\n");
    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    let head = ed.current_view().primary().head();
    let text = ed.doc().text().clone();
    let token = ed.state.panes.tracked.track(bid, &text, head);
    assert_eq!(
        ed.state.panes.tracked.position(token, &ed.state.buffers),
        Some((bid, co(5))),
        "setup: the position is the cursor's"
    );

    let insert_line = lsp_types::TextEdit {
        range: lsp_types::Range::new(
            lsp_types::Position::new(0, 0),
            lsp_types::Position::new(0, 0),
        ),
        new_text: "X\n".to_string(),
    };
    crate::editor::lsp::edits::apply_text_edits(
        &mut ed.state,
        &ed.view.panes,
        pid,
        bid,
        vec![insert_line],
        None,
        hume_rope::position_encoding::PositionEncoding::Utf8,
    )
    .expect("the edit applies");
    assert_eq!(ed.doc().text().to_string(), "X\nabc\ndef\n");

    assert_eq!(
        ed.state.panes.tracked.position(token, &ed.state.buffers),
        Some((bid, co(7)))
    );
}

/// A reload carries it through the line diff, like every other stored position.
#[test]
fn a_tracked_position_follows_a_reload() {
    let mut ed = editor_from("one\ntwo\n-[t]>hree\n");
    let bid = ed.focused_buffer_id();
    let head = ed.current_view().primary().head();
    let text = ed.doc().text().clone();
    let token = ed.state.panes.tracked.track(bid, &text, head);

    let replacement = Buffer::at_start(BufferText::from("zero\none\ntwo\nthree\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::disk(replacement),
    );

    assert_eq!(
        ed.state.panes.tracked.position(token, &ed.state.buffers),
        Some((bid, co(13)))
    );
}

/// A buffer whose text is replaced with no change to carry has no position
/// left to answer for.
#[test]
fn a_tracked_position_is_dropped_when_its_buffer_is_replaced() {
    let mut ed = editor_from("-[a]>\n");
    let fp = FocusedPane::current(&ed.state);
    let view = ed.open_read_only_view(fp, "[tracked]", "one\ntwo\n", None);
    let head = ed.state.buffers.get(view).text().snap(co(0));
    let token = ed
        .state
        .panes
        .tracked
        .track(view, ed.state.buffers.get(view).text(), head);
    assert!(
        ed.state
            .panes
            .tracked
            .position(token, &ed.state.buffers)
            .is_some(),
        "setup: tracked in the view buffer"
    );

    ed.open_read_only_view(
        FocusedPane::current(&ed.state),
        "[tracked]",
        "other\n",
        None,
    );

    assert_eq!(
        ed.state.panes.tracked.position(token, &ed.state.buffers),
        None
    );
}

/// Closing the buffer releases its positions.
#[test]
fn a_tracked_position_is_dropped_when_its_buffer_closes() {
    let mut ed = editor_from("-[a]>\n");
    let other = ed.open_buffer(Buffer::at_start(BufferText::from("other\n")));
    let head = ed.state.buffers.get(other).text().snap(co(0));
    let token = ed
        .state
        .panes
        .tracked
        .track(other, ed.state.buffers.get(other).text(), head);

    ed.close_buffer(other);

    assert_eq!(
        ed.state.panes.tracked.position(token, &ed.state.buffers),
        None
    );
    assert_eq!(ed.state.panes.tracked.len(), 0, "the entry itself goes");
}

/// A token nobody minted, or already released, answers absent.
#[test]
fn an_unknown_or_released_token_answers_absent() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    let head = ed.current_view().primary().head();
    let text = ed.doc().text().clone();
    let token = ed.state.panes.tracked.track(bid, &text, head);
    ed.state.panes.tracked.untrack(token);
    assert_eq!(
        ed.state.panes.tracked.position(token, &ed.state.buffers),
        None
    );
    assert_eq!(
        ed.state.panes.tracked.position(
            hume_scripting::host::HostToken::from_raw(u64::MAX),
            &ed.state.buffers
        ),
        None
    );
}

// ── Decorations ─────────────────────────────────────────────────────────────

/// A decoration set right after an edit, against the edited text, stays where
/// it was set: the edit already moved the decorations stored before it.
#[test]
fn a_decoration_set_after_an_edit_is_not_moved_by_that_edit() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let hint = |pos| hume_decorations::InlayHintEntry {
        pos: co(pos),
        text: "!".to_string(),
        before: false,
    };
    ed.state
        .config
        .decorations
        .set_inlay_hints("test".to_string(), bid, vec![hint(2)]);

    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    assert_eq!(
        ed.doc().text().to_string(),
        "Xabc\n",
        "setup: the edit landed"
    );
    ed.state
        .config
        .decorations
        .set_inlay_hints("test".to_string(), bid, vec![hint(3)]);
    ed.settle();

    let positions: Vec<_> = ed
        .state
        .config
        .decorations
        .inlay_hints_for_buffer(bid)
        .map(|h| h.pos)
        .collect();
    assert_eq!(positions, vec![co(3)]);
}
