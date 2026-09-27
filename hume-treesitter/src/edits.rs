use hume_editing::changeset::ChangeSet;

// ── Incremental parse helpers ─────────────────────────────────────────────────

/// Translate a `ChangeSet` into a sequence of `tree_sitter::InputEdit`s.
///
/// `rope` must be the buffer text **before** the edit (the old document). One
/// `InputEdit` per [`ChangeSet::edited_regions`] region: that walk already
/// pairs a `Delete`/`Insert` replacement in either op order (every
/// `hume-ops` builder emits delete-then-insert, but `ChangeSet::invert`
/// emits insert-then-delete for every undone replacement, `compose` can
/// produce either, and `indent`/`unindent` deliberately emit
/// insert-then-delete; see `hume-ops/src/edit/indent.rs`) into one region,
/// so this walk doesn't have to.
pub(crate) fn input_edits_from_changeset(
    cs: &ChangeSet,
    rope: &ropey::Rope,
) -> Vec<tree_sitter::InputEdit> {
    let mut edits: Vec<tree_sitter::InputEdit> = cs
        .edited_regions()
        .into_iter()
        .map(|r| make_input_edit(r.old.start.index(), r.old.end.index(), &r.inserted, rope))
        .collect();

    // All edits are computed in pre-edit coordinate space (the old rope).
    // `tree.edit()` mutates coordinates in-place: applying a left edit first shifts
    // every subsequent byte position, so a right edit specified in original coords
    // would land at the wrong place.  Reversing to descending start order means the
    // rightmost edit is applied first: its coordinates are never invalidated by
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

    let (start_row, start_byte_col) =
        hume_rope::lines::char_to_line_byte(rope, hume_rope::offset::CharOffset::new(start_char));
    let (old_end_row, old_end_byte_col) =
        hume_rope::lines::char_to_line_byte(rope, hume_rope::offset::CharOffset::new(old_end_char));

    let (new_end_row, new_end_byte_col) =
        hume_rope::lines::advance_byte_point(start_row.index(), start_byte_col, inserted);

    tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: tree_sitter::Point {
            row: start_row.index(), // tree-sitter's own row unit, not this crate's line-domain type
            column: start_byte_col.index(),
        },
        old_end_position: tree_sitter::Point {
            row: old_end_row.index(), // tree-sitter's own row unit, not this crate's line-domain type
            column: old_end_byte_col.index(),
        },
        new_end_position: tree_sitter::Point {
            row: new_end_row,
            column: new_end_byte_col.index(),
        },
    }
}

#[cfg(test)]
mod tests;
