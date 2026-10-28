//! A native command's operand (`CommandArg`) reaching it from a script's
//! `call!` and from a key binding, and being refused when it doesn't fit.

use super::*;
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
    ed.execute_keymap_command("probe".into(), Count::typed(1), false);
}

fn probe(ed: &mut Editor, body: &str) {
    with_probe(ed, body);
    run_probe(ed);
}

fn feed_settled(ed: &mut Editor, keys: &str) {
    for ch in keys.chars() {
        ed.feed_key(key(ch));
        ed.settle();
    }
}

fn text(ed: &Editor) -> String {
    ed.state
        .buffers
        .get(ed.focused_buffer_id())
        .text()
        .to_string()
}

// ── Character commands from a script ────────────────────────────────────────

#[test]
fn call_find_forward_with_a_char_moves_to_it() {
    let mut ed = editor_from("-[h]>ello a world\n");
    probe(&mut ed, r#"(call! "find-forward" (focused-pane) #\a)"#);
    assert_eq!(state(&ed), "hello -[a]> world\n");
}

#[test]
fn call_replace_with_a_steel_char_replaces_the_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "replace" (focused-pane) #\x)"#);
    assert_eq!(text(&ed), "xello\n");
}

#[test]
fn call_surround_add_wraps_the_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "surround-add" (focused-pane) #\()"#);
    assert_eq!(text(&ed), "(h)ello\n");
}

// ── Operands that don't fit ─────────────────────────────────────────────────

#[test]
fn call_replace_with_a_string_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "replace" (focused-pane) "x")"#);
    assert_eq!(text(&ed), "hello\n");
    assert!(has_error(&ed, "'replace': needs a character, got a string"));
}

#[test]
fn call_with_an_operand_to_a_command_taking_none_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "move-right" (focused-pane) "x")"#);
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(has_error(&ed, "'move-right': takes no argument"));
}

#[test]
fn call_replace_without_a_char_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "replace" (focused-pane))"#);
    assert!(has_error(&ed, "'replace': needs a character"));
    assert!(!reading_char(&ed));
    ed.feed_key(key('x'));
    assert_eq!(text(&ed), "hello\n");
}

// ── A key bound to a character command reads the next key ───────────────────

#[test]
fn replace_dispatched_without_a_char_reads_the_next_key() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_keymap_command("replace".into(), Count::typed(1), false);
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
    let pid = ed.view.focused_pane();
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
    probe(
        &mut ed,
        r#"(on-next-char! (lambda (ch) (call! "replace" (focused-pane) ch)))"#,
    );
    feed_settled(&mut ed, "x");
    assert_eq!(text(&ed), "xello\n");
}

/// The probe replaces the selection with `z` when the callback gets `#f`.
const REPLACE_Z_ON_CANCEL: &str =
    r#"(on-next-char! (lambda (ch) (if ch (+ 1 0) (call! "replace" (focused-pane) #\z))))"#;

#[test]
fn on_next_char_hands_false_to_the_callback_on_escape() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, REPLACE_Z_ON_CANCEL);
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(text(&ed), "zello\n");
}

#[test]
fn on_next_char_hands_false_to_the_callback_on_a_mouse_click() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, REPLACE_Z_ON_CANCEL);
    ed.handle_input(mouse_left_down(0, 0));
    ed.settle();
    assert_eq!(text(&ed), "zello\n");
}

#[test]
fn on_next_char_hands_false_to_the_callback_on_a_paste() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, REPLACE_Z_ON_CANCEL);
    ed.feed_paste("x");
    ed.settle();
    assert_eq!(text(&ed), "zello\n");
}

#[test]
fn on_next_char_chains_for_several_chars() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(
        &mut ed,
        r#"(on-next-char!
             (lambda (a)
               (call! "replace" (focused-pane) a)
               (on-next-char! (lambda (b) (call! "surround-add" (focused-pane) b)))))"#,
    );
    feed_settled(&mut ed, "x(");
    assert_eq!(text(&ed), "(x)ello\n");
}

#[test]
fn reads_queued_by_one_body_take_keys_in_the_order_made() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(
        &mut ed,
        r#"(on-next-char! (lambda (a) (call! "replace" (focused-pane) a)))
           (on-next-char! (lambda (b) (call! "surround-add" (focused-pane) b)))"#,
    );
    feed_settled(&mut ed, "x(");
    assert_eq!(text(&ed), "(x)ello\n");
}

