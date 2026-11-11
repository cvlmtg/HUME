//! `undo-selection-change` (`m u`) and `redo-selection-change` (`m U`).
//!
//! Every key goes through `feed_event`, which settles after it as the run
//! loop does.

use super::*;
use crate::editor::focus::switch_pane_to_buffer;
use hume_scripting::host::EditHost;
use pretty_assertions::assert_eq;

fn undo_selection(ed: &mut Editor) {
    press(ed, "mu");
}

fn redo_selection(ed: &mut Editor) {
    press(ed, "mU");
}

#[test]
fn selection_commands_insert_extending_updates_and_motions_record_nothing() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd eee fff\n");
    press(&mut ed, "x");
    let first_line = state(&ed);
    press(&mut ed, "x");
    ed.feed_event(key_ctrl('x'));
    let two_lines = state(&ed);
    press(&mut ed, "jw");
    press(&mut ed, "ew");
    press(&mut ed, "w");
    let extended = state(&ed);
    press(&mut ed, "e");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), two_lines);
    undo_selection(&mut ed);
    assert_eq!(state(&ed), first_line);
    undo_selection(&mut ed);
    assert_eq!(state(&ed), first_line);
    redo_selection(&mut ed);
    assert_eq!(state(&ed), two_lines);
    redo_selection(&mut ed);
    assert_eq!(state(&ed), extended);
}

#[test]
fn undo_first_returns_to_the_last_entry_after_a_stray_move() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");
    ed.feed_event(key_ctrl('x'));
    ed.feed_event(key_ctrl('x'));
    let selected = state(&ed);
    press(&mut ed, "j");
    assert_ne!(state(&ed), selected);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn redo_returns_to_the_undone_entry() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");
    press(&mut ed, "x");
    let first = state(&ed);
    press(&mut ed, "x");
    let second = state(&ed);

    undo_selection(&mut ed);
    assert_eq!(state(&ed), first);
    redo_selection(&mut ed);

    assert_eq!(state(&ed), second);
}

#[test]
fn reducing_a_multi_selection_keeps_it_in_the_history() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");
    press(&mut ed, "C");
    let copied = state(&ed);
    press(&mut ed, ",");
    assert_ne!(state(&ed), copied);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), copied);
}

#[test]
fn collapsing_keeps_the_selection_it_collapsed() {
    let mut ed = editor_from("-[a]>bc\nxyz\nijk\n");
    press(&mut ed, "x");
    let selected = state(&ed);
    press(&mut ed, ";jj");

    undo_selection(&mut ed);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_count_steps_back_that_many_entries() {
    let mut ed = editor_from("-[a]>\nb\nc\nd\n");
    press(&mut ed, "C");
    let first = state(&ed);
    press(&mut ed, "CC");

    press(&mut ed, "2mu");

    assert_eq!(state(&ed), first);
}

#[test]
fn undo_with_no_history_changes_nothing() {
    let mut ed = editor_from("ab-[c]>d\n");

    undo_selection(&mut ed);
    redo_selection(&mut ed);

    assert_eq!(state(&ed), "ab-[c]>d\n");
}

#[test]
fn a_new_entry_after_undo_drops_the_entries_ahead() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");
    press(&mut ed, "x");
    let first = state(&ed);
    press(&mut ed, "x");
    undo_selection(&mut ed);
    press(&mut ed, "C");
    let copied = state(&ed);

    redo_selection(&mut ed);
    assert_eq!(state(&ed), copied);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), first);
}

#[test]
fn sift_keeps_the_selections_before_and_after() {
    let mut ed = editor_from("-[a]>b ab\nab\n");
    press(&mut ed, "%");
    let all = state(&ed);
    press(&mut ed, "sb");
    ed.feed_event(key_enter());
    let sifted = state(&ed);
    assert_ne!(sifted, all);
    press(&mut ed, ",");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), sifted);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), all);
}

#[test]
fn find_in_extend_mode_records_and_in_move_mode_does_not() {
    let mut ed = editor_from("-[a]>bz\nxyz\nijk\n");
    press(&mut ed, "efz");
    let found = state(&ed);
    press(&mut ed, "e");
    press(&mut ed, "jfk");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), found);
}

