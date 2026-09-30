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
    ed.reload_buffer_in_place(FocusedPane::current(&ed.state), replacement);

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.state.focus.id(), away);
    ed.execute_typed("bp", None).unwrap();
    assert_eq!(ed.focused_buffer_id(), bid);
    assert_eq!(
        state(&ed),
        "-[s]>hort\n",
        "the pane's cursor on line 4 clamps to the reloaded text's last line"
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
    ed.reload_buffer_in_place(FocusedPane::current(&ed.state), replacement);

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
            hume_scripting::host::WidgetToken::from_raw(u64::MAX),
            &ed.state.buffers
        ),
        None
    );
}
