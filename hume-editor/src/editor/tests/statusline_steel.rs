// End-to-end Steel coverage for `set-statusline-text!` and the
// `steel:<name>` statusline element it feeds (`StatusElement::Custom`).

use super::*;
use crate::editor::message_log::Severity;
use crate::ui::statusline::{StatusElement, render_element};
use crate::ui::theme::EditorColors;
use hume_scripting::ScriptingHost;

fn custom_text(ed: &Editor, name: &str) -> String {
    let colors = EditorColors::default();
    let (text, _) = render_element(&StatusElement::Custom(name.into()), ed, &colors, "");
    text.into_owned()
}

#[test]
fn pushed_text_renders_for_the_focused_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-statusline-text! "greeting" (current-buffer) "hello")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    assert_eq!(custom_text(&ed, "greeting"), "hello");
}

#[test]
fn a_name_never_pushed_renders_empty() {
    let ed = editor_from("-[a]>\n");
    assert_eq!(custom_text(&ed, "never-pushed"), "");
}

#[test]
fn pushed_text_is_not_shown_once_a_different_buffer_is_focused() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "x\n").unwrap();
    let other_str = other.to_string_lossy().replace('\\', "/");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "arm" "" (lambda ()
                 (set-statusline-text! "greeting" (current-buffer) "hello")
                 (switch-to-buffer! (open-buffer! "{other_str}"))))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    // Focus moved to the second buffer, which never had anything pushed —
    // the first buffer's "hello" must not leak across the switch.
    assert_eq!(custom_text(&ed, "greeting"), "");
}

#[test]
fn closing_the_buffer_clears_its_pushed_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    let scratch = tmp.path().join("scratch.txt");
    std::fs::write(&scratch, "x\n").unwrap();
    let scratch_str = scratch.to_string_lossy().replace('\\', "/");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "arm" "" (lambda ()
                 (define b (open-buffer! "{scratch_str}"))
                 (set-statusline-text! "greeting" b "hello")
                 (close-buffer! b)))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    assert!(
        ed.state.config.statusline_text.is_empty(),
        "a closed buffer's pushed text must not leak: {:?}",
        ed.state.config.statusline_text
    );
}

#[test]
fn empty_text_clears_a_previously_pushed_value() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-statusline-text! "greeting" (current-buffer) "hello")
             (set-statusline-text! "greeting" (current-buffer) "")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    assert_eq!(custom_text(&ed, "greeting"), "");
    assert!(
        ed.state.config.statusline_text.is_empty(),
        "clearing the only pushed name for a buffer must drop its now-empty \
         entry too, not leave an empty map behind"
    );
}

#[test]
fn set_statusline_text_on_a_stale_bid_raises_unknown_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    let scratch = tmp.path().join("scratch.txt");
    std::fs::write(&scratch, "x\n").unwrap();
    let scratch_str = scratch.to_string_lossy().replace('\\', "/");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "arm" "" (lambda ()
                 (define b (open-buffer! "{scratch_str}"))
                 (close-buffer! b)
                 (set-statusline-text! "greeting" b "hello")))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains("set-statusline-text!")
            && e.text.contains("unknown buffer")),
        "a stale bid must surface as a Steel error naming the builtin, got: {entries:?}"
    );
}
