//! A confirm against other overlays and pending key sequences.

use super::*;
use pretty_assertions::assert_eq;

// ── Confirm can't open over another overlay or a pending key sequence ────────────

/// A confirm must never open while a picker is on screen: `confirm_permit`
/// requires the stack to be at `Base`, so a picker on top blocks it outright
/// rather than letting it push above the picker and eat every key it needs.
/// The blocked change only warns, and the
/// deferred prompt still arrives on the next buffer-enter after the picker
/// closes, same deferral rule as a mode-blocked or non-focused change.
#[test]
fn confirm_does_not_open_over_a_live_picker_but_defers_to_next_buffer_enter() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let session = crate::editor::input_stack::picker::PickerSession::new(
        steel::rvals::SteelVal::BoolV(false),
        hume_scripting::host::PickerOpts::default(),
        Vec::new(),
    );
    crate::editor::input_stack::picker::open_picker(&mut ed.state, &ed.view, session, "picker!")
        .expect("no `.` is in progress");

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "must warn instead of prompting"
    );
    assert!(ed.state.input.confirm().is_none());
    assert!(
        ed.state.input.picker().is_some(),
        "the picker must stay open"
    );

    let picker_ref = ed
        .state
        .input
        .ref_of::<crate::editor::input_stack::PickerLayer>()
        .expect("sanity: the picker must still be open");
    ed.state.truncate_layers(&ed.view, picker_ref);
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);
    assert!(
        ed.state.input.confirm().is_some(),
        "the deferred prompt must arrive once the picker is gone"
    );
}

/// `enter_buffer_disk_check` retiring a confirm that no longer targets the
/// buffer being entered must not take collateral: a `Prompt` opened above
/// the stale confirm since (nothing about `push_mode_layer` truncates an
/// overlay sitting on `Base`, so this is a reachable stack) has nothing to
/// do with the question the confirm was answering.
///
/// The retirement must remove the confirm alone. `truncate_layers` at the
/// confirm's own ref would also remove every layer above it, closing the
/// prompt and firing its callback with `#f`.
#[test]
fn stale_confirm_retirement_does_not_take_a_prompt_above_it_with_it() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "sanity: confirm open on A"
    );

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.prompt(
        pane,
        "Name: ".to_string(),
        String::new(),
        steel::rvals::SteelVal::Void,
    )
    .unwrap();
    assert!(ed.state.minibuf().is_some(), "sanity: prompt open above it");

    let path_b = tmp.parent().unwrap().join("b.txt");
    std::fs::write(&path_b, "b\n").unwrap();
    ed.execute_typed("e", Some(path_b.to_str().unwrap()))
        .unwrap();
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: distinct buffers");

    ed.enter_buffer_disk_check(bid_b);

    assert!(
        ed.state
            .input
            .ref_of::<crate::editor::input_stack::ConfirmLayer>()
            .is_none(),
        "the stale confirm must be gone"
    );
    assert!(
        ed.state.minibuf().is_some(),
        "the unrelated prompt above it must survive untouched"
    );
    // `pending_work` isn't empty in general (`:e`'s own `OnBufferOpen`
    // announcement is queued there too), so check specifically for the
    // absence of a `Call`, the variant a fired Steel callback would be.
    assert!(
        !ed.state
            .config
            .pending_work
            .iter()
            .any(|w| matches!(w, crate::editor::event::PendingWork::Call { .. })),
        "the surviving prompt's callback must not have fired"
    );
}

/// A disk-change confirm must still be able to open while a `Scrollable`
/// popup (hover, the `gn`/`gp` diagnostic overlay) is showing. A popup owns
/// no keys beyond Ctrl-u/d and dies on the very next one anyway, so it must
/// not block the one prompt that actually needs the keyboard.
///
/// For that reason `confirm_permit` cannot gate on
/// `is::<BaseLayer>(top())`. A `Popup` above `Base` would fail that check,
/// and only the `Severity::Warning` fallback would fire.
#[test]
fn confirm_still_opens_over_an_open_scrollable_popup() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let popup = crate::editor::input_stack::PopupLayer::new(
        "hello".to_string(),
        None,
        hume_ui::popup::PopupLayout::Cursor,
        &mut ed.view.theme,
        &ed.view.registry,
    );
    ed.state.push_layer(&ed.view, popup);

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);

    assert!(
        ed.state.input.confirm().is_some(),
        "the confirm must open even with a popup on top"
    );
    assert!(
        ed.state.input.popup().is_none(),
        "landing above it must clear the popup, matching every other opener"
    );
}

