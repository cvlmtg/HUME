use super::*;
use pretty_assertions::assert_eq;

// ── word-chars (configurable extra word characters) ────────────────────────
//
// Full-dispatch coverage of the setting: the ops-level tests in
// hume-ops/src/motion/tests/word.rs and hume-ops/src/text_object/tests/word.rs
// cover the span math; these confirm the setting actually reaches behavior
// through the real keymap/registry/dispatch path (:set, direct field write,
// insert-mode Ctrl-W, `*`, symbol-under-cursor, and replay).

#[test]
fn w_follows_buffer_word_chars() {
    // Exercises the typed-command path (:set), not just a direct field write.
    let mut ed = editor_from("-[f]>oo-bar baz\n");
    type_cmd(&mut ed, ":set buffer word-chars=-");
    ed.feed_key(key('w'));
    // Default `word-selects-whitespace`: `w` also covers the destination
    // word's leading space.
    assert_eq!(state(&ed), "foo-bar-[ baz]>\n");
}

#[test]
fn w_follows_global_word_chars() {
    let mut ed = editor_from("-[f]>oo-bar baz\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo-bar-[ baz]>\n");
}

#[test]
fn mm_selects_whole_hyphenated_word() {
    let mut ed = editor_from("-[f]>oo-bar baz\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_keys([key('m'), key('m')]);
    // First word on the line: no leading whitespace to absorb, so the
    // default `word-selects-whitespace` falls back to the trailing space.
    assert_eq!(state(&ed), "-[foo-bar ]>baz\n");
}

#[test]
fn miw_selects_whole_hyphenated_word() {
    let mut ed = editor_from("foo-b-[a]>r\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_keys([key('m'), key('i'), key('w')]);
    assert_eq!(state(&ed), "-[foo-bar]>\n");
}

#[test]
fn ctrl_w_deletes_whole_hyphenated_word() {
    let mut ed = editor_from("-[\n]>");
    ed.state.settings.word_chars = "-".into();
    ed.feed_key(key('i'));
    for ch in "foo-bar".chars() {
        ed.feed_key(key(ch));
    }
    assert_eq!(ed.doc().text().to_string(), "foo-bar\n");
    ed.feed_key(key_ctrl('w'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "Ctrl-W must delete the whole hyphenated run, not just \"bar\""
    );
}

#[test]
fn star_anchors_hyphenated_word_on_both_sides() {
    let mut ed = editor_from("-[f]>oo-bar baz\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_key(key('*'));
    assert_eq!(ed.state.registers.search_register(), Some(r"\bfoo-bar\b"));
}

#[test]
fn star_omits_leading_anchor_for_leading_hyphen() {
    // "--foo": the run's leading edge is '-', not a built-in word char, so
    // rust-regex's own `\b` can never match there — no leading anchor.
    let mut ed = editor_from("-[-]>-foo bar\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_key(key('*'));
    assert_eq!(ed.state.registers.search_register(), Some(r"--foo\b"));
}

#[test]
fn star_with_no_word_chars_is_unchanged() {
    let mut ed = editor_from("-[f]>oo bar\n");
    ed.feed_key(key('*'));
    assert_eq!(ed.state.registers.search_register(), Some(r"\bfoo\b"));
}

#[test]
fn symbol_under_cursor_returns_whole_hyphenated_word() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo-b-[a]>r baz\n");
    ed.state.settings.word_chars = "-".into();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "check" "" (lambda ()
             (log! 'info (symbol-under-cursor (current-buffer)))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "foo-bar");
}

#[test]
fn select_word_nearest_on_line_follows_word_chars() {
    let mut ed = editor_from("-[f]>oo-bar baz\n");
    ed.state.settings.word_chars = "-".into();
    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );
    assert_eq!(state(&ed), "-[foo-bar ]>baz\n");
}

/// The test above already exercises the wrap branch of
/// `cmd_visual_select_word_nearest_on_line` — wrap is on by default
/// (`wrap-mode = indent`, width 0). What it doesn't reach is a buffer line
/// that actually splits into two visual sub-rows, where the wrap-aware path
/// hands `nearest_word_on_line` a `line_start` bounded to the cursor's own
/// sub-row rather than the whole buffer line.
///
/// Wrapping at column 12 puts "hello world " on row 0 and "foo-bar" on row 1.
/// Fail oracle, both halves at once: dropping `word-chars` from the wrap
/// branch selects "bar" alone; losing the sub-row bound absorbs row 0's
/// trailing space.
#[test]
fn select_word_nearest_on_line_follows_word_chars_across_a_wrapped_row() {
    let mut ed = editor_from("hello world foo-b-[a]>r\n");
    ed.state.settings.word_chars = "-".into();
    ed.view.panes[ed.state.focused_pane_id].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 12 }),
        saved: None,
    });
    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );
    assert_eq!(state(&ed), "hello world -[foo-bar]>\n");
}

#[test]
fn invalid_word_chars_is_rejected() {
    let result = crate::editor::commands::typed_set(
        &mut editor_from("-[a]>b\n"),
        Some("global word-chars=- "),
        false,
    );
    assert!(result.is_err(), "a space in word-chars must be rejected");
}

/// Mirrors `word_motion_settings::dot_repeat_of_mm_delete_reresolves_word_selects_whitespace`:
/// `.` must re-resolve `word-chars` fresh from settings at replay time, not
/// bake in whatever was configured when the original `mm` first ran.
#[test]
fn dot_repeat_reresolves_word_chars() {
    let mut ed = editor_from("-[f]>oo-bar baz-qux\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_keys([key('m'), key('m')]); // selects "foo-bar " (whole run + trailing space)
    ed.feed_key(key('d')); // delete -> "baz-qux\n"
    assert_eq!(ed.doc().text().to_string(), "baz-qux\n");

    ed.state.settings.word_chars = String::new();
    ed.feed_key(key('.')); // replay: re-establishes via mm (now without word-chars), then deletes

    assert_eq!(
        ed.doc().text().to_string(),
        "-qux\n",
        "replay must re-resolve word-chars, selecting bare \"baz\" (no trailing \
         whitespace to absorb, since '-' is no longer a word char) rather than \
         reusing the whole \"baz-qux\" run from the first press"
    );
}
