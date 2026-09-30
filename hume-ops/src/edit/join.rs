//! `join-lines-select-spaces`: join lines inside each selection and select
//! the inserted spaces.

use hume_editing::edit::Edited;
use hume_editing::edit::{Landing, Landings, edit};
use hume_editing::lines::{leading_whitespace_end, line_break};
use hume_editing::selection::Facing;
use hume_editing::state::EditState;
use hume_rope::cluster::ClusterRange;
use hume_rope::line::ContentLine;

/// Join lines inside each selection and select the inserted spaces.
///
/// For each selection:
/// - Single-line: join with the next line.
/// - Multi-line: join all lines in the range.
///
/// Each consecutive pair is joined by replacing the newline (and leading
/// whitespace of the next line) with a single space. Whitespace-only or empty
/// next lines produce no separator; the newline is simply removed.
///
/// After the join, every inserted space becomes a selection of the cluster
/// it lands in.
pub fn join_lines_select_spaces(state: EditState) -> Edited {
    let view = state.view();
    let last_content_line = state.text().last_content_line();
    // No selection spans or reaches a joinable line pair (all on the last
    // line): nothing changes, cursors included.
    let has_work = view.iter().any(|sel| {
        let lines = sel.lines();
        lines.start != lines.end || lines.start < last_content_line
    });
    if !has_work {
        return Edited::unchanged(state);
    }

    let text = state.text();
    let primary = view.primary().index();
    edit(&state, |b| {
        let mut spaces = Vec::new();
        let mut fallback = Vec::new();
        // The first line not yet joined: selections and the lines each spans
        // ascend, so a line an earlier selection joined is skipped.
        let mut next_unjoined = 0;

        for sel in view.iter() {
            let lines = sel.lines();
            // A cursor on the last content line has no next line to join: the
            // structural '\n' stays.
            let end_line = if lines.start == lines.end {
                lines.end.advance(1).min(last_content_line)
            } else {
                lines.end
            };

            let mut last_deletion = None;
            // Bare-`usize` range, `ContentLine` re-minted each iteration:
            // `ContentLine` has no `Step`/`Range` impl to loop over directly
            // (see CLAUDE.md's "Line counts and ranges"). Sound here: both
            // endpoints are already-valid `ContentLine`s.
            for line_idx in lines.start.index().max(next_unjoined)..end_line.index() {
                let line = ContentLine::new(line_idx);
                let nl_pos = line_break(text, line);
                let content_start = leading_whitespace_end(text, line.advance(1));
                let is_blank = content_start >= line_break(text, line.advance(1));
                let joined = ClusterRange::between(text.full_slice(), nl_pos, content_start.into())
                    .expect("a line's break comes before the next line's content");

                last_deletion = Some(b.delete(joined));
                if !is_blank {
                    let mark = b.insert(nl_pos, " ");
                    spaces.push(Landing::covering(mark, Facing::Forward));
                }
                next_unjoined = line_idx + 1;
            }
            // With no space inserted, every later line of the run was blank and
            // went whole, so the cursor takes the last cluster of the joined
            // line before the join point, or its break when that line is empty.
            fallback.push(match last_deletion {
                Some(at) => Landing::cursor_ending_at(at),
                None => Landing::kept(sel.selection()),
            });
        }

        // The result is the inserted spaces, so they can be adjusted;
        // selections whose lines did not join leave none and are dropped. With
        // no space at all, the original selections' results stand in, since a
        // selection set is never empty.
        if spaces.is_empty() {
            Landings::new(fallback, primary)
        } else {
            Landings::new(spaces, 0)
        }
    })
}
