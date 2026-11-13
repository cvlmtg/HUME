//! `on-syntax-changed`.

use super::*;
use hume_core::Severity;

const FIRED: &str = "syntax changed";

/// A `rust` buffer whose first parse has not landed, with a hook that logs
/// [`FIRED`] each time `on-syntax-changed` runs.
fn rust_editor_with_hook(tmp: &std::path::Path) -> Editor {
    let mut ed =
        structural::grammar_editor_undrained("rust", "tree_sitter_rust", &[], "-[f]>n a() {}\n");
    run(
        &mut ed,
        tmp,
        &format!(r#"(register-hook! 'on-syntax-changed (lambda (pane) (log! 'trace "{FIRED}")))"#),
    );
    ed
}

/// Settle enough times for a posted parse to land, its event to be queued,
/// and the event to drain.
fn settle_through_the_parse(ed: &mut Editor) {
    for _ in 0..4 {
        ed.settle();
    }
}

fn fired(ed: &Editor) -> usize {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == FIRED)
        .count()
}

#[test]
fn fires_once_after_the_first_parse_with_no_input_between() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = rust_editor_with_hook(tmp.path());
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 1);
}

#[test]
fn fires_once_per_reparse() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = rust_editor_with_hook(tmp.path());
    settle_through_the_parse(&mut ed);
    type_chars(&mut ed, "ix");
    ed.feed_key(key_esc());
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 2);
}

#[test]
fn does_not_fire_without_a_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = rust_editor_with_hook(tmp.path());
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 1);
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 1);
}

#[test]
fn fires_once_when_the_buffer_loses_its_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = rust_editor_with_hook(tmp.path());
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 1);
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, None).expect_open();
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 2);
    settle_through_the_parse(&mut ed);
    assert_eq!(fired(&ed), 2);
}
