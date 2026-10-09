//! A native command's operand (`CommandArg`) reaching it from a script's
//! `call!` and from a key binding, and being refused when it doesn't fit.

use super::*;
use crate::editor::message_log::Severity;
use crate::editor::minibuf::history::HistoryKind;

/// Attach a scripting host defining `probe`, a no-parameter command whose
/// body is `body`, run on the focused pane.
fn with_probe(ed: &mut Editor, body: &str) {
    attach_steel(
        ed,
        &format!(r#"(define-command! "probe" "" (lambda () {body}))"#),
    );
}

fn run_probe(ed: &mut Editor) {
    ed.execute_keymap_command("probe".into(), Some(1), false);
}

fn text(ed: &Editor) -> String {
    ed.state
        .buffers
        .get(ed.focused_buffer_id())
        .text()
        .to_string()
}

fn has_error(ed: &Editor, needle: &str) -> bool {
    ed.state
        .message_log
        .entries()
        .any(|e| e.severity == Severity::Error && e.text.contains(needle))
}

// ── Character commands from a script ────────────────────────────────────────

#[test]
fn call_find_forward_with_a_char_moves_to_it() {
    let mut ed = editor_from("-[h]>ello a world\n");
    with_probe(&mut ed, r#"(call! "find-forward" (focused-pane) #\a)"#);
    run_probe(&mut ed);
    assert_eq!(state(&ed), "hello -[a]> world\n");
}

#[test]
fn call_replace_with_a_steel_char_replaces_the_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "replace" (focused-pane) #\x)"#);
    run_probe(&mut ed);
    assert_eq!(text(&ed), "xello\n");
}

#[test]
fn call_surround_add_wraps_the_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "surround-add" (focused-pane) #\()"#);
    run_probe(&mut ed);
    assert_eq!(text(&ed), "(h)ello\n");
}

// ── Operands that don't fit ─────────────────────────────────────────────────

#[test]
fn call_replace_with_a_string_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "replace" (focused-pane) "x")"#);
    run_probe(&mut ed);
    assert_eq!(text(&ed), "hello\n");
    assert!(has_error(&ed, "'replace': needs a character, got a string"));
}

#[test]
fn call_with_an_operand_to_a_command_taking_none_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "move-right" (focused-pane) "x")"#);
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(has_error(&ed, "'move-right': takes no argument"));
}

// ── A missing character is read from the next key ───────────────────────────

#[test]
fn call_replace_without_a_char_reads_the_next_key() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "replace" (focused-pane))"#);
    run_probe(&mut ed);
    assert_eq!(text(&ed), "hello\n");
    ed.feed_key(key('x'));
    assert_eq!(text(&ed), "xello\n");
}

#[test]
fn replace_dispatched_without_a_char_reads_the_next_key() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_keymap_command("replace".into(), Some(1), false);
    ed.feed_key(key('x'));
    assert_eq!(text(&ed), "xello\n");
}

#[test]
fn bind_key_to_a_character_command_reads_the_next_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(&mut ed, tmp.path(), r#"(bind-key! 'normal "Z" "replace")"#);
    ed.feed_key(key('Z'));
    ed.feed_key(key('x'));
    assert_eq!(text(&ed), "xello\n");
}

#[test]
fn a_key_other_than_a_char_cancels_the_read() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('r'));
    ed.feed_key(key_esc());
    ed.feed_key(key('l'));
    assert_eq!(state(&ed), "h-[e]>llo\n");
}

#[test]
fn till_key_moves_to_the_char_read_in_normal_mode() {
    let mut ed = editor_from("-[ab]>cdexf\n");
    ed.feed_key(key('t'));
    ed.feed_key(key('x'));
    assert_eq!(state(&ed), "abcd-[e]>xf\n");
}

#[test]
fn till_key_extends_to_the_char_read_in_extend_mode() {
    let mut ed = editor_from("-[ab]>cdexf\n");
    ed.feed_key(key('e'));
    ed.feed_key(key('t'));
    ed.feed_key(key('x'));
    assert_eq!(state(&ed), "-[abcde]>xf\n");
    assert_eq!(ed.state.mode(), Mode::Extend);
}

