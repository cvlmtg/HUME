//! Property-based fuzz test for the full `Editor` key-handling pipeline.
//!
//! Feeds random sequences of plausible key events to `Editor::handle_input`,
//! settling after each one (mirroring `Editor::run`'s loop), and asserts that
//! no sequence ever panics or leaves the editor in an invalid state.
//!
//! This complements the `proptest_doc` tests (which target `BufferText` and pure
//! ops) by exercising the whole editor: mode transitions, minibuffer, search,
//! sift-within, undo/redo, and multi-cursor, all interacting.
#[cfg(test)]
mod tests {
    use hume_editing::selection::EditView;
    use proptest::prelude::*;
    use termina::event::{Event as TerminalEvent, KeyCode, KeyEvent, Modifiers};

    use crate::editor::Editor;
    use test_fixtures::testing::parse_state;

    // ── Invariant checker ─────────────────────────────────────────────────────

    fn assert_editor_invariants(ed: &Editor) {
        let text = ed.doc().text();
        let sels = EditView::bind(text, ed.current_selections());
        sels.check().expect("the selections fit the text");

        // Text always ends with structural '\n'.
        assert!(
            text.to_string().ends_with('\n'),
            "buffer must end with \\n, got: {:?}",
            text.to_string()
        );

        // SelectionSet is non-empty.
        assert!(sels.len() > 0, "selection set must not be empty");

        // Every anchor and head is a grapheme-cluster start.
        for sel in sels.iter() {
            assert!(
                text.snap(sel.anchor().offset()) == sel.anchor()
                    && text.snap(sel.head().offset()) == sel.head(),
                "selection ({:?}, {:?}) splits a grapheme cluster of {:?}",
                sel.anchor().offset(),
                sel.head().offset(),
                text.to_string()
            );
        }

        // All selection positions are within the buffer.
        let len = text.end();
        for sel in sels.iter() {
            assert!(
                sel.head().offset() < len,
                "selection head {:?} out of bounds (buf len {:?})",
                sel.head().offset(),
                len
            );
            assert!(
                sel.anchor().offset() < len,
                "selection anchor {:?} out of bounds (buf len {:?})",
                sel.anchor().offset(),
                len
            );
        }
    }

    // ── Key strategy ─────────────────────────────────────────────────────────

    /// A `FuzzKey` is a compact representation of a key event that proptest
    /// can generate and shrink. Using an enum (rather than raw `KeyEvent`)
    /// lets proptest shrink toward simpler keys when a failure is found.
    #[derive(Debug, Clone)]
    enum FuzzKey {
        Char(char),
        Esc,
        Enter,
        Backspace,
    }

    impl FuzzKey {
        fn to_key_event(&self) -> KeyEvent {
            match self {
                FuzzKey::Char(ch) => KeyEvent::new(KeyCode::Char(*ch), Modifiers::NONE),
                FuzzKey::Esc => KeyEvent::new(KeyCode::Escape, Modifiers::NONE),
                FuzzKey::Enter => KeyEvent::new(KeyCode::Enter, Modifiers::NONE),
                FuzzKey::Backspace => KeyEvent::new(KeyCode::Backspace, Modifiers::NONE),
            }
        }
    }