#[test]
fn find_is_navigation() {
    let mut ed = editor_from("-[a]>bz\nxyz\nijk\n");
    press(&mut ed, "C");
    let copied = state(&ed);
    press(&mut ed, "fz");
    assert_eq!(state(&ed), "ab-[z]>\nxy-{z}>\nijk\n");
    press(&mut ed, "j");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), copied);
}

#[test]
fn a_cancelled_search_records_nothing() {
    let mut ed = editor_from("-[f]>oo bar\nfoo bar\n");
    press(&mut ed, "C");
    let copied = state(&ed);
    press(&mut ed, ",");
    let kept = state(&ed);
    press(&mut ed, "/bar");
    ed.feed_event(key_esc());
    assert_eq!(state(&ed), kept);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), copied);
}

#[test]
fn search_jumps_are_navigation() {
    let mut ed = editor_from("-[a]>bc\nfoo bar foo\n");
    press(&mut ed, "x");
    let selected = state(&ed);
    press(&mut ed, "/foo");
    ed.feed_event(key_enter());
    press(&mut ed, "n");
    assert_eq!(state(&ed), "abc\nfoo bar -[foo]>\n");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_search_confirmed_in_extend_mode_continues_the_entry() {
    let mut ed = editor_from("-[a]>bc\nfoo bar foo\n");
    press(&mut ed, "x");
    let line = state(&ed);
    press(&mut ed, "e/foo");
    ed.feed_event(key_enter());
    let extended = state(&ed);
    assert_ne!(extended, line);
    press(&mut ed, "ej");

    undo_selection(&mut ed);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), extended);
}

#[test]
fn a_deleted_selection_comes_back_after_undo() {
    let mut ed = editor_from("-[o]>ne\ntwo\n\nthree\n");
    press(&mut ed, "mip");
    let paragraph = state(&ed);
    press(&mut ed, "du;");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), paragraph);
}

#[test]
fn an_edit_that_leaves_cursors_records_nothing() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "d");
    let deleted = state(&ed);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), deleted);
}

#[test]
fn leaving_insert_mode_replaces_the_entry_it_was_entered_with() {
    let mut ed = editor_from("-[f]>oo bar\nxyz\n");
    press(&mut ed, "miw");
    press(&mut ed, "cfoobar");
    ed.feed_event(key_esc());
    let typed = state(&ed);
    assert_eq!(typed, "-[foobar]> bar\nxyz\n");
    press(&mut ed, "j");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), typed);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), typed);
}

#[test]
fn a_selection_saved_in_insert_mode_survives_leaving_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-l" "select-line")"#,
    );
    press(&mut ed, "xiQ");
    ed.feed_event(key_ctrl('l'));
    let selected_in_insert = state(&ed);
    assert_eq!(selected_in_insert, "-[Qabc\n]>xyz\n");
    ed.feed_event(key_esc());
    assert_eq!(state(&ed), "-[Q]>abc\nxyz\n");
    press(&mut ed, "j");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), selected_in_insert);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), "Q-[abc\n]>xyz\n");
}

#[test]
fn a_selection_left_before_typing_at_a_cursor_is_not_offered() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "xjiQQ");
    ed.feed_event(key_esc());
    let typed = state(&ed);
    assert_eq!(typed, "abc\nxy-[QQ]>z\n");
    press(&mut ed, "k");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), typed);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), typed);
}

#[test]
fn leaving_insert_mode_without_typing_records_nothing() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "x");
    let line = state(&ed);
    press(&mut ed, "ji");
    ed.feed_event(key_esc());

    undo_selection(&mut ed);

    assert_eq!(state(&ed), line);
}

#[test]
fn a_paste_offers_what_it_pasted_then_what_it_pasted_from() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "x");
    let line = state(&ed);
    press(&mut ed, "yp");
    let pasted = state(&ed);
    press(&mut ed, "gg");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), pasted);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), line.replacen("xyz", "abc\nxyz", 1));
}

#[test]
fn a_selection_left_before_pasting_at_a_cursor_is_not_offered() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "xyjp");
    let pasted = state(&ed);
    press(&mut ed, "gg");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), pasted);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), pasted);
}

#[test]
fn replacing_the_selected_text_replaces_its_entry() {
    let mut ed = editor_from("-[a]>bc xyz\n");
    press(&mut ed, "miw");
    press(&mut ed, "rz");
    let replaced = state(&ed);
    assert_eq!(replaced, "-[zzz]> xyz\n");
    press(&mut ed, "j");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), replaced);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), replaced);
}

