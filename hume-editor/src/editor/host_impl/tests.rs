use std::path::Path;

use hume_engine::pipeline::BufferId;
use hume_scripting::host::{BufferHost, LanguageHost};

use crate::editor::Editor;
use crate::editor::scripting_setup::make_init_host;

use super::{line_start_offset, validate_offset, virtual_line_segments_to_bytes};

#[test]
fn close_buffer_errs_when_id_unknown() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::BufferText::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let mut host = make_init_host(&mut ed.state, &mut ed.view);
    // BufferId::default() is a zeroed key — not present in any live store.
    let err = host.close_buffer(BufferId::default()).unwrap_err();
    assert!(!err.is_empty(), "expected an error message");
}

#[test]
fn switch_to_buffer_noop_when_same() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::BufferText::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let bid = ed.focused_buffer_id();
    let mut host = make_init_host(&mut ed.state, &mut ed.view);
    // Switching to the same buffer should not error.
    host.switch_to_buffer(bid, bid).expect("same-buffer switch");
}

#[test]
fn attach_grammar_errs_for_bad_path() {
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::BufferText::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let mut host = make_init_host(&mut ed.state, &mut ed.view);
    let err = host
        .attach_grammar(
            "rust",
            Path::new("/no/such/lib.dylib"),
            "rust_language",
            Path::new("/no/such/highlights.scm"),
            None,
            None,
        )
        .unwrap_err();
    assert!(
        err.contains("register-grammar!"),
        "unexpected message: {err}"
    );
}

// ── `virtual_line_segments_to_bytes` — validation moved here from the Steel
// boundary (`VirtualLineSpec`'s segments are unvalidated char offsets; this
// function is the sole enforcement point); these replace the tests that used
// to live at `hume-scripting/src/builtins/decorations/tests.rs`.

fn seg(start: usize, end: usize, scope: &str) -> (usize, usize, String) {
    (start, end, scope.to_string())
}

#[test]
fn virtual_line_segments_to_bytes_sorts_by_start() {
    // ASCII text, so char and byte offsets coincide — isolates the sorting
    // behavior from the conversion.
    let out = virtual_line_segments_to_bytes("abcdef", vec![seg(4, 6, "b"), seg(0, 2, "a")])
        .expect("valid segments");
    assert_eq!(
        out,
        vec![seg(0, 2, "a"), seg(4, 6, "b")],
        "must come out sorted by start regardless of input order"
    );
}

#[test]
fn virtual_line_segments_to_bytes_rejects_start_ge_end() {
    let err = virtual_line_segments_to_bytes("abcdef", vec![seg(2, 2, "x")]).unwrap_err();
    assert!(err.contains("start < end"), "got: {err}");
}

#[test]
fn virtual_line_segments_to_bytes_rejects_end_past_char_length() {
    // "ab" is 2 chars; a segment ending at 5 is out of range.
    let err = virtual_line_segments_to_bytes("ab", vec![seg(0, 5, "x")]).unwrap_err();
    assert!(err.contains("char length"), "got: {err}");
}

#[test]
fn virtual_line_segments_to_bytes_rejects_overlap() {
    let err =
        virtual_line_segments_to_bytes("abcdef", vec![seg(0, 3, "a"), seg(2, 5, "b")]).unwrap_err();
    assert!(err.contains("overlap"), "got: {err}");
}

#[test]
fn virtual_line_segments_to_bytes_rejects_segment_splitting_a_grapheme_cluster() {
    // "e" (1 byte) + combining acute accent U+0301 (2 bytes) is 2 chars but
    // one grapheme cluster spanning bytes 0..3. Char offset 1 falls between
    // the two chars but not on the cluster boundary — the engine
    // (`rows.rs`'s `segment_virtual_row`) resolves scope once per cluster at
    // its start byte, so a segment edge here would silently mis-apply
    // instead of erroring under a mere char-boundary check.
    let err = virtual_line_segments_to_bytes("e\u{301}", vec![seg(0, 1, "x")]).unwrap_err();
    assert!(err.contains("grapheme-cluster boundary"), "got: {err}");
}

