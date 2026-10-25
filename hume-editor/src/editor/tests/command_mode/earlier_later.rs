//! `:earlier` and `:later`.

use super::*;
use pretty_assertions::assert_eq;

// ── :earlier / :later ─────────────────────────────────────────────────────────

#[test]
fn earlier_with_no_arg_steps_back_one_revision() {
    let mut ed = editor_from("-[h]>ello\n");
    type_text(&mut ed, "X");
    let after_one = state(&ed);
    type_text(&mut ed, "Y");
    assert_ne!(state(&ed), after_one);
    submit(&mut ed, "earlier");
    assert_eq!(
        state(&ed),
        after_one,
        ":earlier with no arg must undo exactly one insert session"
    );
}

#[test]
fn earlier_count_then_later_roundtrip() {
    let mut ed = editor_from("-[h]>ello\n");
    let initial = state(&ed);
    type_text(&mut ed, "X");
    type_text(&mut ed, "Y");
    let after_two_text = ed.doc().text().to_string();
    submit(&mut ed, "earlier 2");
    assert_eq!(state(&ed), initial, ":earlier 2 must undo both sessions");
    submit(&mut ed, "later 2");
    assert_eq!(
        ed.doc().text().to_string(),
        after_two_text,
        ":later 2 must redo both sessions (selections come from the recorded redo path, not the live insert-teardown cursor)"
    );
}

#[test]
fn earlier_past_root_clamps_and_reports() {
    let mut ed = editor_from("-[h]>ello\n");
    let initial = state(&ed);
    type_text(&mut ed, "X");
    submit(&mut ed, "earlier 99");
    assert_eq!(state(&ed), initial, ":earlier past root must clamp at root");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change")
    );
}

#[test]
fn later_past_tip_clamps_and_reports() {
    let mut ed = editor_from("-[h]>ello\n");
    type_text(&mut ed, "X");
    let after_one_text = ed.doc().text().to_string();
    submit(&mut ed, "earlier");
    submit(&mut ed, "later 99");
    assert_eq!(
        ed.doc().text().to_string(),
        after_one_text,
        ":later past tip must clamp at tip"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at newest change")
    );
}

#[test]
fn earlier_zero_duration_is_noop_and_hour_walks_to_root() {
    let mut ed = editor_from("-[h]>ello\n");
    let initial = state(&ed);
    type_text(&mut ed, "X");
    let after_one = state(&ed);
    submit(&mut ed, "earlier 0s");
    assert_eq!(state(&ed), after_one, ":earlier 0s must take no steps");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        None,
        ":earlier 0s is satisfied: no report"
    );
    submit(&mut ed, "earlier 1h");
    assert_eq!(
        state(&ed),
        initial,
        ":earlier 1h must walk every revision back to the root"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change"),
        ":earlier past the root by age must clamp with a message"
    );
}

#[test]
fn earlier_bad_spec_is_error() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("earlier abc", None)
        .expect_err("abc is not a count or duration");
    assert_eq!(err.message(), "invalid time-travel spec: abc");
    assert_eq!(state(&ed), before, "a spec error must not touch the buffer");
}

#[test]
fn earlier_bang_is_rejected() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("earlier!", None)
        .expect_err(":earlier! must be rejected");
    assert_eq!(err.message(), "`:earlier` takes no `!`");
    assert_eq!(state(&ed), before);
}

/// `:earlier`/`:later` must refuse a read-only buffer exactly like `u`/
/// `Ctrl-r` do (same message, same side effects), since both walk
/// through the same `undo_steps`/`redo_steps` (`read_only_view.rs`'s
/// `read_only_buffer_blocks_undo_and_redo` is this test's key-driven
/// counterpart).
#[test]
fn earlier_and_later_agree_with_undo_and_redo_on_a_read_only_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    type_text(&mut ed, "X");
    let after_edit = state(&ed);

    ed.doc_mut().read_only = true;

    submit(&mut ed, "earlier");
    assert_eq!(
        state(&ed),
        after_edit,
        ":earlier must not undo on a read-only buffer"
    );
    assert_eq!(
        status(&ed),
        read_only_refusal(&ed),
        ":earlier must report the same refusal as u"
    );
    ed.state.status_msg = None;

    submit(&mut ed, "later");
    assert_eq!(
        state(&ed),
        after_edit,
        ":later must not redo on a read-only buffer"
    );
    assert_eq!(
        status(&ed),
        read_only_refusal(&ed),
        ":later must report the same refusal as Ctrl-r"
    );
}

#[test]
fn later_zero_duration_walks_to_tip_and_reports() {
    let mut ed = editor_from("-[h]>ello\n");
    type_text(&mut ed, "X");
    type_text(&mut ed, "Y");
    let after_two_text = ed.doc().text().to_string();
    submit(&mut ed, "earlier 1h");
    submit(&mut ed, "later 0s");
    assert_eq!(
        ed.doc().text().to_string(),
        after_two_text,
        ":later 0s must walk every revision forward to the tip"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at newest change"),
        ":later past the tip by age must clamp with a message"
    );
}

#[test]
fn later_old_age_is_noop_and_silent() {
    let mut ed = editor_from("-[h]>ello\n");
    type_text(&mut ed, "X");
    type_text(&mut ed, "Y");
    submit(&mut ed, "earlier");
    let mid = state(&ed);
    ed.state.status_msg = None;
    submit(&mut ed, "later 1h");
    assert_eq!(
        state(&ed),
        mid,
        ":later 1h from a mid-chain revision is already satisfied: no steps"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        None,
        "a satisfied :later must not report"
    );
}

/// `:later <age>` at the tip (no prior `:earlier`) must stay silent: there
/// is nothing newer to redo onto regardless of how the tip's own age
/// compares to the requested one. The age comparison itself needs a tip old
/// enough to exceed the requested age, which this test can't force without a
/// clock seam. This test covers the dispatch path. The age case is tested at
/// the `hume-editing` unit level (`redo_steps_newer_than_at_the_tip_is_satisfied_
/// regardless_of_the_tip_s_own_age`), where backdating a revision's timestamp
/// is possible.
#[test]
fn later_at_the_tip_with_no_prior_earlier_is_silent() {
    let mut ed = editor_from("-[h]>ello\n");
    type_text(&mut ed, "X");
    let before = state(&ed);
    submit(&mut ed, "later 1h");
    assert_eq!(
        state(&ed),
        before,
        ":later at the tip must not move: nothing exists to redo onto"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        None,
        "a satisfied :later must not report"
    );
}

#[test]
fn later_bad_spec_is_error() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("later abc", None)
        .expect_err("abc is not a count or duration");
    assert_eq!(err.message(), "invalid time-travel spec: abc");
    assert_eq!(state(&ed), before, "a spec error must not touch the buffer");
}

#[test]
fn later_bang_is_rejected() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("later!", None)
        .expect_err(":later! must be rejected");
    assert_eq!(err.message(), "`:later` takes no `!`");
    assert_eq!(state(&ed), before);
}
