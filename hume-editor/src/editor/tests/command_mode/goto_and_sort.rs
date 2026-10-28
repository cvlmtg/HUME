//! `:goto` and `:sort`.

use super::*;
use pretty_assertions::assert_eq;

// ── :goto / bare :N line-jump ─────────────────────────────────────────────────

/// `:goto N` places the cursor at the start of line N (1-based).
#[test]
fn colon_goto_moves_to_line() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":goto 3");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(2), // 0-indexed: line 3 → index 2
        ":goto 3 must land on line 3 (0-indexed: 2)"
    );
}

/// Bare `:3` is accepted as shorthand for `:goto 3`.
#[test]
fn colon_bare_number_moves_to_line() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":3");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(2),
        ":3 must land on line 3 (0-indexed: 2)"
    );
}

/// Past-EOF numbers clamp to the last content line.
/// 20-line buffer → last content index is 19.
#[test]
fn colon_goto_clamps_past_eof() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":goto 9999");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(19),
        ":goto 9999 must clamp to last content line (index 19)"
    );
}

/// Bare large number also clamps.
#[test]
fn colon_bare_number_clamps_past_eof() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":9999");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(19),
        ":9999 must clamp to last content line (index 19)"
    );
}

/// `:goto 0` is an error: line numbers start at 1.
#[test]
fn colon_goto_zero_is_error() {
    let mut ed = jump_editor(5);
    let before = state(&ed);
    type_cmd(&mut ed, ":goto 0");
    assert_eq!(state(&ed), before, ":goto 0 must not move the cursor");
    assert!(
        ed.state.status_msg.is_some(),
        ":goto 0 must report an error"
    );
}

/// Non-numeric argument reports an error and leaves the cursor unmoved.
#[test]
fn colon_goto_invalid_arg_is_error() {
    let mut ed = jump_editor(5);
    let before = state(&ed);
    type_cmd(&mut ed, ":goto abc");
    assert_eq!(state(&ed), before, ":goto abc must not move the cursor");
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .unwrap_or("")
            .contains("invalid line number"),
        ":goto abc must report 'invalid line number', got: {:?}",
        ed.state.status_msg
    );
}

/// `:goto` with no argument reports an error.
#[test]
fn colon_goto_no_arg_is_error() {
    let mut ed = jump_editor(5);
    let before = state(&ed);
    type_cmd(&mut ed, ":goto");
    assert_eq!(
        state(&ed),
        before,
        ":goto with no arg must not move the cursor"
    );
    assert!(
        ed.state.status_msg.is_some(),
        ":goto with no arg must report an error"
    );
}

/// `:goto` records the pre-jump position so `Ctrl-o` returns to it.
#[test]
fn colon_goto_records_jump() {
    let mut ed = jump_editor(5);
    let before = state(&ed);

    type_cmd(&mut ed, ":goto 15");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_view().primary().head().offset()),
        hume_rope::line::ContentLine::new(14), // 0-indexed: line 15 → index 14
        ":goto 15 must land on line 15 (index 14)"
    );

    // Ctrl-o must restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl-o must restore the pre-goto position"
    );
}

/// `:goto` to the line the cursor is already on is a no-op and must not
/// truncate forward jump-list history.
#[test]
fn colon_goto_noop_does_not_clobber_forward_history() {
    let mut ed = jump_editor(10);

    // `gg` records a jump, puts us at line 0.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Jump backward to the original line-10 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, at_top);

    // `:goto 11`: already on line 11 (index 10), so a no-op.
    type_cmd(&mut ed, ":goto 11");
    assert_eq!(
        state(&ed),
        back_at_start,
        ":goto to the current line must not move"
    );

    // Forward history (the jump to line 0) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        at_top,
        "a no-op :goto must not have truncated forward jump-list history"
    );
}

// ── :sort ─────────────────────────────────────────────────────────────────────

#[test]
fn sort_end_to_end_through_the_minibuffer() {
    let mut ed = editor_from("C -{B}> D\nF -[A]> G\n");
    submit(&mut ed, "sort");
    assert_eq!(state(&ed), "F -[A]> G\nC -{B}> D\n");
}

#[test]
fn sort_reverse_flag_through_the_minibuffer() {
    let mut ed = editor_from("-{a}>\n-[b]>\n");
    submit(&mut ed, "sort -r");
    assert_eq!(state(&ed), "-[b]>\n-{a}>\n");
}

#[test]
fn sort_insensitive_flag_through_the_minibuffer() {
    let mut ed = editor_from("-{Banana}>\n-[apple]>\n");
    submit(&mut ed, "sort -i");
    assert_eq!(state(&ed), "-[apple]>\n-{Banana}>\n");
}

