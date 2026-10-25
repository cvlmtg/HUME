//! `increment`/`decrement` and their sequence variants: dispatch by name
//! (count, refusals, undo) and the keys bound to them.

use super::*;

#[test]
fn increment_steps_by_the_count() {
    let mut ed = editor_from("-[x]> 9\n");

    call_counted(&mut ed, "increment", Some(3));

    assert_eq!(state(&ed), "x -[12]>\n");
}

#[test]
fn no_count_steps_by_one() {
    let mut ed = editor_from("-[x]> 9\n");

    call_counted(&mut ed, "increment", None);

    assert_eq!(state(&ed), "x -[10]>\n");
}

#[test]
fn decrement_steps_down_by_the_count() {
    let mut ed = editor_from("-[1]>0\n");

    call_counted(&mut ed, "decrement", Some(4));

    assert_eq!(state(&ed), "-[6]>\n");
}

#[test]
fn increment_sequence_steps_the_nth_selection_n_times_the_count() {
    let mut ed = editor_from("-{1}>\n-[1]>\n-[1]>\n");

    call_counted(&mut ed, "increment-sequence", Some(2));

    assert_eq!(state(&ed), "-{3}>\n-[5]>\n-[7]>\n");
}

#[test]
fn decrement_sequence_steps_the_nth_selection_down_n_times_the_count() {
    let mut ed = editor_from("-{9}>\n-[9]>\n");

    call_counted(&mut ed, "decrement-sequence", None);

    assert_eq!(state(&ed), "-{8}>\n-[7]>\n");
}

#[test]
fn an_increment_is_one_undo_step() {
    let mut ed = editor_from("-{1}>\n-[2]>\n");
    call_counted(&mut ed, "increment", None);
    assert_eq!(state(&ed), "-{2}>\n-[3]>\n", "premise: the increment edits");

    ed.handle_key(key('u'));

    assert_eq!(ed.doc().text().to_string(), "1\n2\n");
}

#[test]
fn no_target_refuses_and_records_no_revision() {
    let mut ed = editor_from("-[x]>\n");
    let bid = ed.focused_buffer_id();

    call_counted(&mut ed, "increment", None);

    assert_eq!(state(&ed), "-[x]>\n");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("No number, date or time to change")
    );
    assert!(!ed.state.buffers.get(bid).can_undo());
}

#[test]
fn a_result_out_of_range_refuses_and_records_no_revision() {
    let mut ed = editor_from("-[9]>999-12-31\n");
    let bid = ed.focused_buffer_id();

    call_counted(&mut ed, "increment", None);

    assert_eq!(state(&ed), "-[9]>999-12-31\n");
    assert_eq!(ed.state.status_msg.as_deref(), Some("Result out of range"));
    assert!(!ed.state.buffers.get(bid).can_undo());
}

#[test]
fn a_step_that_leaves_the_text_as_it_was_records_no_revision() {
    let mut ed = editor_from("-[0]>x0\n");
    let bid = ed.focused_buffer_id();

    ed.feed_key(key_ctrl('s'));

    assert_eq!(state(&ed), "-[0x0]>\n");
    assert!(!ed.state.buffers.get(bid).can_undo());
}

#[test]
fn pressing_ctrl_a_again_keeps_stepping_the_same_date_field() {
    let mut ed = editor_from("2026-1-[0]>-18\n");

    ed.feed_keys([key_ctrl('a'), key_ctrl('a')]);

    assert_eq!(state(&ed), "2026--[12]>-18\n");
}

#[test]
fn a_read_only_buffer_refuses_the_increment() {
    let mut ed = editor_from("-[a]>\n");
    ed.report(Severity::Warning, "1 test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());
    let before = ed.doc().text().to_string();

    call_counted(&mut ed, "increment", None);

    assert_eq!(ed.doc().text().to_string(), before);
    assert_eq!(status(&ed), read_only_refusal(&ed));
}

#[test]
fn ctrl_a_increments_and_ctrl_s_decrements_with_and_without_kitty() {
    for kitty in [false, true] {
        let mut ed = editor_from("-[x]> 5\n");
        ed.kitty_enabled = kitty;

        ed.feed_key(key_ctrl('a'));
        assert_eq!(state(&ed), "x -[6]>\n", "kitty: {kitty}");

        ed.feed_keys([key_ctrl('s'), key_ctrl('s')]);
        assert_eq!(state(&ed), "x -[4]>\n", "kitty: {kitty}");
    }
}

#[test]
fn g_a_and_g_s_step_in_sequence() {
    let mut ed = editor_from("-{1}>\n-[1]>\n-[1]>\n");

    ed.feed_keys([key('G'), key('A')]);
    assert_eq!(state(&ed), "-{2}>\n-[3]>\n-[4]>\n");

    ed.feed_keys([key('G'), key('S')]);
    assert_eq!(state(&ed), "-{1}>\n-[1]>\n-[1]>\n");
}

#[test]
fn a_count_before_the_key_sets_the_step() {
    let mut ed = editor_from("-[x]> 5\n");

    ed.feed_keys([key('1'), key('0'), key_ctrl('a')]);

    assert_eq!(state(&ed), "x -[15]>\n");
}

#[test]
fn dot_repeats_with_the_recorded_count_or_its_own() {
    let mut ed = editor_from("-[x]> 5\n");
    ed.feed_keys([key('3'), key_ctrl('a')]);
    assert_eq!(state(&ed), "x -[8]>\n", "premise: the increment edits");

    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "x -[11]>\n");

    ed.feed_keys([key('2'), key('.')]);
    assert_eq!(state(&ed), "x -[13]>\n");
}

#[test]
fn ctrl_a_in_extend_mode_increments_and_leaves_extend_mode() {
    let mut ed = editor_from("-[x]> 5\n");
    ed.feed_key(key('e'));
    assert_eq!(ed.state.mode(), Mode::Extend, "premise: in Extend mode");

    ed.feed_key(key_ctrl('a'));

    assert_eq!(state(&ed), "x -[6]>\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
}
