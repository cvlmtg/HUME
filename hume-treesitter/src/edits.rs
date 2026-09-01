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
                // A replacement reaches here in either op order: every
                // `hume-ops` builder emits delete-then-insert (handled by the
                // lookahead above), but `ChangeSet::invert` emits
                // insert-then-delete for every undone replacement, `compose`
                // can produce either, and `indent`/`unindent` deliberately
                // emit insert-then-delete (see `hume-ops/src/edit/indent.rs`)
                // so their own position remap can use `Assoc`. A following
                // Delete is consumed together so both orders still coalesce
                // into one edit.
                let del_n = match ops.as_slice().first() {
                    Some(Operation::Delete(n)) => {
                        let n = *n;
                        ops.next();
                        n
                    }
                    _ => 0,
                };
                edits.push(make_input_edit(
                    pre_char,
                    pre_char + del_n,
                    ins_s.as_str(),
                    rope,
                ));
                pre_char += del_n;
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

    let (start_row, start_byte_col) = hume_rope::lines::char_to_line_byte(rope, start_char);
    let (old_end_row, old_end_byte_col) = hume_rope::lines::char_to_line_byte(rope, old_end_char);

    let (new_end_row, new_end_byte_col) =
        hume_rope::lines::advance_byte_point(start_row, start_byte_col, inserted);

    tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: tree_sitter::Point {
            row: start_row,
            column: start_byte_col, // column-name-safe: tree-sitter's Point::column is a byte offset
        },
        old_end_position: tree_sitter::Point {
            row: old_end_row,
            column: old_end_byte_col, // column-name-safe: tree-sitter's Point::column is a byte offset
        },
        new_end_position: tree_sitter::Point {
            row: new_end_row,
            column: new_end_byte_col, // column-name-safe: tree-sitter's Point::column is a byte offset
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