    /// Generate a single fuzz key from a weighted alphabet.
    ///
    /// Weights are tuned so the editor spends time in all modes:
    /// - High weight on printable chars so Insert mode gets real input.
    /// - Moderate weight on common Normal-mode commands.
    /// - Lower weight on Esc/Enter so modeframes aren't immediately closed.
    fn arb_fuzz_key() -> impl Strategy<Value = FuzzKey> {
        prop_oneof![
            // ── Printable chars (Insert mode content + search/command input) ──
            // Letters
            8 => prop_oneof![
                Just('a'), Just('b'), Just('c'), Just('d'), Just('e'),
                Just('f'), Just('g'), Just('h'), Just('i'), Just('j'),
                Just('k'), Just('l'), Just('m'), Just('n'), Just('o'),
                Just('p'), Just('r'), Just('s'), Just('u'), Just('w'),
                Just('x'), Just('y'), Just('z'),
            ].prop_map(FuzzKey::Char),
            // Punctuation / symbols: exercises text objects, search patterns
            2 => prop_oneof![
                Just('('), Just(')'), Just('['), Just(']'),
                Just('{'), Just('}'), Just('"'), Just('\''),
                Just(' '), Just('.'), Just('/'), Just('?'),
                Just('*'), Just('%'), Just(':'),
            ].prop_map(FuzzKey::Char),
            // Chars that join or extend a cluster, or are wide: combining
            // mark, ZWJ, variation selector, regional indicators, é, CJK,
            // emoji, NBSP.
            2 => prop_oneof![
                Just('\u{301}'), Just('\u{200d}'), Just('\u{fe0f}'),
                Just('\u{1f1ee}'), Just('\u{1f1f9}'), Just('\u{e9}'),
                Just('\u{6f22}'), Just('\u{1f600}'), Just('\u{a0}'),
            ].prop_map(FuzzKey::Char),
            // Digits: numeric prefixes (e.g. `3w`, `5j`)
            1 => prop_oneof![
                Just('1'), Just('2'), Just('3'), Just('4'), Just('5'),
            ].prop_map(FuzzKey::Char),
            // ── Control keys ─────────────────────────────────────────────────
            3 => Just(FuzzKey::Esc),
            2 => Just(FuzzKey::Enter),
            1 => Just(FuzzKey::Backspace),
        ]
    }

    fn arb_key_sequence(max_len: usize) -> impl Strategy<Value = Vec<FuzzKey>> {
        proptest::collection::vec(arb_fuzz_key(), 1..=max_len)
    }

    // ── Initial state strategy ────────────────────────────────────────────────

    /// A small set of realistic starting documents for the fuzzer.
    ///
    /// Using fixed documents (rather than fully random ones) gives the fuzzer
    /// a stable base; the interesting behaviour is in the key sequences.
    fn arb_initial_editor() -> impl Strategy<Value = Editor> {
        prop_oneof![
            Just("-[h]>ello world\n"),
            Just("-[f]>oo\nbar\nbaz\n"),
            Just("-[a]>bcde\nfghij\n"),
            Just("-[x]>\n"),                   // single-char buffer
            Just("-[a]>a bb cc aa bb cc\n"),   // repeated words for search
            Just("-[e\u{301}]>x y\u{301}z\n"), // combining marks
            Just("\u{1f1ee}\u{1f1f9}\u{1f1eb}\u{1f1f7} -[a]>\n"), // flag pairs
            Just("-[\u{6f22}]>\u{5b57} \u{1f468}\u{200d}\u{1f469}\n"), // CJK and ZWJ
        ]
        .prop_map(|s| {
            let (text, sels) = parse_state(s);
            Editor::for_testing_with(test_fixtures::testing::state(text, sels))
        })
    }

    // ── Property tests ────────────────────────────────────────────────────────

    proptest! {
        /// Feeding any sequence of plausible keys to the editor must never
        /// panic and must leave the buffer and selections in a valid state.
        ///
        /// Invariants checked after every key:
        /// - BufferText always ends with `\n`.
        /// - SelectionSet is non-empty and all positions are in-bounds.
        #[test]
        fn prop_random_keys_never_panic(
            mut ed in arb_initial_editor(),
            keys in arb_key_sequence(60),
        ) {
            for key in &keys {
                ed.handle_input(TerminalEvent::Key(key.to_key_event()));
                ed.settle();
                assert_editor_invariants(&ed);
            }
        }

        /// The same property with longer sequences, to exercise multi-step
        /// interactions like search → confirm → n → sift-within → undo.
        #[test]
        fn prop_long_key_sequence_never_panic(
            mut ed in arb_initial_editor(),
            keys in arb_key_sequence(200),
        ) {
            for key in &keys {
                ed.handle_input(TerminalEvent::Key(key.to_key_event()));
                ed.settle();
            }
            // Check invariants only at the end for speed. Panics during the
            // loop are still caught by proptest as failures.
            assert_editor_invariants(&ed);
        }
    }
}