#[test]
fn scrolling_leaves_the_read_pending() {
    let mut lines = String::from("-[l]>ine0\n");
    for i in 1..30 {
        lines.push_str(&format!("line{i}\n"));
    }
    let mut ed = editor_from(&lines);
    ed.feed_key(key('f'));
    ed.handle_input(mouse_wheel(true));
    let pid = ed.state.focus.id();
    assert!(ed.view.panes[pid].viewport.top().line > hume_rope::line::ContentLine::new(0));
    assert!(reading_char(&ed));
    ed.feed_key(key('e'));
    let head = ed.current_view().primary().head().offset().index();
    assert_eq!(text(&ed).chars().nth(head), Some('e'));
}

#[test]
fn a_paste_cancels_the_read_and_is_dropped() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('r'));
    ed.handle_input(TerminalEvent::Paste("x".to_string()));
    assert!(!reading_char(&ed));
    assert_eq!(text(&ed), "hello\n");
}

// ── on-next-char! ───────────────────────────────────────────────────────────

#[test]
fn on_next_char_hands_the_next_key_to_the_callback() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(
        &mut ed,
        r#"(on-next-char! (lambda (ch) (call! "replace" (focused-pane) ch)))"#,
    );
    run_probe(&mut ed);
    ed.feed_key(key('x'));
    ed.settle();
    assert_eq!(text(&ed), "xello\n");
}

/// The probe replaces the selection with `z` when the callback gets `#f`.
const REPLACE_Z_ON_CANCEL: &str =
    r#"(on-next-char! (lambda (ch) (if ch (+ 1 0) (call! "replace" (focused-pane) #\z))))"#;

#[test]
fn on_next_char_hands_false_to_the_callback_on_escape() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, REPLACE_Z_ON_CANCEL);
    run_probe(&mut ed);
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(text(&ed), "zello\n");
}

#[test]
fn on_next_char_hands_false_to_the_callback_on_a_mouse_click() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, REPLACE_Z_ON_CANCEL);
    run_probe(&mut ed);
    ed.handle_input(mouse_left_down(0, 0));
    ed.settle();
    assert_eq!(text(&ed), "zello\n");
}

#[test]
fn on_next_char_chains_for_several_chars() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(
        &mut ed,
        r#"(on-next-char!
             (lambda (a)
               (call! "replace" (focused-pane) a)
               (on-next-char! (lambda (b) (call! "surround-add" (focused-pane) b)))))"#,
    );
    run_probe(&mut ed);
    ed.feed_key(key('x'));
    ed.settle();
    ed.feed_key(key('('));
    ed.settle();
    assert_eq!(text(&ed), "(x)ello\n");
}

#[test]
fn reads_queued_by_one_body_take_keys_in_the_order_made() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(
        &mut ed,
        r#"(on-next-char! (lambda (a) (call! "replace" (focused-pane) a)))
           (on-next-char! (lambda (b) (call! "surround-add" (focused-pane) b)))"#,
    );
    run_probe(&mut ed);
    ed.feed_key(key('x'));
    ed.settle();
    ed.feed_key(key('('));
    ed.settle();
    assert_eq!(text(&ed), "(x)ello\n");
}

#[test]
fn char_commands_called_without_chars_take_keys_in_call_order() {
    let mut ed = editor_from("-[h]>ello a\n");
    with_probe(
        &mut ed,
        r#"(call! "replace" (focused-pane))
           (call! "find-forward" (focused-pane))"#,
    );
    run_probe(&mut ed);
    ed.feed_key(key('x'));
    ed.feed_key(key('a'));
    assert_eq!(state(&ed), "xello -[a]>\n");
}

#[test]
fn a_read_made_under_a_popup_queues_behind_the_pending_one() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(
        &mut ed,
        r#"(on-next-char! (lambda (a) (call! "replace" (focused-pane) a)))
           (show-popup! (focused-pane) "type a delimiter" #:kind 'scrollable)
           (on-next-char! (lambda (b) (call! "surround-add" (focused-pane) b)))"#,
    );
    run_probe(&mut ed);
    ed.feed_key(key('x'));
    ed.settle();
    ed.feed_key(key('('));
    ed.settle();
    assert_eq!(text(&ed), "(x)ello\n");
}

