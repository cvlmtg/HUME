use hume_editing::changeset::ChangeSet;

// ── Incremental parse helpers ─────────────────────────────────────────────────

/// Translate a `ChangeSet` into a sequence of `tree_sitter::InputEdit`s.
///
/// `rope` must be the buffer text **before** the edit (the old document). One
/// `InputEdit` per [`ChangeSet::edited_regions`] region: that walk already
/// pairs a `Delete`/`Insert` replacement in either op order (every
/// `hume-ops` builder emits delete-then-insert, but `ChangeSet::invert`
/// emits insert-then-delete for every undone replacement, `compose` can
/// produce either, and `indent`/`unindent` emit
/// insert-then-delete; see `hume-ops/src/edit/indent.rs`) into one region,
/// so this walk doesn't have to.
pub(crate) fn input_edits_from_changeset(cs: &ChangeSet, rope: &ropey::Rope) -> ChangeEdits {
    // All edits are computed in pre-edit coordinate space (the old rope).
    // `tree.edit()` mutates coordinates in-place: applying a left edit first shifts
    // every subsequent byte position, so a right edit specified in original coords
    // would land at the wrong place.  Reversing to descending start order means the
    // rightmost edit is applied first: its coordinates are never invalidated by
    // anything to its left, and vice versa, so all edits remain valid in the
    // pre-edit coordinate space at apply time.
    ChangeEdits(
        cs.edited_regions()
            .into_iter()
            .rev()
            .map(|r| make_input_edit(r.old.start.index(), r.old.end.index(), &r.inserted, rope))
            .collect(),
    )
}

/// The `InputEdit`s of one change, in the order `Tree::edit` takes them:
/// over disjoint regions of the text before it, each ending before the
/// next one starts, by descending start, each in that text's coordinates.
#[derive(Clone, Debug)]
pub(crate) struct ChangeEdits(Vec<tree_sitter::InputEdit>);

impl std::ops::Deref for ChangeEdits {
    type Target = [tree_sitter::InputEdit];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ChangeEdits {
    /// Where the change moves each byte of the text before it.
    pub(crate) fn byte_map(&self) -> ByteMap {
        let mut edits = Vec::with_capacity(self.0.len());
        let mut shifts = Vec::with_capacity(self.0.len() + 1);
        let mut shift = 0isize;
        shifts.push(shift);
        for edit in self.0.iter().rev() {
            edits.push((edit.start_byte, edit.old_end_byte));
            shift += edit.new_end_byte as isize - edit.old_end_byte as isize;
            shifts.push(shift);
        }
        ByteMap { edits, shifts }
    }
}

/// Where one change moves each byte of the text before it, as
/// `InputEdit::edit_range` moves a range's ends through each of its edits
/// in turn: a byte inside an edit moves to the edit's start, and one at or
/// past an edit's old end shifts with it.
pub(crate) struct ByteMap {
    /// Each edit's start and old end, by ascending start.
    edits: Vec<(usize, usize)>,
    /// `shifts[i]` is the change in length the first `i` edits make.
    shifts: Vec<isize>,
}

impl ByteMap {
    pub(crate) fn map(&self, byte: usize) -> usize {
        let past = self.edits.partition_point(|&(_, old_end)| old_end <= byte);
        let byte = match self.edits.get(past) {
            Some(&(start, _)) if start < byte => start,
            _ => byte,
        };
        byte.checked_add_signed(self.shifts[past])
            .expect("a change moves no byte before the text's start")
    }
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
