use hume_editing::changeset::{ChangeSet, Operation};

// ── Incremental parse helpers ─────────────────────────────────────────────────

/// Translate a `ChangeSet` into a sequence of `tree_sitter::InputEdit`s.
///
/// `rope` must be the buffer text **before** the edit (the old document).  All
/// char offsets in the changeset are converted to byte offsets and (row, byte-col)
/// positions via the rope's index helpers.
pub(crate) fn input_edits_from_changeset(
    cs: &ChangeSet,
    rope: &ropey::Rope,
) -> Vec<tree_sitter::InputEdit> {
    let mut edits = Vec::new();
    let mut pre_char: usize = 0;
    let mut ops = cs.ops().iter();

    while let Some(op) = ops.next() {
        match op {
            Operation::Retain(n) => {
                pre_char += n;
            }
            Operation::Delete(del_n) => {
                let start_char = pre_char;
                let old_end_char = pre_char + del_n;
                // A following Insert forms a replace — consume it together.
                let inserted = match ops.as_slice().first() {
                    Some(Operation::Insert(s)) => {
                        let s = s.as_str();
                        ops.next();
                        s
                    }
                    _ => "",
                };
                edits.push(make_input_edit(start_char, old_end_char, inserted, rope));
                pre_char = old_end_char;
            }
            Operation::Insert(ins_s) => {
                // Pure insert: old document position doesn't advance.
                edits.push(make_input_edit(pre_char, pre_char, ins_s.as_str(), rope));
            }
        }
    }

    // All edits are computed in pre-edit coordinate space (the old rope).
    // `tree.edit()` mutates coordinates in-place: applying a left edit first shifts
    // every subsequent byte position, so a right edit specified in original coords
    // would land at the wrong place.  Reversing to descending start order means the
    // rightmost edit is applied first — its coordinates are never invalidated by
    // anything to its left, and vice versa, so all edits remain valid in the
    // pre-edit coordinate space at apply time.
    edits.reverse();
    edits
}

/// Build a single `InputEdit` from char-indexed old/new positions and the inserted text.
fn make_input_edit(
    start_char: usize,
    old_end_char: usize,
    inserted: &str,
    rope: &ropey::Rope,
) -> tree_sitter::InputEdit {
    let start_byte = rope.char_to_byte(start_char);
    let old_end_byte = rope.char_to_byte(old_end_char);
    let new_end_byte = start_byte + inserted.len(); // str::len() is byte count

    let start_row = rope.char_to_line(start_char);
    let start_col = start_byte - rope.line_to_byte(start_row);

    let old_end_row = rope.char_to_line(old_end_char);
    let old_end_col = old_end_byte - rope.line_to_byte(old_end_row);

    let (new_end_row, new_end_col) = new_end_point(start_row, start_col, inserted);

    tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: tree_sitter::Point {
            row: start_row,
            column: start_col,
        },
        old_end_position: tree_sitter::Point {
            row: old_end_row,
            column: old_end_col,
        },
        new_end_position: tree_sitter::Point {
            row: new_end_row,
            column: new_end_col,
        },
    }
}