// ── Search with a pattern from a script ─────────────────────────────────────

fn search_history(ed: &Editor, kind: HistoryKind) -> Vec<String> {
    ed.state
        .history
        .get(kind)
        .entries()
        .iter()
        .cloned()
        .collect()
}

fn jump_count(ed: &Editor) -> usize {
    ed.state.panes.jumps[ed.state.focus.id()].len()
}

#[test]
fn call_search_forward_with_a_pattern_acts_as_typed_and_confirmed() {
    let mut ed = editor_from("-[h]>ello world\n");
    let jumps_before = jump_count(&ed);
    with_probe(
        &mut ed,
        r#"(call! "search-forward" (focused-pane) "world")"#,
    );
    run_probe(&mut ed);
    assert_eq!(state(&ed), "hello -[world]>\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(reg(&ed, 's'), vec!["world"]);
    assert_eq!(
        search_history(&ed, HistoryKind::SearchForward),
        vec!["world"]
    );
    assert!(search_history(&ed, HistoryKind::SearchBackward).is_empty());
    assert_eq!(
        jump_count(&ed),
        jumps_before + 1,
        "the pre-search position must be recorded"
    );
}

#[test]
fn call_search_backward_with_a_pattern_selects_the_previous_match() {
    let mut ed = editor_from("hello -[w]>orld\n");
    with_probe(&mut ed, r#"(call! "search-backward" (focused-pane) "hel")"#);
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[hel]>lo world\n");
    assert_eq!(
        search_history(&ed, HistoryKind::SearchBackward),
        vec!["hel"]
    );
}

#[test]
fn search_next_after_a_script_search_jumps_to_the_next_match() {
    let mut ed = editor_from("-[x]> abc abc\n");
    with_probe(&mut ed, r#"(call! "search-forward" (focused-pane) "abc")"#);
    run_probe(&mut ed);
    assert_eq!(state(&ed), "x -[abc]> abc\n");
    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "x abc -[abc]>\n");
}

#[test]
fn call_search_forward_with_extend_extends_from_the_anchor() {
    let mut ed = editor_from("-[h]>ello world\n");
    with_probe(
        &mut ed,
        r#"(call! "search-forward" (focused-pane) 1 #t "world")"#,
    );
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[hello world]>\n");
}

#[test]
fn call_search_forward_with_a_char_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "search-forward" (focused-pane) #\l)"#);
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(has_error(
        &ed,
        "'search-forward': takes a text argument, got a character"
    ));
}

#[test]
fn call_search_forward_with_an_invalid_pattern_writes_nothing() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(&mut ed, r#"(call! "search-forward" (focused-pane) "(")"#);
    run_probe(&mut ed);
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(has_error(&ed, "invalid pattern"));
    assert!(reg(&ed, 's').is_empty());
    assert!(search_history(&ed, HistoryKind::SearchForward).is_empty());
    assert!(
        ed.state
            .buffers
            .get(ed.focused_buffer_id())
            .search_pattern
            .is_none()
    );
}

/// An empty pattern, or one that is only flags, is an invalid pattern: the
/// previous search stays in place and `call!` answers `#f`.
#[test]
fn call_search_forward_with_an_empty_pattern_keeps_the_last_search() {
    for empty in ["", "m/", "v/"] {
        let mut ed = editor_from("-[h]>ello\n");
        with_probe(
            &mut ed,
            &format!(
                r#"(call! "search-forward" (focused-pane) "lo")
                   (if (call! "search-forward" (focused-pane) "{empty}") (+ 1 0) {REPLACE_ON_FALSE})"#
            ),
        );
        run_probe(&mut ed);
        assert_eq!(text(&ed), "helxx\n", "pattern {empty:?}");
        assert!(has_error(&ed, "invalid pattern"), "pattern {empty:?}");
        assert_eq!(reg(&ed, 's'), vec!["lo"], "pattern {empty:?}");
        assert_eq!(
            search_history(&ed, HistoryKind::SearchForward),
            vec!["lo"],
            "pattern {empty:?}"
        );
        assert_eq!(
            ed.state
                .buffers
                .get(ed.focused_buffer_id())
                .search_pattern
                .as_ref()
                .map(|p| p.pattern_str.as_str()),
            Some("lo"),
            "pattern {empty:?}"
        );
    }
}

/// Like Enter on a pattern with no match: the pattern, register, and history
/// are written and the selection stays; `call!` answers `#f`, which the probe
/// turns into a visible move.
#[test]
fn call_search_forward_with_no_match_records_it_and_returns_false() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(
        &mut ed,
        r#"(if (call! "search-forward" (focused-pane) "zzz")
               (+ 1 0)
               (call! "move-right" (focused-pane)))"#,
    );
    run_probe(&mut ed);
    assert_eq!(state(&ed), "h-[e]>llo\n", "call! must return #f");
    assert_eq!(reg(&ed, 's'), vec!["zzz"]);
    assert_eq!(search_history(&ed, HistoryKind::SearchForward), vec!["zzz"]);
    assert!(
        ed.state
            .buffers
            .get(ed.focused_buffer_id())
            .search_pattern
            .is_some()
    );
}

