/// Property-based tests for Document-level invariants.
///
/// These tests complement the unit tests in individual modules and the
/// ChangeSet-level proptests in `changeset.rs`. They verify that:
///
/// 1. Any sequence of edit operations + undo/redo never corrupts the buffer
///    or desynchronises the selection set.
/// 2. Any sequence of pure operations (motions, text objects, selection
///    commands) never violates the buffer or selection invariants.
/// 3. Specific undo/redo properties hold (undo reverses an edit, undo+redo
///    is identity, N edits then N undos restores the original state).
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::buffer::Buffer;
    use crate::document::Document;
    use crate::edit::{
        delete_char_backward, delete_char_forward, delete_selection, insert_char,
    };
    use crate::motion::{
        cmd_goto_line_end, cmd_goto_line_start, cmd_move_down, cmd_move_left, cmd_move_right,
        cmd_move_up, cmd_select_next_WORD, cmd_select_next_word, cmd_select_prev_WORD,
        cmd_select_prev_word,
    };
    use crate::selection::{Selection, SelectionSet};
    use crate::selection_cmd::{
        cmd_collapse_selection, cmd_cycle_primary_backward, cmd_cycle_primary_forward,
        cmd_flip_selections, cmd_keep_primary_selection,
    };
    use crate::text_object::{cmd_around_word, cmd_inner_line, cmd_inner_word};

    // ── Invariant checker ─────────────────────────────────────────────────────

    /// Assert all buffer and selection invariants after any operation.
    ///
    /// Called after every operation in every proptest. A panic here means the
    /// code under test produced an invalid state.
    fn assert_invariants(buf: &Buffer, sels: &SelectionSet) {
        // Buffer invariant 1: always ends with structural '\n'.
        assert!(
            buf.to_string().ends_with('\n'),
            "buffer must end with \\n, got: {:?}",
            buf.to_string()
        );

        // Buffer invariant 2: len_chars > 0 (at minimum the structural '\n').
        let len = buf.len_chars();
        assert!(len > 0, "buffer must have at least 1 char");

        // SelectionSet invariant 1: never empty.
        assert!(sels.len() > 0, "selection set must not be empty");

        // SelectionSet invariant 2: all positions strictly within the buffer.
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

        // SelectionSet invariant 3: sorted ascending by start().
        let starts: Vec<usize> = sels.iter_sorted().map(|s| s.start()).collect();
        for w in starts.windows(2) {
            assert!(
                w[0] <= w[1],
                "selections not sorted: start {} > start {}",
                w[0],
                w[1]
            );
        }

        // SelectionSet invariant 4: no overlapping or adjacent selections.
        // Adjacent means one ends where the next begins — both are merged.
        let mut prev_end: Option<usize> = None;
        for sel in sels.iter_sorted() {
            if let Some(pe) = prev_end {
                assert!(
                    sel.start() > pe,
                    "overlapping/adjacent selections: previous end {}, next start {}",
                    pe,
                    sel.start()
                );
            }
            prev_end = Some(sel.end());
        }
    }

    // ── Strategies ────────────────────────────────────────────────────────────

    /// Generate a random Buffer with content up to `max_len` chars.
    ///
    /// Uses a small ASCII alphabet plus spaces and newlines. `Buffer::from`
    /// normalises CRLF and appends the structural trailing `\n` if missing, so
    /// every generated buffer already satisfies the buffer invariant.
    fn arb_buffer(max_len: usize) -> impl Strategy<Value = Buffer> {
        proptest::collection::vec(
            prop_oneof![
                3 => b'a'..=b'z',  // letters are most common
                1 => Just(b' '),
                1 => Just(b'\n'),
                1 => Just(b'.'),   // punctuation for word-boundary tests
            ],
            0..=max_len,
        )
        .prop_map(|bytes| Buffer::from(String::from_utf8(bytes).unwrap().as_str()))
    }

    /// Generate a `SelectionSet` with 1..=`max_sels` valid, non-overlapping
    /// selections inside a buffer of length `buf_len`.
    ///
    /// Every position is in `0..buf_len`. Positions are paired into selections
    /// with random directionality, then sorted and de-overlapped via
    /// `merge_overlapping`.
    fn arb_selection_set(
        buf_len: usize,
        max_sels: usize,
    ) -> impl Strategy<Value = SelectionSet> {
        // With only 1 valid position (a single-char buffer of just '\n'), we
        // can only produce a single cursor at position 0.
        if buf_len <= 1 {
            return Just(SelectionSet::single(Selection::cursor(0))).boxed();
        }

        let n_sels = 1..=max_sels;
        let max_pos = buf_len - 1;

        n_sels
            .prop_flat_map(move |n| {
                // Generate 2*n positions in 0..buf_len and pair them up.
                proptest::collection::vec(0..buf_len, 2 * n)
                    .prop_flat_map(move |positions| {
                        let _ = max_pos; // suppress unused warning
                        proptest::collection::vec(proptest::bool::ANY, n).prop_map(
                            move |flips| {
                                let sels: Vec<Selection> = positions
                                    .chunks(2)
                                    .zip(flips.iter())
                                    .map(|(pair, &flip)| {
                                        let (a, b) = (pair[0].min(max_pos), pair[1].min(max_pos));
                                        // Ensure anchor != head when possible so we get real
                                        // selections, but a cursor (anchor == head) is also valid.
                                        if flip {
                                            Selection::new(a, b)
                                        } else {
                                            Selection::new(b, a)
                                        }
                                    })
                                    .collect();
                                // Use index 0 as primary; merge_overlapping adjusts it.
                                SelectionSet::from_vec(sels, 0).merge_overlapping()
                            },
                        )
                    })
            })
            .boxed()
    }

    /// Generate a random `(Buffer, SelectionSet)` pair.
    fn arb_initial_state(max_buf_len: usize) -> impl Strategy<Value = (Buffer, SelectionSet)> {
        arb_buffer(max_buf_len).prop_flat_map(|buf| {
            let buf_len = buf.len_chars();
            arb_selection_set(buf_len, 3).prop_map(move |sels| (buf.clone(), sels))
        })
    }

    // ── Operation enums ───────────────────────────────────────────────────────

    /// Edit operations that go through `Document::apply_edit` and are recorded
    /// in the undo history.
    #[derive(Debug, Clone)]
    enum EditOp {
        InsertChar(char),
        DeleteCharForward,
        DeleteCharBackward,
        DeleteSelection,
        Undo,
        Redo,
    }

    fn arb_edit_op() -> impl Strategy<Value = EditOp> {
        prop_oneof![
            // Edits weighted higher than undo/redo so the history grows first.
            4 => prop_oneof![
                Just(b'a'), Just(b'b'), Just(b'c'),
                Just(b' '), Just(b'\n'),
            ]
            .prop_map(|b| EditOp::InsertChar(b as char)),
            4 => Just(EditOp::DeleteCharForward),
            4 => Just(EditOp::DeleteCharBackward),
            4 => Just(EditOp::DeleteSelection),
            1 => Just(EditOp::Undo),
            1 => Just(EditOp::Redo),
        ]
    }

    /// Apply an `EditOp` to a `Document`, mutating it in place.
    fn apply_edit_op(doc: &mut Document, op: &EditOp) {
        match op {
            EditOp::InsertChar(ch) => {
                let ch = *ch;
                doc.apply_edit(move |b, s| insert_char(b, s, ch));
            }
            EditOp::DeleteCharForward => {
                doc.apply_edit(delete_char_forward);
            }
            EditOp::DeleteCharBackward => {
                doc.apply_edit(delete_char_backward);
            }
            EditOp::DeleteSelection => {
                doc.apply_edit(delete_selection);
            }
            EditOp::Undo => doc.undo(),
            EditOp::Redo => doc.redo(),
        }
    }

    /// Pure operations that transform `(Buffer, SelectionSet)` without
    /// touching the undo history.
    #[derive(Debug, Clone)]
    enum PureOp {
        MoveRight,
        MoveLeft,
        MoveUp,
        MoveDown,
        GotoLineStart,
        GotoLineEnd,
        SelectNextWord,
        SelectPrevWord,
        SelectNextWORD,
        SelectPrevWORD,
        InnerWord,
        AroundWord,
        InnerLine,
        CollapseSelection,
        FlipSelections,
        KeepPrimarySelection,
        CyclePrimaryForward,
        CyclePrimaryBackward,
    }

    fn arb_pure_op() -> impl Strategy<Value = PureOp> {
        prop_oneof![
            Just(PureOp::MoveRight),
            Just(PureOp::MoveLeft),
            Just(PureOp::MoveUp),
            Just(PureOp::MoveDown),
            Just(PureOp::GotoLineStart),
            Just(PureOp::GotoLineEnd),
            Just(PureOp::SelectNextWord),
            Just(PureOp::SelectPrevWord),
            Just(PureOp::SelectNextWORD),
            Just(PureOp::SelectPrevWORD),
            Just(PureOp::InnerWord),
            Just(PureOp::AroundWord),
            Just(PureOp::InnerLine),
            Just(PureOp::CollapseSelection),
            Just(PureOp::FlipSelections),
            Just(PureOp::KeepPrimarySelection),
            Just(PureOp::CyclePrimaryForward),
            Just(PureOp::CyclePrimaryBackward),
        ]
    }

    /// Apply a `PureOp`, returning the new `SelectionSet` (buffer unchanged).
    fn apply_pure_op(buf: &Buffer, sels: SelectionSet, op: &PureOp) -> SelectionSet {
        match op {
            PureOp::MoveRight => cmd_move_right(buf, sels, 1),
            PureOp::MoveLeft => cmd_move_left(buf, sels, 1),
            PureOp::MoveUp => cmd_move_up(buf, sels, 1),
            PureOp::MoveDown => cmd_move_down(buf, sels, 1),
            PureOp::GotoLineStart => cmd_goto_line_start(buf, sels, 1),
            PureOp::GotoLineEnd => cmd_goto_line_end(buf, sels, 1),
            PureOp::SelectNextWord => cmd_select_next_word(buf, sels, 1),
            PureOp::SelectPrevWord => cmd_select_prev_word(buf, sels, 1),
            PureOp::SelectNextWORD => cmd_select_next_WORD(buf, sels, 1),
            PureOp::SelectPrevWORD => cmd_select_prev_WORD(buf, sels, 1),
            PureOp::InnerWord => cmd_inner_word(buf, sels),
            PureOp::AroundWord => cmd_around_word(buf, sels),
            PureOp::InnerLine => cmd_inner_line(buf, sels),
            PureOp::CollapseSelection => cmd_collapse_selection(buf, sels),
            PureOp::FlipSelections => cmd_flip_selections(buf, sels),
            PureOp::KeepPrimarySelection => cmd_keep_primary_selection(buf, sels),
            PureOp::CyclePrimaryForward => cmd_cycle_primary_forward(buf, sels),
            PureOp::CyclePrimaryBackward => cmd_cycle_primary_backward(buf, sels),
        }
    }

    // ── Property tests ────────────────────────────────────────────────────────

    proptest! {
        /// A random sequence of edit operations (including undo and redo)
        /// applied to a Document must never violate buffer or selection
        /// invariants at any point in the sequence.
        #[test]
        fn prop_random_edit_sequence_preserves_invariants(
            (buf, sels) in arb_initial_state(30),
            ops in proptest::collection::vec(arb_edit_op(), 1..=25),
        ) {
            let mut doc = Document::new(buf, sels);
            assert_invariants(doc.buf(), doc.sels());

            for op in &ops {
                apply_edit_op(&mut doc, op);
                assert_invariants(doc.buf(), doc.sels());
            }
        }

        /// A random sequence of pure operations (motions, text objects,
        /// selection commands) must never violate buffer or selection
        /// invariants at any point in the sequence.
        #[test]
        fn prop_random_pure_ops_preserve_invariants(
            (buf, sels) in arb_initial_state(30),
            ops in proptest::collection::vec(arb_pure_op(), 1..=25),
        ) {
            let cur_buf = buf;
            let mut cur_sels = sels;
            assert_invariants(&cur_buf, &cur_sels);

            for op in &ops {
                let new_sels = apply_pure_op(&cur_buf, cur_sels, op);
                assert_invariants(&cur_buf, &new_sels);
                // cur_buf is unchanged — pure ops never modify the buffer
                cur_sels = new_sels;
            }
        }

        /// Applying any single edit then undoing it must restore the exact
        /// original buffer content and selection state.
        #[test]
        fn prop_undo_reverses_single_edit(
            (buf, sels) in arb_initial_state(30),
            op in prop_oneof![
                prop_oneof![
                    Just(b'a'), Just(b'b'), Just(b'c'), Just(b' '), Just(b'\n'),
                ].prop_map(|b| EditOp::InsertChar(b as char)),
                Just(EditOp::DeleteCharForward),
                Just(EditOp::DeleteCharBackward),
                Just(EditOp::DeleteSelection),
            ],
        ) {
            let original_content = buf.to_string();
            let original_sels = sels.clone();

            let mut doc = Document::new(buf, sels);
            apply_edit_op(&mut doc, &op);
            doc.undo();

            prop_assert_eq!(doc.buf().to_string(), original_content);
            prop_assert_eq!(doc.sels().clone(), original_sels);
        }

        /// Applying an edit, undoing it, then redoing it must produce the same
        /// state as immediately after the edit (undo+redo is identity).
        #[test]
        fn prop_undo_redo_identity(
            (buf, sels) in arb_initial_state(30),
            op in prop_oneof![
                prop_oneof![
                    Just(b'a'), Just(b'b'), Just(b'c'), Just(b' '), Just(b'\n'),
                ].prop_map(|b| EditOp::InsertChar(b as char)),
                Just(EditOp::DeleteCharForward),
                Just(EditOp::DeleteCharBackward),
                Just(EditOp::DeleteSelection),
            ],
        ) {
            let mut doc = Document::new(buf, sels);
            apply_edit_op(&mut doc, &op);

            let after_content = doc.buf().to_string();
            let after_sels = doc.sels().clone();

            doc.undo();
            doc.redo();

            prop_assert_eq!(doc.buf().to_string(), after_content);
            prop_assert_eq!(doc.sels().clone(), after_sels);
        }

        /// Applying N edits then undoing N times must restore the exact
        /// original buffer content and selections.
        #[test]
        fn prop_full_undo_restores_initial(
            (buf, sels) in arb_initial_state(30),
            // Only plain edits — no undo/redo — so undo count == edit count.
            ops in proptest::collection::vec(
                prop_oneof![
                    prop_oneof![
                        Just(b'a'), Just(b'b'), Just(b'c'), Just(b' '), Just(b'\n'),
                    ].prop_map(|b| EditOp::InsertChar(b as char)),
                    Just(EditOp::DeleteCharForward),
                    Just(EditOp::DeleteCharBackward),
                    Just(EditOp::DeleteSelection),
                ],
                1..=10,
            ),
        ) {
            let original_content = buf.to_string();
            let original_sels = sels.clone();

            let mut doc = Document::new(buf, sels);
            let n = ops.len();

            for op in &ops {
                apply_edit_op(&mut doc, op);
            }
            for _ in 0..n {
                doc.undo();
            }

            prop_assert_eq!(doc.buf().to_string(), original_content);
            prop_assert_eq!(doc.sels().clone(), original_sels);
        }

        /// Interleaved edits and undos must never violate invariants at any
        /// step. This is a weaker version of the full-undo test: it does not
        /// claim the final state matches the initial state, only that every
        /// intermediate state is valid.
        #[test]
        fn prop_interleaved_edit_undo_preserves_invariants(
            (buf, sels) in arb_initial_state(30),
            ops in proptest::collection::vec(arb_edit_op(), 1..=30),
        ) {
            let mut doc = Document::new(buf, sels);
            assert_invariants(doc.buf(), doc.sels());

            for op in &ops {
                apply_edit_op(&mut doc, op);
                assert_invariants(doc.buf(), doc.sels());
            }
        }
    }
}