#[test]
fn joining_lines_offers_the_spaces_it_selects() {
    let mut ed = editor_from("-[a]>bc\nxyz\nend\n");
    press(&mut ed, "%J");
    let joined = state(&ed);
    press(&mut ed, "gg");
    assert_ne!(state(&ed), joined);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), joined);
}

#[test]
fn an_edit_that_lands_on_a_cursor_saves_nothing() {
    let mut ed = editor_from("-[f]>oo bar\n");
    press(&mut ed, "miw");
    press(&mut ed, "mw(");
    press(&mut ed, "w");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), "(-[foo]>) bar\n");
}

#[test]
fn typing_offers_the_selection_insert_mode_was_entered_with() {
    let mut ed = editor_from("-[f]>oo bar\n");
    press(&mut ed, "miwiQ");
    ed.feed_event(key_esc());
    press(&mut ed, "l");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), "Q-[foo]> bar\n");
}

#[test]
fn undoing_typing_restores_the_selection_insert_mode_was_entered_with() {
    for entry in ["i", "a"] {
        let mut ed = editor_from("-[f]>oo bar\n");
        press(&mut ed, "miw");
        let word = state(&ed);
        press(&mut ed, entry);
        press(&mut ed, "Q");
        ed.feed_event(key_esc());

        press(&mut ed, "u");

        assert_eq!(state(&ed), word, "{entry}");
    }
}

#[test]
fn in_insert_mode_selection_undo_stops_before_the_undo_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc\nx-[y]>z\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-u" "undo-selection-change")"#,
    );
    press(&mut ed, "xggiQQ");
    let typing = state(&ed);

    ed.feed_event(key_ctrl('u'));

    assert_eq!(state(&ed), typing);
    assert_eq!(
        status(&ed),
        "cannot move through undo history while an edit session is open"
    );
}

#[test]
fn an_edit_in_another_pane_carries_this_panes_history() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "x");
    let selected = state(&ed);
    press(&mut ed, "j");
    let pane_a = ed.view.focused_pane();
    vsplit_focused(&mut ed);
    press(&mut ed, "gg");
    press(&mut ed, "iQQ");
    ed.feed_event(key_esc());
    ed.switch_focused_pane(pane_a);
    ed.settle();

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected.replacen("-[abc", "QQ-[abc", 1));
}

#[test]
fn each_buffer_in_a_pane_keeps_its_own_history() {
    let mut ed = editor_from("-[a]>bc\n");
    press(&mut ed, "x");
    let selected = state(&ed);
    press(&mut ed, "h");
    let pane = ed.view.focused_pane();
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_scratch(BufferText::from("xyz\n"));
    switch_pane_to_buffer(&mut ed.state, &mut ed.view, pane, bid_b);
    ed.settle();

    undo_selection(&mut ed);
    assert_eq!(state(&ed), "-[x]>yz\n");

    switch_pane_to_buffer(&mut ed.state, &mut ed.view, pane, bid_a);
    ed.settle();
    undo_selection(&mut ed);
    assert_eq!(state(&ed), selected);
}

#[test]
fn a_split_starts_with_the_source_panes_history() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "x");
    let selected = state(&ed);
    press(&mut ed, "j");
    let moved = state(&ed);
    vsplit_focused(&mut ed);
    ed.settle();
    assert_eq!(state(&ed), moved);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn extending_in_a_split_continues_the_entry_it_copied() {
    let mut ed = editor_from("-[a]>bc\nxyz\nijk\n");
    press(&mut ed, "x");
    let line = state(&ed);
    press(&mut ed, "C");
    vsplit_focused(&mut ed);
    ed.settle();
    ed.feed_event(key_ctrl('x'));

    undo_selection(&mut ed);

    assert_eq!(state(&ed), line);
}

const TWO_COPIES: &str = r#"(define-command! "two-copies" "" (lambda (pane)
     (call! "copy-selection-on-next-line" pane)
     (call! "copy-selection-on-next-line" pane)))"#;

