/// Property-based fuzz test for the full `Editor` key-handling pipeline.
///
/// Feeds random sequences of plausible key events to `Editor::handle_key` and
/// asserts that no sequence ever panics or leaves the editor in an invalid state.
///
/// This complements the `proptest_doc` tests (which target `Text` and pure
/// ops) by exercising the whole editor: mode transitions, minibuffer, search,
/// select-within, undo/redo, and multi-cursor, all interacting.
#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use proptest::prelude::*;

    use crate::editor::buffer::Buffer;
    use crate::editor::Editor;
    use crate::testing::parse_state;

    // ── Invariant checker ─────────────────────────────────────────────────────

    fn assert_editor_invariants(ed: &Editor) {
        let buf = ed.doc().text();
        let sels = ed.current_selections();

        // Text always ends with structural '\n'.
        assert!(
            buf.to_string().ends_with('\n'),
            "buffer must end with \\n, got: {:?}",
            buf.to_string()
        );

        // SelectionSet is non-empty.
        assert!(sels.len() > 0, "selection set must not be empty");

        // All selection positions are within the buffer.
        let len = buf.len_chars();
        for sel in sels.iter_sorted() {
            assert!(
                sel.head < len,
                "selection head {} out of bounds (buf len {})",
                sel.head,
                len
            );
            assert!(
                sel.anchor < len,
                "selection anchor {} out of bounds (buf len {})",
                sel.anchor,
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
                FuzzKey::Char(ch) => KeyEvent::new(KeyCode::Char(*ch), KeyModifiers::NONE),
                FuzzKey::Esc => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                FuzzKey::Enter => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                FuzzKey::Backspace => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
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
            // Punctuation / symbols — exercises text objects, search patterns
            2 => prop_oneof![
                Just('('), Just(')'), Just('['), Just(']'),
                Just('{'), Just('}'), Just('"'), Just('\''),
                Just(' '), Just('.'), Just('/'), Just('?'),
                Just('*'), Just('%'), Just(':'),
            ].prop_map(FuzzKey::Char),
            // Digits — numeric prefixes (e.g. `3w`, `5j`)
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
    /// a stable base — the interesting behaviour is in the key sequences.
    fn arb_initial_editor() -> impl Strategy<Value = Editor> {
        prop_oneof![
            Just("-[h]>ello world\n"),
            Just("-[f]>oo\nbar\nbaz\n"),
            Just("-[a]>bcde\nfghij\n"),
            Just("-[x]>\n"),                    // single-char buffer
            Just("-[a]>a bb cc aa bb cc\n"),    // repeated words for search
        ]
        .prop_map(|s| {
            let (buf, sels) = parse_state(s);
            Editor::for_testing(Buffer::new(buf, sels))
        })
    }

    // ── Property tests ────────────────────────────────────────────────────────

    proptest! {
        /// Feeding any sequence of plausible keys to the editor must never
        /// panic and must leave the buffer and selections in a valid state.
        ///
        /// Invariants checked after every key:
        /// - Text always ends with `\n`.
        /// - SelectionSet is non-empty and all positions are in-bounds.
        #[test]
        fn prop_random_keys_never_panic(
            mut ed in arb_initial_editor(),
            keys in arb_key_sequence(60),
        ) {
            for key in &keys {
                ed.handle_key(key.to_key_event());
                assert_editor_invariants(&ed);
            }
        }

        /// The same property with longer sequences, to exercise multi-step
        /// interactions like search → confirm → n → select-within → undo.
        #[test]
        fn prop_long_key_sequence_never_panic(
            mut ed in arb_initial_editor(),
            keys in arb_key_sequence(200),
        ) {
            for key in &keys {
                ed.handle_key(key.to_key_event());
            }
            // Check invariants only at the end for speed — panics during the
            // loop are still caught by proptest as failures.
            assert_editor_invariants(&ed);
        }
    }
}