/// Compute `new_end_position` for an insertion starting at `(start_row, start_col)`.
fn new_end_point(start_row: usize, start_col: usize, inserted: &str) -> (usize, usize) {
    let newline_count = inserted.bytes().filter(|&b| b == b'\n').count();
    if newline_count == 0 {
        (start_row, start_col + inserted.len())
    } else {
        // Column is the byte count after the last newline in the inserted text.
        let last_nl = inserted.rfind('\n').unwrap();
        (start_row + newline_count, inserted.len() - last_nl - 1)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{input_edits_from_changeset, new_end_point};
    use hume_editing::changeset::ChangeSetBuilder;

    #[test]
    fn pure_insert_at_start() {
        let rope = ropey::Rope::from_str("hello\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.insert("AB");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 0);
        assert_eq!(e.old_end_byte, 0);
        assert_eq!(e.new_end_byte, 2);
        assert_eq!(e.start_position, tree_sitter::Point { row: 0, column: 0 });
        assert_eq!(e.old_end_position, tree_sitter::Point { row: 0, column: 0 });
        assert_eq!(e.new_end_position, tree_sitter::Point { row: 0, column: 2 });
    }

    #[test]
    fn pure_insert_middle() {
        let rope = ropey::Rope::from_str("hello\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(3);
        b.insert("XY");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 3);
        assert_eq!(e.old_end_byte, 3);
        assert_eq!(e.new_end_byte, 5);
        assert_eq!(e.new_end_position.column, 5);
    }

    #[test]
    fn pure_delete_single_char() {
        let rope = ropey::Rope::from_str("abc\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(1);
        b.delete(1);
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 1);
        assert_eq!(e.old_end_byte, 2);
        assert_eq!(e.new_end_byte, 1);
    }

    #[test]
    fn delete_crosses_line_boundary() {
        let rope = ropey::Rope::from_str("foo\nbar\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(3);
        b.delete(3); // deletes "\nba"
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 3);
        assert_eq!(e.old_end_byte, 6);
        assert_eq!(e.new_end_byte, 3);
        assert_eq!(e.old_end_position, tree_sitter::Point { row: 1, column: 2 });
    }

    #[test]
    fn replace_within_one_line() {
        let rope = ropey::Rope::from_str("hello world\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(6);
        b.delete(5);
        b.insert("Rust");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 6);
        assert_eq!(e.old_end_byte, 11);
        assert_eq!(e.new_end_byte, 10);
        assert_eq!(e.new_end_position.column, 10);
    }

    #[test]
    fn multiline_insert_new_end_position() {
        let rope = ropey::Rope::from_str("ab\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(1);
        b.insert("foo\nbar\n");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        // "foo\nbar\n" — 2 newlines; last '\n' at byte 7; col = 8 - 7 - 1 = 0
        assert_eq!(e.new_end_position.row, 2);
        assert_eq!(e.new_end_position.column, 0);
    }

    #[test]
    fn two_separate_edit_sites_emit_two_edits() {
        let rope = ropey::Rope::from_str("abc\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.delete(1); // delete 'a'
        b.retain(1); // keep 'b'
        b.delete(1); // delete 'c'
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 2);
        // Edits are returned in DESCENDING start-byte order so callers that apply
        // them via `tree.edit()` (which mutates coordinates in-place) apply the
        // rightmost edit first — keeping all original-coordinate offsets valid.
        assert!(
            edits[0].start_byte > edits[1].start_byte,
            "edits must be in descending start-byte order for correct tree.edit() baking"
        );
        assert_eq!(edits[0].start_byte, 2);
        assert_eq!(edits[0].old_end_byte, 3);
        assert_eq!(edits[1].start_byte, 0);
        assert_eq!(edits[1].old_end_byte, 1);
    }

    #[test]
    fn multibyte_utf8_byte_offsets() {
        // "é" = U+00E9 (precomposed) = 2 bytes in UTF-8, 1 char.
        // "漢" = U+6F22 = 3 bytes, 1 char.
        let rope = ropey::Rope::from_str("é漢\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.delete(1); // delete "é" (1 char, but 2 bytes)
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 0);
        // "é" (U+00E9) = 2 bytes — different from char count of 1.
        assert_eq!(e.old_end_byte, 2, "byte offset must count bytes not chars");
        assert_eq!(e.new_end_byte, 0);
    }

    #[test]
    fn new_end_point_no_newlines() {
        let (row, col) = new_end_point(2, 5, "hello");
        assert_eq!(row, 2);
        assert_eq!(col, 10); // 5 + 5
    }

    #[test]
    fn new_end_point_with_newlines() {
        let (row, col) = new_end_point(1, 3, "foo\nbar\nbaz");
        // 2 newlines → row + 2 = 3; col = "baz".len() = 3
        assert_eq!(row, 3);
        assert_eq!(col, 3);
    }

    #[test]
    fn new_end_point_trailing_newline() {
        // Inserted text ends with '\n' — col must be 0.
        let (row, col) = new_end_point(0, 0, "foo\n");
        assert_eq!(row, 1);
        assert_eq!(col, 0);
    }

    /// Regression: a single changeset with edits at two non-adjacent positions must
    /// produce an incremental parse tree identical to a full reparse of the same bytes.
    ///
    /// The fix: `input_edits_from_changeset` returns edits in DESCENDING start-byte
    /// order.  `tree.edit()` mutates coordinates in-place, so the rightmost edit must
    /// be applied first — its original-coordinate bytes stay valid because nothing to
    /// its left has been touched yet.  Before the fix (ascending order), a left edit's
    /// byte-delta corrupted the right edit's coordinates, misaligning nodes and causing
    /// highlight queries to return wrong results after multi-cursor edits.
    ///
    /// Uses the JSON grammar from `tests/fixtures/grammars/` (requires
    /// `scripts/fetch-test-grammars.sh`).
    #[test]
    fn multi_edit_changeset_incremental_tree_matches_full_reparse() {
        use hume_engine::grammar::LoadedGrammar;

        let parser_path = crate::test_support::grammar_parser_path("json");
        if !parser_path.exists() {
            // Grammar fixture not fetched — skip rather than fail CI unexpectedly.
            return;
        }

        let grammar =
            LoadedGrammar::open(&parser_path, "tree_sitter_json").expect("load json grammar");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(grammar.language())
            .expect("set language");

        // Old text: JSON array.  Two edits: replace "abc" (chars 2-4) with "X" and
        // replace "def" (chars 8-10) with "YY".  Different byte deltas at each site
        // so the left edit's shift would corrupt the right edit's coordinates if
        // applied in the wrong order.
        //
        // Chars: [ " a b c " , " d e f " ]  \n
        //         0  1 2 3 4 5 6  7 8 9 10 11 12 13
        let old_text = "[\"abc\",\"def\"]\n";
        let rope = ropey::Rope::from_str(old_text);

        let old_bytes: Vec<u8> = old_text.bytes().collect();
        let old_tree = parser.parse(&old_bytes, None).expect("initial parse");

        // Changeset: retain 2, delete 3 + insert "X", retain 3, delete 3 + insert "YY", retain rest.
        // Edit 1: chars [2,5) → "X"   (byte delta: 1 - 3 = -2)
        // Edit 2: chars [8,11) → "YY" (byte delta: 2 - 3 = -1)
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(2);
        b.delete(3);
        b.insert("X");
        b.retain(3);
        b.delete(3);
        b.insert("YY");
        b.retain_rest();
        let cs = b.finish();

        // Verify edits come out in descending order (right before left).
        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 2, "expected two edits from the changeset");
        assert!(
            edits[0].start_byte > edits[1].start_byte,
            "edits must be descending: right edit first, then left"
        );

        // Apply edits and do an incremental reparse.
        let mut baked_tree = old_tree;
        for edit in &edits {
            baked_tree.edit(edit);
        }
        let new_text = "[\"X\",\"YY\"]\n";
        let new_bytes: Vec<u8> = new_text.bytes().collect();
        let incremental_tree = parser
            .parse(&new_bytes, Some(&baked_tree))
            .expect("incremental parse");
        let full_tree = parser.parse(&new_bytes, None).expect("full parse");

        assert_eq!(
            incremental_tree.root_node().to_sexp(),
            full_tree.root_node().to_sexp(),
            "incremental tree from multi-edit changeset must match full parse"
        );
    }
}
