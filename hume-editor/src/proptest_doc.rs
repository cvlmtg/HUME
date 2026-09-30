//! Property-based tests for BufferText-level invariants.
//!
//! These tests complement the unit tests in individual modules and the
//! ChangeSet-level proptests in `hume-editing/src/changeset/`. They verify
//! that:
//!
//! 1. Any sequence of edit operations + undo/redo never corrupts the buffer
//!    or desynchronises the selection set.
//! 2. Any sequence of pure operations (motions, text objects, selection
//!    commands) never violates the buffer or selection invariants.
//! 3. Specific undo/redo properties hold (undo reverses an edit, undo+redo
//!    is identity, N edits then N undos restores the original state).
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::editor::buffer::Buffer;
    use crate::editor::tests::co;
    use hume_editing::changeset::ChangeSet;
    use hume_editing::grapheme::{graphemes_at, is_cluster_boundary};
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    use hume_ops::edit::{
        delete_char_backward, delete_char_forward, delete_selection, insert_char,
    };
    use hume_ops::motion::{
        cmd_goto_line_end, cmd_goto_line_start, cmd_move_left, cmd_move_right,
        cmd_select_next_uppercase_word, cmd_select_next_word, cmd_select_prev_uppercase_word,
        cmd_select_prev_word,
    };
    use hume_ops::register::yank_selections;
    use hume_ops::selection_cmd::{
        cmd_collapse_selection_to_head, cmd_cycle_primary_backward, cmd_cycle_primary_forward,
        cmd_flip_selections, cmd_keep_primary_selection,
    };
    use hume_ops::text_object::{
        cmd_around_word, cmd_inner_line, cmd_inner_word, cmd_select_uppercase_word,
    };
    use hume_ops::{MotionMode, WordCtx};

    // ── DocHelper — thin wrapper keeping sels alongside Buffer ────────────────

    struct DocHelper {
        buf: Buffer,
        sels: SelectionSet,
    }

    impl DocHelper {
        fn new(text: BufferText, sels: SelectionSet) -> Self {
            let buf = Buffer::new(text, sels.clone());
            Self { buf, sels }
        }
        fn text(&self) -> &BufferText {
            self.buf.text()
        }
        fn apply_edit(
            &mut self,
            cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
        ) {
            let (new_sels, _cs) = self.buf.apply_edit(self.sels.clone(), cmd);
            self.sels = new_sels;
        }
        fn undo(&mut self) {
            if let Some((sels, _cs, _steps)) = self.buf.undo_n(1) {
                self.sels = sels;
            }
        }
        fn redo(&mut self) {
            if let Some((sels, _cs, _steps)) = self.buf.redo_n(1) {
                self.sels = sels;
            }
        }
    }

    // ── Invariant checker ─────────────────────────────────────────────────────

    /// Assert all buffer and selection invariants after any operation.
    ///
    /// Called after every operation in every proptest. A panic here means the
    /// code under test produced an invalid state.
    fn assert_invariants(text: &BufferText, sels: &SelectionSet) {
        // Text invariant 1: always ends with structural '\n'.
        assert!(
            text.to_string().ends_with('\n'),
            "buffer must end with \\n, got: {:?}",
            text.to_string()
        );

        // Text invariant 2: len_chars > 0 (at minimum the structural '\n').
        let len = text.end();
        assert!(len > co(0), "buffer must have at least 1 char");

        // SelectionSet invariant 1: never empty.
        assert!(sels.len() > 0, "selection set must not be empty");

        // SelectionSet invariant 2: all positions strictly within the buffer.
        for sel in sels.iter_sorted() {
            assert!(
                sel.head() < len,
                "selection head {:?} out of bounds (text len {:?})",
                sel.head(),
                len
            );
            assert!(
                sel.anchor() < len,
                "selection anchor {:?} out of bounds (text len {:?})",
                sel.anchor(),
                len
            );
        }

        // SelectionSet invariant 3: every anchor and head is a cluster start.
        for sel in sels.iter_sorted() {
            assert!(
                is_cluster_boundary(text, sel.anchor()) && is_cluster_boundary(text, sel.head()),
                "selection ({:?}, {:?}) splits a grapheme cluster of {:?}",
                sel.anchor(),
                sel.head(),
                text.to_string()
            );
        }

        // SelectionSet invariant 4: sorted ascending by start().
        let starts: Vec<_> = sels.iter_sorted().map(|s| s.start()).collect();
        for w in starts.windows(2) {
            assert!(
                w[0] <= w[1],
                "selections not sorted: start {:?} > start {:?}",
                w[0],
                w[1]
            );
        }

        // SelectionSet invariant 5: no overlapping or adjacent selections.
        // Adjacent means one ends where the next begins; both are merged.
        let mut prev_end: Option<hume_rope::offset::CharOffset> = None;
        for sel in sels.iter_sorted() {
            if let Some(pe) = prev_end {
                assert!(
                    sel.start() > pe,
                    "overlapping/adjacent selections: previous end {:?}, next start {:?}",
                    pe,
                    sel.start()
                );
            }
            prev_end = Some(sel.end_inclusive(text));
        }
    }

    // ── Strategies ────────────────────────────────────────────────────────────

    /// One piece of buffer text: an ASCII letter, blank, punctuation, or a
    /// unicode-corpus sample (combining marks, ZWJ emoji, flags, CJK, NBSP…).
    fn arb_atom() -> impl Strategy<Value = String> {
        prop_oneof![
            6 => (b'a'..=b'z').prop_map(|b| char::from(b).to_string()),
            2 => Just(" ".to_string()),
            2 => Just("\n".to_string()),
            1 => Just(".".to_string()),
            1 => Just("\t".to_string()),
            1 => Just("\r\n".to_string()),
            5 => proptest::sample::select(test_fixtures::unicode::ALL).prop_map(str::to_string),
        ]
    }

    /// Generate a random BufferText of up to `max_atoms` atoms.
    ///
    /// `BufferText::from` normalises every line ending to LF and appends the
    /// structural trailing `\n` if missing, so every generated buffer already
    /// satisfies the buffer invariant.
    fn arb_buffer(max_atoms: usize) -> impl Strategy<Value = BufferText> {
        proptest::collection::vec(arb_atom(), 0..=max_atoms)
            .prop_map(|atoms| BufferText::from(atoms.concat().as_str()))
    }

    /// The char offset of every grapheme cluster start in `text`.
    fn cluster_starts(text: &BufferText) -> Vec<usize> {
        graphemes_at(text, co(0))
            .map(|cluster| cluster.start.index())
            .collect()
    }

    /// Generate a `SelectionSet` with 1..=`max_sels` selections whose anchors
    /// and heads are drawn from `starts` (every one a cluster start),
    /// merged into a valid set.
    fn arb_selection_set(
        starts: Vec<usize>,
        max_sels: usize,
    ) -> impl Strategy<Value = SelectionSet> {
        let n_starts = starts.len();
        (1..=max_sels)
            .prop_flat_map(move |n| {
                let starts = starts.clone();
                proptest::collection::vec((0..n_starts, 0..n_starts), n).prop_map(move |picks| {
                    let sels: Vec<Selection> = picks
                        .into_iter()
                        .map(|(a, b)| Selection::new(co(starts[a]), co(starts[b])))
                        .collect();
                    SelectionSet::from_vec(sels, 0)
                })
            })
            .boxed()
    }

    /// Generate a random `(BufferText, SelectionSet)` pair.
    fn arb_initial_state(max_buf_len: usize) -> impl Strategy<Value = (BufferText, SelectionSet)> {
        arb_buffer(max_buf_len).prop_flat_map(|text| {
            arb_selection_set(cluster_starts(&text), 3).prop_map(move |sels| (text.clone(), sels))
        })
    }

    // ── Operation enums ───────────────────────────────────────────────────────

    /// Edit operations that go through `BufferText::apply_edit` and are recorded
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

    /// A typed char: ASCII, or a char that joins or extends a cluster
    /// (combining mark, ZWJ, variation selector, regional indicator) or is
    /// wide (CJK, emoji).
    fn arb_insert_char() -> impl Strategy<Value = char> {
        prop_oneof![
            4 => prop_oneof![Just('a'), Just('b'), Just('c'), Just(' '), Just('\n')],
            3 => prop_oneof![
                Just('\u{301}'), Just('\u{200d}'), Just('\u{fe0f}'), Just('\u{1f1ee}'),
                Just('\u{1f1f9}'), Just('\u{e9}'), Just('\u{6f22}'), Just('\u{1f600}'),
                Just('\u{a0}'),
            ],
        ]
    }

    fn arb_edit_op() -> impl Strategy<Value = EditOp> {
        prop_oneof![
            // Edits weighted higher than undo/redo so the history grows first.
            4 => arb_insert_char().prop_map(EditOp::InsertChar),
            4 => Just(EditOp::DeleteCharForward),
            4 => Just(EditOp::DeleteCharBackward),
            4 => Just(EditOp::DeleteSelection),
            1 => Just(EditOp::Undo),
            1 => Just(EditOp::Redo),
        ]
    }

    /// Apply an `EditOp` to a `DocHelper`, mutating it in place.
    fn apply_edit_op(doc: &mut DocHelper, op: &EditOp) {
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

    /// Pure operations that transform `(BufferText, SelectionSet)` without
    /// touching the undo history.
    #[derive(Debug, Clone)]
    enum PureOp {
        MoveRight,
        MoveLeft,
        GotoLineStart,
        GotoLineEnd,
        SelectNextWord,
        SelectPrevWord,
        SelectNextUppercaseWord,
        SelectPrevUppercaseWord,
        SelectNextWordAround,
        SelectPrevWordAround,
        SelectNextUppercaseWordAround,
        SelectPrevUppercaseWordAround,
        InnerWord,
        AroundWord,
        SelectUppercaseWordAround,
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
            Just(PureOp::GotoLineStart),
            Just(PureOp::GotoLineEnd),
            Just(PureOp::SelectNextWord),
            Just(PureOp::SelectPrevWord),
            Just(PureOp::SelectNextUppercaseWord),
            Just(PureOp::SelectPrevUppercaseWord),
            Just(PureOp::SelectNextWordAround),
            Just(PureOp::SelectPrevWordAround),
            Just(PureOp::SelectNextUppercaseWordAround),
            Just(PureOp::SelectPrevUppercaseWordAround),
            Just(PureOp::InnerWord),
            Just(PureOp::AroundWord),
            Just(PureOp::SelectUppercaseWordAround),
            Just(PureOp::InnerLine),
            Just(PureOp::CollapseSelection),
            Just(PureOp::FlipSelections),
            Just(PureOp::KeepPrimarySelection),
            Just(PureOp::CyclePrimaryForward),
            Just(PureOp::CyclePrimaryBackward),
        ]
    }

    fn arb_motion_mode() -> impl Strategy<Value = MotionMode> {
        prop_oneof![Just(MotionMode::Move), Just(MotionMode::Extend)]
    }

    /// Apply a `PureOp` with the given `MotionMode`, returning the new
    /// `SelectionSet` (buffer unchanged).
    ///
    /// Word-family ops use `WordCtx::bare`/`WordCtx::around` (no configured
    /// `word-chars`): there is no per-buffer settings layer here to resolve
    /// a real one from, and none of these proptests exercise `word-chars`
    /// itself (that is covered by `hume-editor/src/editor/tests/word_chars.rs`).
    fn apply_pure_op(
        text: &BufferText,
        sels: SelectionSet,
        op: &PureOp,
        mode: MotionMode,
    ) -> SelectionSet {
        match op {
            PureOp::MoveRight => cmd_move_right(text, sels, 1, mode),
            PureOp::MoveLeft => cmd_move_left(text, sels, 1, mode),
            PureOp::GotoLineStart => cmd_goto_line_start(text, sels, 1, mode),
            PureOp::GotoLineEnd => cmd_goto_line_end(text, sels, 1, mode),
            PureOp::SelectNextWord => cmd_select_next_word(text, sels, 1, WordCtx::bare(mode)),
            PureOp::SelectPrevWord => cmd_select_prev_word(text, sels, 1, WordCtx::bare(mode)),
            PureOp::SelectNextUppercaseWord => {
                cmd_select_next_uppercase_word(text, sels, 1, WordCtx::bare(mode))
            }
            PureOp::SelectPrevUppercaseWord => {
                cmd_select_prev_uppercase_word(text, sels, 1, WordCtx::bare(mode))
            }
            // "Around" variants exercise the same command with `around: true`
            // (effective `word-selects-whitespace`).
            PureOp::SelectNextWordAround => {
                cmd_select_next_word(text, sels, 1, WordCtx::around(mode))
            }
            PureOp::SelectPrevWordAround => {
                cmd_select_prev_word(text, sels, 1, WordCtx::around(mode))
            }
            PureOp::SelectNextUppercaseWordAround => {
                cmd_select_next_uppercase_word(text, sels, 1, WordCtx::around(mode))
            }
            PureOp::SelectPrevUppercaseWordAround => {
                cmd_select_prev_uppercase_word(text, sels, 1, WordCtx::around(mode))
            }
            PureOp::InnerWord => cmd_inner_word(text, sels, 0, WordCtx::bare(mode)),
            PureOp::AroundWord => cmd_around_word(text, sels, 0, WordCtx::bare(mode)),
            PureOp::SelectUppercaseWordAround => {
                cmd_select_uppercase_word(text, sels, 0, WordCtx::around(mode))
            }
            PureOp::InnerLine => cmd_inner_line(text, sels, 0, mode),
            // Selection-manipulation commands don't use mode; pass it anyway for API uniformity.
            PureOp::CollapseSelection => cmd_collapse_selection_to_head(text, sels, 0, mode),
            PureOp::FlipSelections => cmd_flip_selections(text, sels, 0, mode),
            PureOp::KeepPrimarySelection => cmd_keep_primary_selection(text, sels, 0, mode),
            PureOp::CyclePrimaryForward => cmd_cycle_primary_forward(text, sels, 0, mode),
            PureOp::CyclePrimaryBackward => cmd_cycle_primary_backward(text, sels, 0, mode),
        }
    }

    // ── Property tests ────────────────────────────────────────────────────────

    proptest! {
        /// A random sequence of edit operations (including undo and redo)
        /// applied to a BufferText must never violate buffer or selection
        /// invariants at any point in the sequence.
        #[test]
        fn prop_random_edit_sequence_preserves_invariants(
            (text, sels) in arb_initial_state(30),
            ops in proptest::collection::vec(arb_edit_op(), 1..=25),
        ) {
            let mut doc = DocHelper::new(text, sels);
            assert_invariants(doc.text(), &doc.sels);

            for op in &ops {
                apply_edit_op(&mut doc, op);
                assert_invariants(doc.text(), &doc.sels);
            }
        }

        /// A random sequence of pure operations (motions, text objects,
        /// selection commands) must never violate buffer or selection
        /// invariants at any point in the sequence.
        #[test]
        fn prop_random_pure_ops_preserve_invariants(
            (text, sels) in arb_initial_state(30),
            ops in proptest::collection::vec((arb_pure_op(), arb_motion_mode()), 1..=25),
        ) {
            let cur_text = text;
            let mut cur_sels = sels;
            assert_invariants(&cur_text, &cur_sels);

            for (op, mode) in &ops {
                let new_sels = apply_pure_op(&cur_text, cur_sels, op, *mode);
                assert_invariants(&cur_text, &new_sels);
                // cur_text is unchanged: pure ops never modify the buffer
                cur_sels = new_sels;
            }
        }

        /// Applying any single edit then undoing it must restore the exact
        /// original buffer content and selection state.
        #[test]
        fn prop_undo_reverses_single_edit(
            (text, sels) in arb_initial_state(30),
            op in prop_oneof![
                arb_insert_char().prop_map(EditOp::InsertChar),
                Just(EditOp::DeleteCharForward),
                Just(EditOp::DeleteCharBackward),
                Just(EditOp::DeleteSelection),
            ],
        ) {
            let original_content = text.to_string();
            let original_sels = sels.clone();

            let mut doc = DocHelper::new(text, sels);
            apply_edit_op(&mut doc, &op);
            doc.undo();

            prop_assert_eq!(doc.text().to_string(), original_content);
            prop_assert_eq!(doc.sels.clone(), original_sels);
        }

        /// Applying an edit, undoing it, then redoing it must produce the same
        /// state as immediately after the edit (undo+redo is identity).
        #[test]
        fn prop_undo_redo_identity(
            (text, sels) in arb_initial_state(30),
            op in prop_oneof![
                arb_insert_char().prop_map(EditOp::InsertChar),
                Just(EditOp::DeleteCharForward),
                Just(EditOp::DeleteCharBackward),
                Just(EditOp::DeleteSelection),
            ],
        ) {
            let mut doc = DocHelper::new(text, sels);
            apply_edit_op(&mut doc, &op);

            let after_content = doc.text().to_string();
            let after_sels = doc.sels.clone();

            doc.undo();
            doc.redo();

            prop_assert_eq!(doc.text().to_string(), after_content);
            prop_assert_eq!(doc.sels.clone(), after_sels);
        }

        /// Applying N edits then undoing N times must restore the exact
        /// original buffer content and selections.
        #[test]
        fn prop_full_undo_restores_initial(
            (text, sels) in arb_initial_state(30),
            // Only plain edits (no undo/redo), so undo count == edit count.
            ops in proptest::collection::vec(
                prop_oneof![
                    arb_insert_char().prop_map(EditOp::InsertChar),
                    Just(EditOp::DeleteCharForward),
                    Just(EditOp::DeleteCharBackward),
                    Just(EditOp::DeleteSelection),
                ],
                1..=10,
            ),
        ) {
            let original_content = text.to_string();
            let original_sels = sels.clone();

            let mut doc = DocHelper::new(text, sels);
            let n = ops.len();

            for op in &ops {
                apply_edit_op(&mut doc, op);
            }
            for _ in 0..n {
                doc.undo();
            }

            prop_assert_eq!(doc.text().to_string(), original_content);
            prop_assert_eq!(doc.sels.clone(), original_sels);
        }

        /// Snapping arbitrary positions to clusters gives a valid,
        /// cluster-aligned set, and snapping it again changes nothing.
        #[test]
        fn prop_snap_to_clusters_is_valid_and_idempotent(
            text in arb_buffer(30),
            picks in proptest::collection::vec((0usize..500, 0usize..500), 1..=3),
        ) {
            let len = text.len_chars();
            let raw: Vec<Selection> = picks
                .into_iter()
                .map(|(a, b)| Selection::new(co(a % len), co(b % len)))
                .collect();
            let mut set = SelectionSet::from_vec_unchecked(raw, 0);
            set.snap_to_clusters(&text);
            assert_invariants(&text, &set);
            let mut again = set.clone();
            again.snap_to_clusters(&text);
            prop_assert_eq!(again, set);
        }

        /// Yanking returns the grapheme clusters each selection
        /// covers, in sorted order.
        #[test]
        fn prop_yank_returns_the_covered_clusters(
            (text, sels) in arb_initial_state(30),
        ) {
            let clusters: Vec<_> = graphemes_at(&text, co(0)).collect();
            let chars: Vec<char> = text.to_string().chars().collect();
            let expected: Vec<String> = sels
                .iter_sorted()
                .map(|sel| {
                    let last = sel.end_inclusive(&text);
                    clusters
                        .iter()
                        .filter(|c| c.start >= sel.start() && c.start <= last)
                        .flat_map(|c| chars[c.start.index()..c.end.index()].iter())
                        .collect()
                })
                .collect();
            prop_assert_eq!(yank_selections(&text, &sels), expected);
        }

        /// Interleaved edits and undos must never violate invariants at any
        /// step. This is a weaker version of the full-undo test: it does not
        /// claim the final state matches the initial state, only that every
        /// intermediate state is valid.
        #[test]
        fn prop_interleaved_edit_undo_preserves_invariants(
            (text, sels) in arb_initial_state(30),
            ops in proptest::collection::vec(arb_edit_op(), 1..=30),
        ) {
            let mut doc = DocHelper::new(text, sels);
            assert_invariants(doc.text(), &doc.sels);

            for op in &ops {
                apply_edit_op(&mut doc, op);
                assert_invariants(doc.text(), &doc.sels);
            }
        }
    }
}