#[test]
fn a_script_command_records_one_entry() {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\n");
    attach_steel(&mut ed, TWO_COPIES);
    press(&mut ed, "x");
    let selected = state(&ed);
    call(&mut ed, "two-copies");
    ed.settle();

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn two_script_commands_record_two_entries() {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\nmno\n");
    attach_steel(&mut ed, TWO_COPIES);
    let mut reference = editor_from("-[a]>bc\ndef\nghi\njkl\nmno\n");
    press(&mut reference, "xCC");
    let two_copies = state(&reference);
    press(&mut ed, "x");
    let selected = state(&ed);
    for _ in 0..2 {
        call(&mut ed, "two-copies");
        ed.settle();
    }

    undo_selection(&mut ed);
    assert_eq!(state(&ed), two_copies);
    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn extending_then_copying_in_a_script_keeps_the_selection_it_started_from() {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "grow-and-copy" "" (lambda (pane)
             (call! "move-right" pane 1 #t)
             (call! "copy-selection-on-next-line" pane)))"#,
    );
    press(&mut ed, "x");
    let selected = state(&ed);
    call(&mut ed, "grow-and-copy");
    ed.settle();

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_failed_script_command_records_one_entry() {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "copies-then-fails" "" (lambda (pane)
             (call! "copy-selection-on-next-line" pane)
             (call! "copy-selection-on-next-line" pane)
             (error "boom")))"#,
    );
    press(&mut ed, "x");
    let selected = state(&ed);
    call(&mut ed, "copies-then-fails");
    ed.settle();

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_timer_callback_records_one_entry() {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "copies-later" "" (lambda (pane)
             (after! 0 (lambda ()
               (call! "copy-selection-on-next-line" pane)
               (call! "copy-selection-on-next-line" pane)))))"#,
    );
    press(&mut ed, "x");
    let selected = state(&ed);
    call(&mut ed, "copies-later");
    ed.settle();
    assert_ne!(state(&ed), selected, "the timer ran");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_script_that_closes_the_pane_it_selected_in_leaves_the_other_pane_alone() {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "copy-and-close" "" (lambda (pane)
             (call! "copy-selection-on-next-line" pane)
             (call! "pane-close" pane)))"#,
    );
    press(&mut ed, "x");
    let selected = state(&ed);
    vsplit_focused(&mut ed);
    ed.settle();
    call(&mut ed, "copy-and-close");
    ed.settle();
    assert_eq!(state(&ed), selected);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_script_command_of_motions_is_navigation() {
    let mut ed = editor_from("-[a]>bcdefgh\nxyz\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "three-right" "" (lambda (pane)
             (call! "move-right" pane)
             (call! "move-right" pane)
             (call! "move-right" pane)))"#,
    );
    press(&mut ed, "x");
    let selected = state(&ed);
    for _ in 0..2 {
        call(&mut ed, "three-right");
        ed.settle();
    }

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_mouse_drag_records_one_entry() {
    let mut ed = editor_from("-[0]>123456789\n");
    ed.state.settings.mouse_select = true;
    ed.resize_terminal(80, 25);
    draw(&mut ed);
    press(&mut ed, "x");
    let selected = state(&ed);
    for event in [
        mouse_left_down(2, 0),
        mouse_drag(4, 0),
        mouse_drag(6, 0),
        mouse_left_up(6, 0),
    ] {
        ed.handle_input(event);
        draw(&mut ed);
    }
    assert_eq!(state(&ed), "01-[23456]>789\n");
    press(&mut ed, "l");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), "01-[23456]>789\n");
    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_drag_after_typing_in_insert_mode_selects_from_the_cursor() {
    let mut ed = editor_from("-[0]>123456789\n");
    ed.state.settings.mouse_select = true;
    ed.resize_terminal(80, 25);
    draw(&mut ed);
    press(&mut ed, "x");
    ed.handle_input(mouse_left_down(2, 0));
    draw(&mut ed);
    press(&mut ed, "iz");
    draw(&mut ed);

    ed.handle_input(mouse_drag(6, 0));

    assert_eq!(state(&ed), "01z-[2345]>6789\n");
}

#[test]
fn clicks_are_navigation() {
    let mut ed = editor_from("-[0]>123456789\n");
    ed.resize_terminal(80, 25);
    draw(&mut ed);
    press(&mut ed, "x");
    let selected = state(&ed);
    for x in [2, 5] {
        for event in [mouse_left_down(x, 0), mouse_left_up(x, 0)] {
            ed.handle_input(event);
            ed.settle();
        }
    }

    undo_selection(&mut ed);

    assert_eq!(state(&ed), selected);
}