#[test]
fn sort_bundled_flags_through_the_minibuffer() {
    // `-ri` = reverse + insensitive, bundled into one token.
    let mut ed = editor_from("-{Banana}>\n-[apple]>\n");
    submit(&mut ed, "sort -ri");
    assert_eq!(state(&ed), "-{Banana}>\n-[apple]>\n");
}

#[test]
fn sort_long_flags_through_the_minibuffer() {
    // `-r`/`-i` and their `--reverse`/`--insensitive` long forms are parsed
    // by the same match arm, so exercise the long forms directly rather than
    // trusting that pairing to hold.
    let mut ed = editor_from("-{Banana}>\n-[apple]>\n");
    submit(&mut ed, "sort --reverse --insensitive");
    assert_eq!(state(&ed), "-{Banana}>\n-[apple]>\n");
}

#[test]
fn sort_no_adjacent_lines_reports_info_without_logging() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    ed.execute_typed("sort", None)
        .expect("a single line is refused, not an error");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("sort needs at least two adjacent rows")
    );
    assert_eq!(
        ed.state.message_log.totals(),
        (0, 0),
        "NoAdjacentLines is Severity::Info: a boundary condition, not a failure worth \
         keeping in message_log"
    );
    assert_eq!(state(&ed), before, "a refusal must not touch the buffer");
}

#[test]
fn sort_already_sorted_reports_info_without_logging() {
    let mut ed = editor_from("-{a}>\n-[b]>\n");
    ed.execute_typed("sort", None)
        .expect("already-sorted input is refused, not an error");
    assert_eq!(ed.state.status_msg.as_deref(), Some("already sorted"));
    assert_eq!(
        ed.state.message_log.totals(),
        (0, 0),
        "AlreadySorted is Severity::Info; it must never reach message_log"
    );
}

#[test]
fn sort_unknown_flag_reports_an_error() {
    let mut ed = editor_from("-{b}>\n-[a]>\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("sort -z", None)
        .expect_err("-z is not a recognized flag");
    assert_eq!(err.message(), "unknown flag: -z");
    assert_eq!(
        state(&ed),
        before,
        "an argument error must not touch the buffer"
    );
}

#[test]
fn sort_unknown_long_flag_reports_a_flag_error_not_an_argument_error() {
    // An unrecognized `--`-prefixed token is a flag mistake, not a
    // positional argument, so it gets the flag error rather than the
    // "unknown argument" one positionals get.
    let mut ed = editor_from("-{b}>\n-[a]>\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("sort --bogus", None)
        .expect_err("--bogus is not a recognized flag");
    assert_eq!(err.message(), "unknown flag: --bogus");
    assert_eq!(
        state(&ed),
        before,
        "an argument error must not touch the buffer"
    );
}

#[test]
fn sort_bang_is_rejected() {
    let mut ed = editor_from("-{b}>\n-[a]>\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("sort!", None)
        .expect_err(":sort! must be rejected");
    assert_eq!(err.message(), "`:sort` takes no `!`");
    assert_eq!(state(&ed), before);
}

/// `:sort` on a read-only buffer is refused like any other edit: the text is
/// untouched and an armed `"<reg>` prefix is cleared.
#[test]
fn sort_on_read_only_buffer_is_blocked() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "one".to_string());
    ed.report(Severity::Warning, "two".to_string());
    ed.execute_typed("messages", None).unwrap();
    let before = state(&ed);
    ed.state.register_prefix = Some('a');

    let err = ed
        .execute_typed("sort", None)
        .expect_err("a read-only buffer must refuse :sort");
    assert_eq!(err.message(), read_only_refusal(&ed));
    assert_eq!(
        state(&ed),
        before,
        ":sort must not mutate a read-only buffer"
    );
    assert_eq!(ed.state.register_prefix, None);
}

#[test]
fn sort_undo_restores_text_and_selections_in_one_step() {
    let mut ed = editor_from("C -{B}> D\nF -[A]> G\n");
    let before = state(&ed);
    submit(&mut ed, "sort");
    assert_eq!(state(&ed), "F -[A]> G\nC -{B}> D\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), before, "undo must restore text and selections");
    assert!(
        !ed.doc().can_undo(),
        ":sort must record exactly one undo step"
    );
}

#[test]
fn sort_on_already_sorted_input_leaves_buffer_clean() {
    // `sort_lines` refuses already-ordered input instead of making an
    // identity edit, which would mark the buffer dirty.
    let mut ed = editor_from("-{a}>\n-[b]>\n");
    assert!(!focused_unsaved(&ed));
    submit(&mut ed, "sort");
    assert!(
        !focused_unsaved(&ed),
        "sorting already-sorted rows must not touch the undo history"
    );
}