// ── `line_start_offset` / `validate_offset` — the position-contract fix
// (post-ship code review): a decoration position must address a real char,
// never the buffer's trailing phantom line or one-past-the-end.

#[test]
fn line_start_offset_accepts_the_last_content_line() {
    // "aaa\nbbb\nccc\n": line 2 ("ccc") is the last *content* line.
    let text = hume_editing::text::BufferText::from("aaa\nbbb\nccc\n");
    let pos = line_start_offset(&text, 2, "test").expect("last content line must be valid");
    assert_eq!(pos, 8, "line 2's line-start char offset");
}

#[test]
fn line_start_offset_rejects_the_trailing_phantom_line() {
    // Same fixture: line 3 is the empty line the trailing '\n' produces —
    // `RowMap::last_line()` (hume-engine/src/rows.rs) agrees line 2 is the
    // last renderable line, so line 3 has no row to decorate.
    let text = hume_editing::text::BufferText::from("aaa\nbbb\nccc\n");
    let err = line_start_offset(&text, 3, "test").unwrap_err();
    assert!(err.contains("out of range"), "got: {err}");
}

#[test]
fn validate_offset_accepts_the_last_real_char() {
    // "abc\n" — char offsets 0..=3 are real chars (the last being '\n'
    // itself); char offset 4 is one past the end. 'before' anchoring, so
    // the 'after'-only phantom-line check doesn't apply.
    let text = hume_editing::text::BufferText::from("abc\n");
    validate_offset(&text, 3, true, "test").expect("last real char must be valid");
}

#[test]
fn validate_offset_rejects_one_past_the_end() {
    // Pre-fix, this was deliberately admitted as "an 'after hint at
    // end-of-buffer" — but `visible_char_range` is half-open, so that
    // position could never pass its `contains` check: the hint would be
    // silently accepted and then silently never rendered. Must now error
    // loudly instead, same as every other out-of-range offset.
    let text = hume_editing::text::BufferText::from("abc\n");
    let err = validate_offset(&text, 4, true, "test").unwrap_err();
    assert!(err.contains("out of range"), "got: {err}");
}

#[test]
fn validate_offset_accepts_after_on_the_last_real_content_char() {
    // "abc\n" — an 'after hint on 'c' (offset 2) anchors at offset 3 (the
    // trailing '\n'), still on the last content line. Must not be confused
    // with the phantom-line case below.
    let text = hume_editing::text::BufferText::from("abc\n");
    validate_offset(&text, 2, false, "test").expect("'after on the last content char is valid");
}

#[test]
fn validate_offset_rejects_after_on_the_trailing_newline() {
    // "abc\n" — an 'after hint anchored on the trailing '\n' itself (offset
    // 3) would render at offset 4, the start of the buffer's trailing
    // phantom line — a position `RowMap::last_line()` never lays out, so
    // the hint would be silently accepted and then silently never render
    // (the same failure class `line_start_offset` already rejects for the
    // line-anchored kinds, reachable here through a char offset instead).
    let text = hume_editing::text::BufferText::from("abc\n");
    let err = validate_offset(&text, 3, false, "test").unwrap_err();
    assert!(err.contains("trailing"), "got: {err}");
}

#[test]
fn virtual_line_segments_to_bytes_converts_multibyte_char_offsets() {
    // "é" (2 bytes) + "→" (3 bytes) + "x" (1 byte), each its own grapheme
    // cluster. Char range (0, 2) covers "é→" — independently computed byte
    // width: 2 + 3 = 5, so the expected byte range is (0, 5), not (0, 2).
    let out = virtual_line_segments_to_bytes("é→x", vec![seg(0, 2, "prefix")])
        .expect("valid multi-byte segment");
    assert_eq!(
        out,
        vec![seg(0, 5, "prefix")],
        "char offsets (0, 2) over \"é→x\" must convert to byte offsets (0, 5)"
    );
}
