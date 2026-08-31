// `update_highlight_providers`'s bracket-match highlight (`ui.cursor.match`)
// resolves against the whole primary selection, nearest the head — the same
// rule `#` (goto-matching-pair) itself uses. See `hume_ops::pair::nearest_bracket`.

use super::*;
use hume_editing::selection::Selection;
use hume_engine::pipeline::RenderContext;

/// A `w`-motion-style selection ends on the whitespace following a bracket,
/// head on the space rather than the bracket itself. The highlight must
/// still resolve the bracket nearest the head within the selection, matching
/// what `#` would jump to — not go dark just because the head itself isn't
/// on a bracket.
#[test]
fn bracket_match_highlight_resolves_nearest_bracket_in_selection() {
    let mut ed = Editor::open(None, Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "(x) y".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());

    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    // "(x) y\n": '(' 0, 'x' 1, ')' 2, ' ' 3, 'y' 4, '\n' 5 — selection covers
    // ") " with the head on the space (3), same shape as a `w` landing.
    ed.state.panes.state[pid][bid].selections = SelectionSet::single(Selection::new(2, 3));

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let spans: Vec<(usize, usize, usize)> = ed.state.panes.render[pid]
        .highlights
        .bracket
        .read()
        .unwrap()
        .iter()
        .map(|&(line, start, end, _)| (line, start, end))
        .collect();
    assert_eq!(
        spans,
        vec![(0, 0, 1)],
        "highlights the '(' partner, not nothing"
    );
}
