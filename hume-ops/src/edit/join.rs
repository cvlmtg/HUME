//! `join-lines-select-spaces` — join lines inside each selection and select
//! the inserted spaces.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::lines::{line_break_char, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

use super::apply_edit;

/// Join lines inside each selection and select the inserted spaces.
///
/// For each selection:
/// - Single-line: join with the next line.
/// - Multi-line: join all lines in the range.
///
/// Each consecutive pair is joined by replacing the newline (and leading
/// whitespace of the next line) with a single space. Whitespace-only or empty
/// next lines produce no separator — the newline is simply removed.
///
/// After the join, every inserted space becomes a 1-char selection.
pub fn join_lines_select_spaces(text: BufferText, sels: SelectionSet) -> (BufferText, SelectionSet, ChangeSet) {
    // Fast path: no selection spans or reaches a joinable line pair.
    // Return unchanged to avoid resetting cursors (all on last line → no-op).
    let has_work = sels.iter_sorted().any(|sel| {
        let start = text.char_to_line(sel.start());
        let end = text.char_to_line(sel.end_inclusive(&text));
        start != end || start < text.last_content_line()
    });
    if !has_work {
        let mut b = ChangeSetBuilder::new(text.len_chars());
        b.retain_rest();
        return (text, sels, b.finish());
    }

    let mut space_positions: Vec<usize> = Vec::new();

    let (new_buf, fallback_sels, cs) = apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let start_line = text.char_to_line(sel.start());
        let mut end_line = text.char_to_line(sel.end_inclusive(text));
        if start_line == end_line {
            // Clamp to the last content line: a cursor there must not join
            // with the trailing structural-newline line — it would delete
            // the structural '\n' and panic in the changeset validator.
            end_line = (end_line + 1).min(text.last_content_line());
        }

        for line in start_line..end_line {
            let nl_pos = line_break_char(text, line);
            let next_start = line_end_exclusive(text, line);
            let next_end_excl = line_end_exclusive(text, line + 1);

            let content_start = {
                let mut p = next_start;
                while p < next_end_excl {
                    match text.char_at(p) {
                        Some(c) if c == ' ' || c == '\t' || c == '\r' => p += 1,
                        _ => break,
                    }
                }
                p
            };

            let is_blank = content_start >= next_end_excl.saturating_sub(1);

            b.retain(nl_pos.saturating_sub(b.old_pos()));
            b.delete(content_start - nl_pos);

            if !is_blank {
                b.insert(" ");
                space_positions.push(b.new_pos() - 1);
            }
        }

        new_sels.push(Selection::collapsed(b.new_pos().saturating_sub(1)));
    });

    // Result is the inserted spaces — the command's contract is "select the
    // separators so they can be adjusted." Selections on lines that didn't join
    // produce no space and are intentionally dropped; keeping them would scatter
    // cursors on untouched chars outside the edit. The empty case keeps the
    // original cursors only because a SelectionSet can't be empty, not as a
    // competing rule.
    let new_sel_set = if space_positions.is_empty() {
        fallback_sels
    } else {
        let sels: Vec<Selection> = space_positions
            .into_iter()
            .map(Selection::collapsed)
            .collect();
        SelectionSet::from_vec(sels, 0)
    };

    (new_buf, new_sel_set, cs)
}