#[test]
fn a_read_made_under_a_popup_queues_behind_the_pending_one() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(
        &mut ed,
        r#"(on-next-char! (lambda (a) (call! "replace" (focused-pane) a)))
           (show-popup! (focused-pane) "type a delimiter" #:kind 'scrollable)
           (on-next-char! (lambda (b) (call! "surround-add" (focused-pane) b)))"#,
    );
    feed_settled(&mut ed, "x(");
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
    ed.state.panes.entries[ed.view.focused_pane()].jumps.len()
}

#[test]
fn call_search_forward_with_a_pattern_acts_as_typed_and_confirmed() {
    let mut ed = editor_from("-[h]>ello world\n");
    let jumps_before = jump_count(&ed);
    probe(
        &mut ed,
        r#"(call! "search-forward" (focused-pane) "world")"#,
    );
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
    probe(&mut ed, r#"(call! "search-backward" (focused-pane) "hel")"#);
    assert_eq!(state(&ed), "-[hel]>lo world\n");
    assert_eq!(
        search_history(&ed, HistoryKind::SearchBackward),
        vec!["hel"]
    );
}

#[test]
fn search_next_after_a_script_search_jumps_to_the_next_match() {
    let mut ed = editor_from("-[x]> abc abc\n");
    probe(&mut ed, r#"(call! "search-forward" (focused-pane) "abc")"#);
    assert_eq!(state(&ed), "x -[abc]> abc\n");
    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "x abc -[abc]>\n");
}

#[test]
fn call_search_forward_with_extend_extends_from_the_anchor() {
    let mut ed = editor_from("-[h]>ello world\n");
    probe(
        &mut ed,
        r#"(call! "search-forward" (focused-pane) 1 #t "world")"#,
    );
    assert_eq!(state(&ed), "-[hello world]>\n");
}

#[test]
fn search_key_in_extend_mode_extends_from_the_anchor() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_key(key('e'));
    type_keys(&mut ed, "/wor");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "-[hello wor]>ld\n");
}

#[test]
fn call_search_forward_without_extend_in_extend_mode_moves() {
    let mut ed = editor_from("-[h]>ello world\n");
    with_probe(
        &mut ed,
        r#"(call! "search-forward" (focused-pane) 1 #f "wor")"#,
    );
    ed.feed_key(key('e'));
    run_probe(&mut ed);
    assert_eq!(state(&ed), "hello -[wor]>ld\n");
}

#[test]
fn call_search_forward_without_a_pattern_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "search-forward" (focused-pane))"#);
    assert!(has_error(&ed, "'search-forward': needs a pattern"));
    assert_eq!(ed.state.mode(), Mode::Normal);
}

/// The `prompt!` example in the user manual's `call!` section.
#[test]
fn a_pattern_read_with_prompt_runs_the_search() {
    let mut ed = editor_from("-[h]>ello world\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "search-prompt" "Search forward for a pattern you type."
             (lambda (pane)
               (prompt! pane "search:"
                 (lambda (pat) (when pat (call! "search-forward" pane pat))))))"#,
    );
    ed.execute_keymap_command("search-prompt".into(), Count::typed(1), false);
    type_keys(&mut ed, "wor");
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(state(&ed), "hello -[wor]>ld\n");
}

#[test]
fn call_sift_within_without_a_pattern_raises() {
    let mut ed = editor_from("-[hello]>\n");
    probe(&mut ed, r#"(call! "sift-within" (focused-pane))"#);
    assert!(has_error(&ed, "'sift-within': needs a pattern"));
    assert_eq!(ed.state.mode(), Mode::Normal);
}

#[test]
fn call_search_forward_with_a_char_raises() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "search-forward" (focused-pane) #\l)"#);
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(has_error(
        &ed,
        "'search-forward': takes a text argument, got a character"
    ));
}

#[test]
fn call_search_forward_with_an_invalid_pattern_writes_nothing() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(&mut ed, r#"(call! "search-forward" (focused-pane) "(")"#);
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
        probe(
            &mut ed,
            &format!(
                r#"(call! "search-forward" (focused-pane) "lo")
                   (if (call! "search-forward" (focused-pane) "{empty}") (+ 1 0) {REPLACE_ON_FALSE})"#
            ),
        );
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
    probe(
        &mut ed,
        r#"(if (call! "search-forward" (focused-pane) "zzz")
               (+ 1 0)
               (call! "move-right" (focused-pane)))"#,
    );
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
    probe(&mut ed, r#"(call! "sift-within" (focused-pane) "ab")"#);
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.current_view().len(), 2);
    assert_eq!(ed.current_view().primary().anchor().offset(), co(0));
    assert_eq!(ed.current_view().primary().head().offset(), co(1));
    assert!(reg(&ed, 's').is_empty(), "sift is not a search");
}