#[test]
fn a_wheel_notch_that_moves_nothing_keeps_a_drag_recorded() {
    let mut ed = editor_from("-[0]>123456789\nabcdefghij\n");
    ed.state.settings.mouse_select = true;
    ed.resize_terminal(80, 25);
    draw(&mut ed);
    for event in [
        mouse_left_down(2, 0),
        mouse_drag(6, 0),
        mouse_wheel_at(6, 0, false),
        mouse_left_up(6, 0),
    ] {
        ed.handle_input(event);
        ed.settle();
        draw(&mut ed);
    }
    assert_eq!(state(&ed), "01-[23456]>789\nabcdefghij\n");
    press(&mut ed, "j");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), "01-[23456]>789\nabcdefghij\n");
}

#[test]
fn placing_the_cursor_in_a_buffer_the_pane_never_showed_records_nothing() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.open_scratch(BufferText::from("xyz\n"));
    let pane = focused_pane(&ed);
    live_host!(ed)
        .goto_location_buffer(hume_scripting::host::ShownHandle::for_test(pane), bid, 2)
        .expect("goto-location! must succeed");
    ed.settle();
    assert_eq!(state(&ed), "xy-[z]>\n");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), "xy-[z]>\n");
}

#[test]
fn a_vertical_move_that_only_latches_a_column_adds_no_entry() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "x");
    let first_line = state(&ed);
    press(&mut ed, "jx");
    let last_line = state(&ed);
    ed.execute_keymap_command("move-down", Count::UNTYPED, true);
    ed.settle();
    assert_eq!(state(&ed), last_line);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), first_line);
}

#[test]
fn moving_several_cursors_records_nothing() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\neee\n");
    press(&mut ed, "C");
    let copied = state(&ed);
    press(&mut ed, "jj");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), copied);
}

#[test]
fn a_motion_in_extend_mode_records() {
    let mut ed = editor_from("-[a]>bcdef\nxyz\n");
    for _ in 0..2 {
        ed.execute_keymap_command("move-right", Count::UNTYPED, true);
        ed.settle();
    }
    let extended = state(&ed);
    press(&mut ed, "j");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), extended);
}

#[test]
fn a_half_page_scroll_is_navigation() {
    let mut ed = editor_from(&format!("-[a]>\n{}", "b\n".repeat(40)));
    press(&mut ed, "C");
    let copied = state(&ed);
    ed.feed_event(key_ctrl('d'));
    assert_ne!(state(&ed), copied);
    press(&mut ed, "j");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), copied);
}

/// An edit's name, the state it starts from and the keys or call that make it.
type NamedEdit = (&'static str, &'static str, fn(&mut Editor));

/// Each edit that acts on the selections, run on the selections of a state.
fn edits() -> [NamedEdit; 9] {
    [
        ("delete", "-[foo]> 12 bar\n", |ed| press(ed, "d")),
        ("change", "-[foo]> 12 bar\n", |ed| {
            press(ed, "cX");
            ed.feed_event(key_esc());
        }),
        ("paste", "-[foo]> 12 bar\n", |ed| press(ed, "yp")),
        ("replace", "-[foo]> 12 bar\n", |ed| press(ed, "rz")),
        ("join", "-[foo\n]>bar\nbaz\n", |ed| press(ed, "J")),
        ("uppercase", "-[foo]> 12 bar\n", |ed| {
            call(ed, "make-text-uppercase");
            ed.settle();
        }),
        ("indent", "-[foo]> 12 bar\n", |ed| press(ed, ">")),
        ("unindent", "  -[foo]> 12 bar\n", |ed| press(ed, "<")),
        ("increment", "foo -[12]> bar\n", |ed| {
            ed.feed_event(key_ctrl('a'))
        }),
    ]
}

#[test]
fn an_undone_edit_brings_back_the_selection_it_was_made_with() {
    for (name, selected, edit) in edits() {
        let mut ed = editor_from(selected);
        edit(&mut ed);
        press(&mut ed, "u;");

        undo_selection(&mut ed);

        assert_eq!(state(&ed), selected, "{name}");
    }
}