/// The flip side of the test above: a picker opening while a confirm is
/// already up lands above it, same as over a menu. `picker!` is
/// key-triggered, so dispatch order already proves the stack is where the
/// key path left it. The confirm survives beneath, swallowed by the
/// picker's own full-modal input policy, and resumes driving input the
/// moment the picker retires.
#[test]
fn picker_opens_over_a_live_confirm_and_the_confirm_resumes_once_it_closes() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(ed.state.input.confirm().is_some(), "sanity: confirm open");

    let session = crate::editor::input_stack::picker::PickerSession::new(
        steel::rvals::SteelVal::BoolV(false),
        hume_scripting::host::PickerOpts::default(),
        Vec::new(),
    );
    crate::editor::input_stack::picker::open_picker(&mut ed.state, &ed.view, session, "picker!")
        .expect("no `.` is in progress");

    assert!(ed.state.input.picker().is_some(), "the picker opened");
    assert!(
        ed.state.input.confirm().is_some(),
        "the confirm survives beneath it"
    );
    assert!(
        ed.state
            .input
            .is::<crate::editor::input_stack::PickerLayer>(ed.state.input.top()),
        "the picker, not the confirm, drives input while it's open"
    );

    let picker_ref = ed
        .state
        .input
        .ref_of::<crate::editor::input_stack::PickerLayer>()
        .expect("sanity: the picker must still be open");
    ed.state.truncate_layers(&ed.view, picker_ref);

    assert!(ed.state.input.picker().is_none(), "the picker closed");
    assert!(
        ed.state
            .input
            .is::<crate::editor::input_stack::ConfirmLayer>(ed.state.input.top()),
        "the confirm must drive input again once the picker is gone"
    );
}

/// A picker sitting above a confirm must survive, untouched, when the
/// confirm below it is retired for an unrelated reason (here,
/// `enter_buffer_disk_check` finding it targets a buffer that's no longer
/// focused): `enter_buffer_disk_check` calls
/// `EditorState::retire_stale_confirm`, which excises rather than
/// truncates, specifically because the picker has nothing to do with the
/// question the confirm was answering and must not be taken as collateral.
///
/// Truncating at the confirm's ref would also remove the picker, and the
/// picker's `tear_down` (its "fires exactly once" contract for an incidental
/// removal) would fire `on_select` with `#f`.
#[test]
fn picker_above_a_retired_confirm_survives_untouched() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: back on A");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: confirm open on A"
    );

    let session = crate::editor::input_stack::picker::PickerSession::new(
        steel::rvals::SteelVal::StringV("cb".into()),
        hume_scripting::host::PickerOpts::default(),
        Vec::new(),
    );
    crate::editor::input_stack::picker::open_picker(&mut ed.state, &ed.view, session, "picker!")
        .expect("no `.` is in progress");
    assert!(
        ed.state
            .input
            .is::<crate::editor::input_stack::PickerLayer>(ed.state.input.top()),
        "setup: picker above the confirm"
    );

    ed.enter_buffer_disk_check(bid_b);

    assert!(
        ed.state.input.confirm().is_none(),
        "confirm must be retired"
    );
    assert!(
        ed.state.input.picker().is_some(),
        "the picker above it must survive untouched"
    );
    assert!(
        pending_calls(&ed).is_empty(),
        "the surviving picker's callback must not have fired"
    );
}

/// `EditorState::confirm()` (the statusline's seam) must answer `None`
/// once a `Prompt` lands above an open confirm: `push_mode_layer` only
/// truncates the outgoing *mode* layer, never an overlay sitting on
/// `Base`, so a Steel-initiated `prompt!` (a timer here, standing in for a
/// hook or async callback) buries the confirm without closing it. The
/// confirm is still on the stack. `InputStack::confirm()`, the query
/// `buffer::disk`/`buffer::lifecycle` use, must still find it, but it no
/// longer owns the keyboard, so the statusline must not paint its row
/// over the minibuffer that does.
///
/// `EditorState::confirm()` therefore cannot just delegate to
/// `InputStack::confirm()`, which finds a confirm anywhere on the stack.
/// Doing so would render the disk-change prompt while the user typed into
/// an invisible minibuffer.
#[test]
fn statusline_seam_hides_a_confirm_buried_under_a_prompt() {
    let script_tmp = tempfile::tempdir().unwrap();
    let (mut ed, file_path) = editor_with_file("-[h]>ello\n", "hello\n");
    run(
        &mut ed,
        script_tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda (pane)
             (after! 0 (lambda () (prompt! pane "x: " (lambda (s) (void)))))))"#,
    );
    type_cmd(&mut ed, ":arm");

    rewrite_externally(&file_path, "hello, externally changed!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.confirm().is_some(),
        "sanity: the confirm is open and answerable"
    );

    ed.settle(); // drains the due timer, which pushes Prompt above the confirm

    assert!(
        ed.state.input.confirm().is_some(),
        "sanity: the confirm is still on the stack, merely buried"
    );
    assert!(
        ed.state.confirm().is_none(),
        "a buried confirm must not claim the statusline row"
    );
    assert!(
        ed.state.minibuf().is_some(),
        "the prompt must own the keyboard/row instead"
    );
}

/// A confirm must never open while the pending sequence is non-empty: a live
/// multi-key sequence (e.g. `d` waiting for its motion) already owns the
/// very next keystroke.
///
/// `confirm_permit` checks the pending sequence for this.
#[test]
fn confirm_does_not_open_mid_pending_key_sequence() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.state.pending.push(key('g'), bid);

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert!(ed.state.input.confirm().is_none());
}

/// A sequence started in another buffer can't take the next key here, so it
/// doesn't hold the confirm back.
#[test]
fn confirm_opens_despite_a_sequence_left_in_another_buffer() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    open_other_buffer(&mut ed, "other\n");
    ed.feed_key(key('g'));
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), bid);
    rewrite_externally(&tmp, "hello, externally changed!\n");

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert!(ed.state.input.confirm().is_some());
}