#[test]
fn enter_on_a_sift_reuses_the_selections_the_preview_found() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    type_keys(&mut ed, "sab");
    let scans = ed.state.sift_scans;
    ed.feed_key(key_enter());
    assert_eq!(ed.state.sift_scans, scans);
    let mut called = editor_from("-[ab cd ab]>\n");
    probe(&mut called, r#"(call! "sift-within" (focused-pane) "ab")"#);
    assert_eq!(state(&ed), state(&called));
}

#[test]
fn a_sift_after_a_cancelled_prompt_scans_its_own_pattern() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    type_keys(&mut ed, "sab");
    ed.feed_key(key_esc());
    probe(&mut ed, r#"(call! "sift-within" (focused-pane) "cd")"#);
    assert_eq!(state(&ed), "ab -[cd]> ab\n");
}

/// The `:trim-trailing` typed command `writing-plugins.md` shows: sift each line's
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
    probe(
        &mut ed,
        &format!(r#"(if (call! "sift-within" (focused-pane) "(") (+ 1 0) {REPLACE_ON_FALSE})"#),
    );
    assert_eq!(text(&ed), "xx cd\n");
    assert!(has_error(&ed, "invalid pattern"));
}

#[test]
fn call_sift_within_with_no_match_returns_false_and_keeps_the_selection() {
    let mut ed = editor_from("-[ab]> cd\n");
    probe(
        &mut ed,
        &format!(r#"(if (call! "sift-within" (focused-pane) "zz") (+ 1 0) {REPLACE_ON_FALSE})"#),
    );
    assert_eq!(text(&ed), "xx cd\n");
}

#[test]
fn call_sift_within_with_an_empty_pattern_reports_and_keeps_the_selection() {
    for empty in ["", "m/"] {
        let mut ed = editor_from("-[ab]> cd\n");
        probe(
            &mut ed,
            &format!(
                r#"(if (call! "sift-within" (focused-pane) "{empty}") (+ 1 0) {REPLACE_ON_FALSE})"#
            ),
        );
        assert_eq!(text(&ed), "xx cd\n", "pattern {empty:?}");
        assert!(has_error(&ed, "invalid pattern"), "pattern {empty:?}");
    }
}

#[test]
fn call_sift_within_on_bare_cursors_returns_false() {
    let mut ed = editor_from("-[h]>ello\n");
    probe(
        &mut ed,
        &format!(r#"(if (call! "sift-within" (focused-pane) "h") (+ 1 0) {REPLACE_ON_FALSE})"#),
    );
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
fn enter_on_an_invalid_search_pattern_keeps_no_previewed_prefix() {
    let mut ed = editor_from("-[h]>ello\n");
    type_keys(&mut ed, "/l(");
    ed.feed_key(key_enter());
    assert!(has_error(&ed, "invalid pattern"));
    assert!(
        ed.state
            .buffers
            .get(ed.focused_buffer_id())
            .search_pattern
            .is_none()
    );
}

#[test]
fn enter_on_a_search_reuses_the_matches_the_preview_found() {
    let mut ed = editor_from("-[h]>ello\n");
    type_keys(&mut ed, "/ll");
    let bid = ed.focused_buffer_id();
    let scans = ed.state.buffers.get(bid).search_scans;
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "he-[ll]>o\n");
    assert_eq!(ed.state.buffers.get(bid).search_scans, scans);
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

#[test]
fn helix_surround_md_deletes_the_pair_it_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(h-[e]>llo)\n");
    run(&mut ed, tmp.path(), HELIX_SURROUND);
    feed_settled(&mut ed, "md(");
    assert_eq!(text(&ed), "hello\n");
}

/// `md` is not repeatable itself: `.` repeats the delete its callback
/// ran, on the pair the recorded selection steps find.
#[test]
fn dot_after_helix_md_deletes_the_next_pair() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(h-[e]>llo) (world)\n");
    run(&mut ed, tmp.path(), HELIX_SURROUND);
    feed_settled(&mut ed, "md(");
    assert_eq!(text(&ed), "hello (world)\n");
    select_marked(&mut ed, "hello (w-[o]>rld)\n");
    feed_settled(&mut ed, ".");
    assert_eq!(text(&ed), "hello world\n");
}

#[test]
fn helix_surround_mr_replaces_the_pair_with_the_second_char() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(h-[e]>llo)\n");
    run(&mut ed, tmp.path(), HELIX_SURROUND);
    feed_settled(&mut ed, "mr([");
    assert_eq!(text(&ed), "[hello]\n");
}
