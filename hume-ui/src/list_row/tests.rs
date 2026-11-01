use std::sync::Arc;

use hume_core::TruncateEnd;
use hume_engine::render::Canvas;
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::ResolvedStyle;
use hume_grid::{Grid, Rect};

use super::{ListRow, paint_styled_row};
use crate::popup::StyledRow;
use crate::symbols_in;

fn style() -> ResolvedStyle {
    ResolvedStyle::default()
}

fn selected_row() -> ListRow {
    let scope = ScopeRegistry::new().intern("diagnostic.warning.message");
    ListRow::plain("abcdef".into()).with_selected_scope(Some(scope))
}

#[test]
fn a_cut_keeps_the_rows_selected_scope() {
    let row = selected_row();
    let scope = row.selected_scope;
    assert_eq!(row.fit(3, TruncateEnd::Tail).selected_scope, scope);
}

#[test]
fn an_append_keeps_the_rows_selected_scope() {
    let row = selected_row();
    let scope = row.selected_scope;
    assert_eq!(row.append("gh").selected_scope, scope);
}

#[test]
fn a_shared_fit_shares_a_row_that_fits_and_cuts_one_that_does_not() {
    let row = Arc::new(selected_row());

    let fits = row.fit_shared(6, TruncateEnd::Tail);
    assert!(Arc::ptr_eq(&fits, &row));

    let cut = row.fit_shared(3, TruncateEnd::Tail);
    assert_eq!(*cut, selected_row().fit(3, TruncateEnd::Tail));
}

#[test]
fn styled_runs_stay_adjacent_when_a_run_holds_an_undrawable_grapheme() {
    // A zero-width space draws as nothing, so writing it into the cell
    // reserved for it would leave the terminal's cursor where it was and
    // slide the rest of the row one column left. It renders as its codepoint
    // instead (the same substitution buffer text gets), and the next run
    // begins after the whole placeholder.
    let mut buf = Grid::new(20, 3);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let runs: StyledRow = vec![
        ("a\u{200B}b".to_string(), style()),
        ("cd".to_string(), style()),
    ];
    paint_styled_row(
        &mut canvas,
        0,
        1,
        runs.iter().map(|(t, s)| (t.as_str(), *s)),
        20,
    );

    // The placeholder spans the columns reserved for it, and the second run
    // starts in the cell right after it.
    insta::assert_snapshot!(symbols_in(&buf, Rect::new(0, 1, 12, 1)), @"a<200b>bcd");
}

#[test]
fn styled_runs_stop_at_the_right_edge() {
    // Runs are bounded by the caller's edge, not by the terminal buffer's: a
    // row wider than its box must be clipped at the border rather than
    // written over it. A cluster that would straddle the edge is dropped
    // whole, never half-drawn.
    let mut buf = Grid::new(20, 3);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let runs: StyledRow = vec![
        ("abc".to_string(), style()),
        ("\u{6F22}z".to_string(), style()),
    ];
    // Edge at 4: "abc" fills 0..3, and 漢 would need cells 3 and 4, so it is
    // dropped, leaving 'z' nowhere to start from either.
    paint_styled_row(
        &mut canvas,
        0,
        1,
        runs.iter().map(|(t, s)| (t.as_str(), *s)),
        4,
    );

    insta::assert_snapshot!(symbols_in(&buf, Rect::new(0, 1, 8, 1)), @"abc");
}