// ── Sift with a pattern from a script ───────────────────────────────────────

#[test]
fn call_sift_within_with_a_pattern_keeps_each_match() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    with_probe(&mut ed, r#"(call! "sift-within" (focused-pane) "ab")"#);
    run_probe(&mut ed);
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.current_view().len(), 2);
    assert_eq!(ed.current_view().primary().anchor().offset(), co(0));
    assert_eq!(ed.current_view().primary().head().offset(), co(1));
    assert!(reg(&ed, 's').is_empty(), "sift is not a search");
}

/// The `:trim-trailing` typed command `plugins.md` shows: sift each line's
/// trailing blanks out of the selected lines, or out of the whole buffer when
/// no selection covers whole lines, then delete them.
const TRIM_TRAILING: &str = r#"(define-typed-command! "trim-trailing" "Remove trailing blanks."
     (lambda (pane)
       (when (selections-charwise? pane)
         (call! "select-all" pane))
       (when (call! "sift-within" pane "(?m)[ \t]+$")
         (call! "delete-selection" pane))))"#;

#[test]
fn trim_trailing_without_line_selections_trims_the_whole_buffer() {
    let mut ed = editor_from("-[a]>  \nb \t\nc\n");
    attach_steel(&mut ed, TRIM_TRAILING);
    type_cmd(&mut ed, ":trim-trailing");
    assert_eq!(text(&ed), "a\nb\nc\n");
}

#[test]
fn trim_trailing_with_line_selections_trims_only_those_lines() {
    let mut ed = editor_from("-[a  \n]>b \t\nc \n");
    attach_steel(&mut ed, TRIM_TRAILING);
    type_cmd(&mut ed, ":trim-trailing");
    assert_eq!(text(&ed), "a\nb \t\nc \n");
}

#[test]
fn trim_trailing_on_a_buffer_without_trailing_blanks_changes_nothing() {
    let mut ed = editor_from("-[a]>\nb\nc\n");
    attach_steel(&mut ed, TRIM_TRAILING);
    type_cmd(&mut ed, ":trim-trailing");
    assert_eq!(text(&ed), "a\nb\nc\n");
}

#[test]
fn trim_trailing_on_selected_lines_without_trailing_blanks_changes_nothing() {
    let mut ed = editor_from("-[a\n]>b \nc\n");
    attach_steel(&mut ed, TRIM_TRAILING);
    type_cmd(&mut ed, ":trim-trailing");
    assert_eq!(text(&ed), "a\nb \nc\n");
}

/// The probe replaces the selection with `x` when `call!` answers `#f`, so
/// the text shows both the `#f` and that the selection was left as it was.
const REPLACE_ON_FALSE: &str = r#"(call! "replace" (focused-pane) #\x)"#;