#[test]
fn a_redone_edit_offers_what_the_edit_itself_offers() {
    for (name, selected, edit) in edits() {
        let mut redone = editor_from(selected);
        edit(&mut redone);
        press(&mut redone, "uU;");
        let mut plain = editor_from(selected);
        edit(&mut plain);
        press(&mut plain, ";");

        undo_selection(&mut redone);
        undo_selection(&mut plain);

        assert_eq!(state(&redone), state(&plain), "{name}");
    }
}

#[test]
fn replacing_offers_what_it_wrote() {
    let mut ed = editor_from("-[a]>bc xyz\n");
    press(&mut ed, "miwrz");
    let replaced = state(&ed);
    assert_eq!(replaced, "-[zzz]> xyz\n");
    press(&mut ed, "l");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), replaced);
}

#[test]
fn indenting_offers_the_selection_it_moved() {
    let mut ed = editor_from("-[f]>oo bar\n");
    press(&mut ed, "miw>");
    let indented = state(&ed);
    press(&mut ed, ";");

    undo_selection(&mut ed);

    assert_eq!(state(&ed), indented);
}

#[test]
fn redo_from_the_undo_tree_stays_while_the_text_is_held() {
    let mut ed = editor_from("-[a]>bc\ndef\n");
    press(&mut ed, "miw>jmiw>;miw");
    let live = state(&ed);
    undo_selection(&mut ed);
    let landed = state(&ed);
    assert_ne!(landed, live);
    let bid = ed.focused_buffer_id();
    let _hold = ed.state.buffers.get_mut(bid).hold_text();

    redo_selection(&mut ed);

    assert_eq!(state(&ed), landed);
    assert_eq!(
        status(&ed),
        format!("{} is busy: a pipe is running", ed.doc().display_name())
    );
}

#[test]
fn selection_undo_offers_only_the_edits_its_pane_made() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");
    press(&mut ed, "xyp");
    vsplit_focused(&mut ed);
    ed.settle();
    press(&mut ed, "gg");
    let moved = state(&ed);

    undo_selection(&mut ed);

    assert_eq!(state(&ed), moved);
}

#[test]
fn a_walk_whose_tree_step_was_trimmed_resumes_from_the_newest_tree_step() {
    let mut ed = editor_from("top\n-[a]>bc\ndef\n");
    ed.doc_mut().set_undo_levels(2);
    let pane_a = ed.view.focused_pane();
    press(&mut ed, "miw>jmiw>");
    let second_edit = state(&ed);
    press(&mut ed, ";");
    press(&mut ed, "2mu");
    assert!(state(&ed).contains("-[abc]>"), "sanity: on the first edit");
    vsplit_focused(&mut ed);
    press(&mut ed, "gg");
    press(&mut ed, "iQQ");
    ed.feed_event(key_esc());
    ed.switch_focused_pane(pane_a);
    ed.settle();
    assert!(
        ed.doc().revision(1).is_none(),
        "sanity: the first edit was trimmed"
    );

    undo_selection(&mut ed);

    assert_eq!(state(&ed), format!("QQ{second_edit}"));
}

#[test]
fn a_reload_keeps_the_selection_history_in_order() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("reload.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").expect("the file is written");
    let mut ed = Editor::for_testing(BufferText::from("scratch\n"));
    ed.execute_typed("e", path.to_str())
        .expect(":e opens the file");
    press(&mut ed, "x");
    let first = state(&ed);
    press(&mut ed, "jx");
    assert_ne!(state(&ed), first, "sanity: a second selection");
    std::fs::write(&path, "zero\none\ntwo\nthree\n").expect("the file is rewritten");
    ed.execute_typed("e!", None).expect(":e! reloads the file");

    undo_selection(&mut ed);
    assert_eq!(state(&ed), format!("zero\n{first}"));
    undo_selection(&mut ed);

    assert_eq!(state(&ed), format!("zero\n{first}"));
}

#[test]
fn selection_undo_in_a_read_only_buffer_stops_before_the_undo_tree() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test message");
    ed.execute_typed("messages", None).unwrap();
    ed.settle();

    undo_selection(&mut ed);

    assert_eq!(status(&ed), read_only_refusal(&ed));
}
