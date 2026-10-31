use std::path::Path;

use hume_core::{BufferId, OnUnsaved, Side};
use hume_scripting::host::{BufferHost, LanguageHost};

use crate::editor::Editor;
use crate::editor::tests::{co, init_host};

use super::decorations::{content_line, validate_offset};

#[test]
fn close_buffer_errs_when_id_unknown() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::at_start(
        hume_editing::text::BufferText::empty(),
    ));
    let mut host = init_host!(ed);
    // BufferId::default() is a zeroed key, not present in any live store.
    let err = host
        .close_buffer(BufferId::default(), OnUnsaved::Refuse)
        .unwrap_err();
    assert!(!err.is_empty(), "expected an error message");
}

#[test]
fn switch_to_buffer_noop_when_same() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::at_start(
        hume_editing::text::BufferText::empty(),
    ));
    let bid = ed.focused_buffer_id();
    let pane = hume_core::PaneHandle::with_pane(bid, ed.view.focused_pane());
    let mut host = init_host!(ed);
    // Switching to the same buffer should not error.
    host.switch_to_buffer(hume_scripting::host::ShownHandle::for_test(pane), bid)
        .expect("same-buffer switch");
}

#[test]
fn attach_grammar_errs_for_bad_path() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::at_start(
        hume_editing::text::BufferText::empty(),
    ));
    let mut host = init_host!(ed);
    let err = host
        .attach_grammar(&hume_scripting::GrammarReg {
            name: "rust".into(),
            source: hume_core::GrammarSource {
                path: Path::new("/no/such/lib.dylib").to_path_buf(),
                symbol: "rust_language".into(),
                queries: hume_core::QueryFiles::new(
                    Path::new("/no/such/highlights.scm").to_path_buf(),
                ),
            },
            origin: hume_core::ConfigOrigin::User,
        })
        .unwrap_err();
    assert!(
        err.contains("'rust': grammar load failed"),
        "unexpected message: {err}"
    );
}

#[test]
fn validate_offset_snaps_inside_a_combining_sequence() {
    // "e" + U+0301 is chars 0..2, one cluster; "x" is char 2.
    let text = hume_editing::text::BufferText::from("e\u{301}x\n");
    assert_eq!(
        validate_offset(&text, 1, Side::Before).unwrap(),
        co(0),
        "'before inside the character lands on its start"
    );
    assert_eq!(
        validate_offset(&text, 0, Side::After).unwrap(),
        co(1),
        "'after inside the character lands on its last char, so the hint follows the accent"
    );
}

#[test]
fn validate_offset_snaps_inside_a_zwj_sequence() {
    // Man + ZWJ + woman: chars 0..3, one cluster; "x" is char 3.
    let text = hume_editing::text::BufferText::from("\u{1F468}\u{200D}\u{1F469}x\n");
    assert_eq!(validate_offset(&text, 2, Side::Before).unwrap(), co(0));
    assert_eq!(validate_offset(&text, 0, Side::After).unwrap(), co(2));
}

// ── `content_line` / `validate_offset`: a decoration position must address
// a real line or char, never the buffer's trailing phantom line or
// one-past-the-end.

#[test]
fn content_line_accepts_the_last_content_line() {
    // "aaa\nbbb\nccc\n": line 2 ("ccc") is the last *content* line.
    let text = hume_editing::text::BufferText::from("aaa\nbbb\nccc\n");
    let line = content_line(&text, 2).expect("last content line must be valid");
    assert_eq!(line, hume_rope::line::ContentLine::new(2));
}

#[test]
fn content_line_rejects_the_trailing_phantom_line() {
    // Same fixture: line 3 is the empty line the trailing '\n' produces.
    // `DisplayLineMap::last_line()` (hume-engine/src/display_lines.rs) agrees line 2 is the
    // last renderable line, so line 3 has no row to decorate.
    let text = hume_editing::text::BufferText::from("aaa\nbbb\nccc\n");
    let err = content_line(&text, 3).unwrap_err();
    assert!(err.contains("out of range"), "got: {err}");
}

#[test]
fn validate_offset_accepts_the_last_real_char() {
    // "abc\n": char offsets 0..=3 are real chars (the last being '\n'
    // itself); char offset 4 is one past the end. 'before' anchoring, so
    // the 'after'-only phantom-line check doesn't apply.
    let text = hume_editing::text::BufferText::from("abc\n");
    validate_offset(&text, 3, Side::Before).expect("last real char must be valid");
}

#[test]
fn validate_offset_rejects_one_past_the_end() {
    // `visible_char_range` is half-open, so an 'after' hint at end-of-buffer
    // could never pass its `contains` check: the hint would be silently
    // accepted and then silently never rendered. It must error loudly
    // instead, same as every other out-of-range offset.
    let text = hume_editing::text::BufferText::from("abc\n");
    let err = validate_offset(&text, 4, Side::Before).unwrap_err();
    assert!(err.contains("out of range"), "got: {err}");
}

#[test]
fn validate_offset_accepts_after_on_the_last_real_content_char() {
    // "abc\n": an 'after hint on 'c' (offset 2) anchors at offset 3 (the
    // trailing '\n'), still on the last content line. Must not be confused
    // with the phantom-line case below.
    let text = hume_editing::text::BufferText::from("abc\n");
    validate_offset(&text, 2, Side::After).expect("'after on the last content char is valid");
}

#[test]
fn validate_offset_rejects_after_on_the_trailing_newline() {
    // "abc\n": an 'after hint anchored on the trailing '\n' itself (offset
    // 3) would render at offset 4, the start of the buffer's trailing
    // phantom line, a position `DisplayLineMap::last_line()` never lays out, so
    // the hint would be silently accepted and then silently never render
    // (the same failure class `content_line` already rejects for the
    // line-anchored kinds, reachable here through a char offset instead).
    let text = hume_editing::text::BufferText::from("abc\n");
    let err = validate_offset(&text, 3, Side::After).unwrap_err();
    assert!(err.contains("trailing"), "got: {err}");
}
