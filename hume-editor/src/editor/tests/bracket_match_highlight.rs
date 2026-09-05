// The bracket-match highlight (`ui.cursor.match`) resolves against the whole
// primary selection, nearest the head — see `hume_ops::pair::matching_bracket`.

use super::*;
use hume_editing::selection::Selection;

/// A `w`-motion-style selection ends on the whitespace following a bracket,
/// head on the space rather than the bracket itself. The highlight must
/// still resolve the bracket nearest the head within the selection, matching
/// what `#` would jump to — not go dark just because the head itself isn't
/// on a bracket.
#[test]
fn bracket_match_highlight_resolves_nearest_bracket_in_selection() {
    let mut ed = Editor::open(None, Arc::new(|| {})).unwrap();
    type_text(&mut ed, "(x) y");

    let pid = ed.state.focused_pane_id;
    // "(x) y\n": '(' 0, 'x' 1, ')' 2, ' ' 3, 'y' 4, '\n' 5 — selection covers
    // ") " with the head on the space (3), same shape as a `w` landing.
    ed.set_current_selections(SelectionSet::single(Selection::new(2, 3)));

    render(&mut ed);

    let spans: Vec<(usize, usize, usize)> = pane_highlights(&ed, pid, |h| &h.bracket)
        .into_iter()
        .map(|(line, start, end, _)| (line, start, end))
        .collect();
    assert_eq!(
        spans,
        vec![(0, 0, 1)],
        "highlights the '(' partner, not nothing"
    );
}
