use std::path::Path;

use hume_core::{BufferId, OnUnsaved, Side};
use hume_scripting::host::{BufferHost, LanguageHost};

use crate::editor::Editor;
use crate::editor::tests::{co, init_host};

use super::decorations::validate_offset;

#[test]
fn close_buffer_errs_when_id_unknown() {
    let mut ed = Editor::for_testing(hume_editing::text::BufferText::empty());
    let mut host = init_host!(ed);
    // BufferId::default() is a zeroed key, not present in any live store.
    let err = host
        .close_buffer(BufferId::default(), OnUnsaved::Refuse)
        .unwrap_err();
    assert!(!err.is_empty(), "expected an error message");
}

#[test]
fn switch_to_buffer_noop_when_same() {
    let mut ed = Editor::for_testing(hume_editing::text::BufferText::empty());
    let bid = ed.focused_buffer_id();
    let pane = hume_core::PaneHandle::with_pane(bid, ed.view.focused_pane());
    let mut host = init_host!(ed);
    // Switching to the same buffer should not error.
    host.switch_to_buffer(hume_scripting::host::ShownHandle::for_test(pane), bid)
        .expect("same-buffer switch");
}

#[test]
fn attach_grammar_errs_for_bad_path() {
    let mut ed = Editor::for_testing(hume_editing::text::BufferText::empty());
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

// ── `validate_offset`: a decoration position must address a real char,
// never the buffer's trailing phantom line or one-past-the-end.

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
    // (the same failure class the line-anchored kinds' line check rejects,
    // reachable here through a char offset instead).
    let text = hume_editing::text::BufferText::from("abc\n");
    let err = validate_offset(&text, 3, Side::After).unwrap_err();
    assert!(err.contains("trailing"), "got: {err}");
}

// ── `set_conceals` ───────────────────────────────────────────────────────────

mod conceals {
    use hume_decorations::{ConcealEntry, ConcealEntrySpan};
    use hume_editing::text::BufferText;
    use hume_scripting::host::{DecorationHost, SetError};
    use hume_scripting::{ConcealSpec, ConcealSpecSpan, ConcealSpecText};

    use crate::editor::Editor;
    use crate::editor::tests::init_host;

    const SOURCE: &str = "src";

    fn editor(text: &str) -> Editor {
        Editor::for_testing(BufferText::from(text))
    }

    fn set(ed: &mut Editor, specs: Vec<ConcealSpec>) -> Result<(), SetError> {
        let bid = ed.focused_buffer_id();
        let mut host = init_host!(ed);
        host.set_conceals(SOURCE.to_owned(), bid, specs)
    }

    fn entries(ed: &Editor) -> &[ConcealEntry] {
        ed.state
            .config
            .decorations
            .conceals_for(SOURCE, ed.focused_buffer_id())
    }

    /// Each stored entry's char range; an inserted one is empty at its `at`.
    fn stored(ed: &Editor) -> Vec<(usize, usize)> {
        entries(ed)
            .iter()
            .map(|entry| (entry.chars().start.index(), entry.chars().end.index()))
            .collect()
    }

    fn hidden(start: usize, end: usize) -> ConcealSpec {
        ConcealSpec {
            span: ConcealSpecSpan::Hidden {
                start,
                end,
                text: None,
            },
            reveal: None,
        }
    }

    fn inserted(at: usize) -> ConcealSpec {
        ConcealSpec {
            span: ConcealSpecSpan::Inserted {
                at,
                text: ConcealSpecText {
                    text: "┃ ".into(),
                    scope: None,
                },
            },
            reveal: None,
        }
    }

    fn refusal(result: Result<(), SetError>, index: usize) -> String {
        match result.unwrap_err() {
            SetError::Entry { index: i, message } if i == index => message,
            other => panic!("expected a refusal naming entry {index}, got {other:?}"),
        }
    }

    #[test]
    fn set_conceals_rejects_a_range_crossing_a_line_break() {
        let mut ed = editor("ab\ncd\n");
        let message = refusal(set(&mut ed, vec![hidden(1, 4)]), 0);
        assert!(message.contains("one line"), "got: {message}");
        assert!(stored(&ed).is_empty());
    }

    #[test]
    fn set_conceals_rejects_a_range_covering_the_newline() {
        let mut ed = editor("abc\n");
        let message = refusal(set(&mut ed, vec![hidden(1, 4)]), 0);
        assert!(message.contains("one line"), "got: {message}");
        assert!(stored(&ed).is_empty());
    }

    #[test]
    fn set_conceals_rejects_an_empty_hidden_range() {
        let mut ed = editor("abc\n");
        let message = refusal(set(&mut ed, vec![hidden(2, 2)]), 0);
        assert!(message.contains("empty"), "got: {message}");
        assert!(stored(&ed).is_empty());
    }

    #[test]
    fn set_conceals_rejects_an_empty_range_past_the_last_line_break() {
        let mut ed = editor("abc\n");
        let message = refusal(set(&mut ed, vec![inserted(4)]), 0);
        assert!(
            message.contains("past the last line break"),
            "got: {message}"
        );
        assert!(stored(&ed).is_empty());
    }

    #[test]
    fn set_conceals_rejects_overlapping_entries_naming_the_second() {
        let mut ed = editor("abcdef\n");
        let message = refusal(set(&mut ed, vec![hidden(0, 3), hidden(2, 4)]), 1);
        assert!(message.contains("overlaps entry 0"), "got: {message}");
        assert!(stored(&ed).is_empty());
    }

    #[test]
    fn set_conceals_widens_to_clusters() {
        // "e" + U+0301 is chars 0..2, one cluster; "x" is char 2.
        let mut ed = editor("e\u{301}x\n");
        set(&mut ed, vec![hidden(1, 2)]).unwrap();
        assert_eq!(stored(&ed), [(0, 2)]);
    }

    #[test]
    fn set_conceals_snaps_an_inserted_conceal_and_allows_the_newline() {
        let mut ed = editor("e\u{301}x\n");
        set(&mut ed, vec![inserted(3), inserted(1)]).unwrap();
        assert_eq!(stored(&ed), [(0, 0), (3, 3)]);
    }

    #[test]
    fn set_conceals_interns_the_replacement_scope() {
        let mut ed = editor("abc\n");
        let mut spec = hidden(0, 2);
        spec.span = ConcealSpecSpan::Hidden {
            start: 0,
            end: 2,
            text: Some(ConcealSpecText {
                text: "H1 ".into(),
                scope: Some("markup.heading.1".into()),
            }),
        };
        set(&mut ed, vec![spec]).unwrap();
        let expected = ed.view.registry.intern("markup.heading.1");
        let [entry] = entries(&ed) else {
            panic!("one entry")
        };
        let ConcealEntrySpan::Hidden {
            text: Some(shown), ..
        } = &entry.span
        else {
            panic!("a hidden entry with text")
        };
        assert_eq!(&*shown.text, "H1 ");
        assert_eq!(shown.scope, Some(expected));
    }

    #[test]
    fn set_conceals_replaces_wholesale_and_bumps_the_generation() {
        let mut ed = editor("abcdef\n");
        let bid = ed.focused_buffer_id();
        let generation = |ed: &Editor| ed.state.config.decorations.generation(bid);
        let start = generation(&ed);
        set(&mut ed, vec![hidden(0, 1), hidden(2, 3)]).unwrap();
        let first = generation(&ed);
        assert!(first > start);
        set(&mut ed, vec![hidden(4, 5)]).unwrap();
        assert_eq!(stored(&ed), [(4, 5)]);
        assert!(generation(&ed) > first);
    }
}