#[test]
fn call_sift_within_with_an_invalid_pattern_reports_and_keeps_the_selection() {
    let mut ed = editor_from("-[ab]> cd\n");
    with_probe(
        &mut ed,
        &format!(r#"(if (call! "sift-within" (focused-pane) "(") (+ 1 0) {REPLACE_ON_FALSE})"#),
    );
    run_probe(&mut ed);
    assert_eq!(text(&ed), "xx cd\n");
    assert!(has_error(&ed, "invalid pattern"));
}

#[test]
fn call_sift_within_with_no_match_returns_false_and_keeps_the_selection() {
    let mut ed = editor_from("-[ab]> cd\n");
    with_probe(
        &mut ed,
        &format!(r#"(if (call! "sift-within" (focused-pane) "zz") (+ 1 0) {REPLACE_ON_FALSE})"#),
    );
    run_probe(&mut ed);
    assert_eq!(text(&ed), "xx cd\n");
}

#[test]
fn call_sift_within_with_an_empty_pattern_reports_and_keeps_the_selection() {
    for empty in ["", "m/"] {
        let mut ed = editor_from("-[ab]> cd\n");
        with_probe(
            &mut ed,
            &format!(
                r#"(if (call! "sift-within" (focused-pane) "{empty}") (+ 1 0) {REPLACE_ON_FALSE})"#
            ),
        );
        run_probe(&mut ed);
        assert_eq!(text(&ed), "xx cd\n", "pattern {empty:?}");
        assert!(has_error(&ed, "invalid pattern"), "pattern {empty:?}");
    }
}

#[test]
fn call_sift_within_on_bare_cursors_returns_false() {
    let mut ed = editor_from("-[h]>ello\n");
    with_probe(
        &mut ed,
        &format!(r#"(if (call! "sift-within" (focused-pane) "h") (+ 1 0) {REPLACE_ON_FALSE})"#),
    );
    run_probe(&mut ed);
    assert_eq!(text(&ed), "xello\n");
}

// ── Enter at a prompt runs the command with the typed pattern ──────────────

fn type_keys(ed: &mut Editor, keys: &str) {
    for ch in keys.chars() {
        ed.feed_key(key(ch));
    }
}

#[test]
fn enter_on_an_invalid_search_pattern_reports_and_records_nothing() {
    let mut ed = editor_from("-[h]>ello\n");
    type_keys(&mut ed, "/(");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(has_error(&ed, "invalid pattern"));
    assert!(reg(&ed, 's').is_empty());
    assert!(search_history(&ed, HistoryKind::SearchForward).is_empty());
}

#[test]
fn enter_on_a_search_with_no_match_reports_it() {
    let mut ed = editor_from("-[h]>ello\n");
    type_keys(&mut ed, "/xyz");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert_eq!(ed.state.status_msg.as_deref(), Some("no match"));
    assert_eq!(reg(&ed, 's'), vec!["xyz"]);
}

#[test]
fn enter_on_an_invalid_sift_pattern_reports_and_keeps_the_selection() {
    let mut ed = editor_from("-[ab cd]>\n");
    type_keys(&mut ed, "s(");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "-[ab cd]>\n");
    assert!(has_error(&ed, "invalid pattern"));
}

// ── core:helix-surround, which reads its delimiters with on-next-char! ──────

const HELIX_SURROUND: &str =
    include_str!("../../../../runtime/plugins/core/helix-surround/plugin.scm");

fn feed_settled(ed: &mut Editor, keys: &str) {
    for ch in keys.chars() {
        ed.feed_key(key(ch));
        ed.settle();
    }
}

#[test]
fn helix_surround_md_deletes_the_pair_it_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(h-[e]>llo)\n");
    run(&mut ed, tmp.path(), HELIX_SURROUND);
    feed_settled(&mut ed, "md(");
    assert_eq!(text(&ed), "hello\n");
}

#[test]
fn helix_surround_mr_replaces_the_pair_with_the_second_char() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(h-[e]>llo)\n");
    run(&mut ed, tmp.path(), HELIX_SURROUND);
    feed_settled(&mut ed, "mr([");
    assert_eq!(text(&ed), "[hello]\n");
}
