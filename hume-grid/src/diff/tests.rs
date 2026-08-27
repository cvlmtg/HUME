use super::*;
use crate::color::Rgb;
use crate::style::ResolvedStyle;

const WIDE: &str = "コ";
const WIDE2: &str = "ナ";

fn red() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(Rgb(255, 0, 0)),
        ..Default::default()
    }
}

fn blue() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(Rgb(0, 0, 255)),
        ..Default::default()
    }
}

/// `(y, x, rendered text of each cell)` for every run, with `_` for a
/// continuation — a compact, readable form to assert against.
fn runs_of(next: &Grid, prev: &Grid, max_gap: u16) -> Vec<(u16, u16, String)> {
    next.diff_runs(prev, max_gap)
        .map(|r| {
            let text = r
                .cells
                .iter()
                .map(|c| if c.is_continuation() { "_" } else { c.text() })
                .collect();
            (r.y, r.x, text)
        })
        .collect()
}

#[test]
fn identical_grids_produce_no_runs() {
    let g = Grid::new(6, 2);
    assert!(runs_of(&g, &g.clone(), 4).is_empty());
}

#[test]
fn a_single_changed_cell_is_one_run() {
    let prev = Grid::new(6, 1);
    let mut next = prev.clone();
    next.set_glyph(2, 0, "x", 1, red());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 2, "x".to_string())]);
}

#[test]
fn a_style_only_change_is_still_a_change() {
    let mut prev = Grid::new(3, 1);
    prev.set_glyph(0, 0, "a", 1, red());
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, blue());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 0, "a".to_string())]);
}

#[test]
fn adjacent_changes_merge_into_one_run() {
    let prev = Grid::new(6, 1);
    let mut next = prev.clone();
    next.set_glyph(1, 0, "a", 1, red());
    next.set_glyph(2, 0, "b", 1, red());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 1, "ab".to_string())]);
}

#[test]
fn each_row_is_scanned_separately() {
    let prev = Grid::new(4, 2);
    let mut next = prev.clone();
    next.set_glyph(1, 0, "a", 1, red());
    next.set_glyph(2, 1, "b", 1, red());
    assert_eq!(
        runs_of(&next, &prev, 4),
        [(0, 1, "a".to_string()), (1, 2, "b".to_string())]
    );
}

#[test]
fn narrow_replaced_by_wide_emits_both_columns() {
    let mut prev = Grid::new(5, 1);
    prev.set_glyph(1, 0, "a", 1, red());
    prev.set_glyph(2, 0, "b", 1, red());
    let mut next = prev.clone();
    next.set_glyph(1, 0, WIDE, 2, red());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 1, format!("{WIDE}_"))]);
}

#[test]
fn wide_replaced_by_narrow_emits_both_columns() {
    let mut prev = Grid::new(5, 1);
    prev.set_glyph(1, 0, WIDE, 2, red());
    let mut next = prev.clone();
    next.set_glyph(1, 0, "a", 1, red());
    next.set_glyph(2, 0, "b", 1, red());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 1, "ab".to_string())]);
}

#[test]
fn swapping_a_wide_glyph_for_another_emits_only_its_head() {
    // The continuation is identical in both frames (same style, no text), so
    // it is not a change. Printing the head covers both columns anyway — the
    // emitter advances by the head's own width, not by the run's length.
    let mut prev = Grid::new(5, 1);
    prev.set_glyph(1, 0, WIDE, 2, red());
    let mut next = prev.clone();
    next.set_glyph(1, 0, WIDE2, 2, red());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 1, WIDE2.to_string())]);
}

#[test]
fn a_run_never_starts_at_a_continuation() {
    // A wide glyph shifting one column right: every candidate start must
    // still be a head, or a repaint would begin mid-glyph.
    let mut prev = Grid::new(6, 1);
    prev.set_glyph(1, 0, WIDE, 2, red());
    let mut next = Grid::new(6, 1);
    next.set_glyph(2, 0, WIDE, 2, red());
    for run in next.diff_runs(&prev, 4) {
        assert!(
            !run.cells[0].is_continuation(),
            "run at ({}, {}) starts on a continuation",
            run.x,
            run.y
        );
    }
}

#[test]
fn a_gap_within_the_budget_is_merged() {
    // Changes at 0 and 5 leave four unchanged cells between them.
    let prev = Grid::new(8, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(5, 0, "b", 1, red());
    assert_eq!(runs_of(&next, &prev, 4), [(0, 0, "a    b".to_string())]);
}

#[test]
fn a_gap_past_the_budget_splits_the_run() {
    // Changes at 0 and 6 leave five unchanged cells — one too many.
    let prev = Grid::new(8, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(6, 0, "b", 1, red());
    assert_eq!(
        runs_of(&next, &prev, 4),
        [(0, 0, "a".to_string()), (0, 6, "b".to_string())]
    );
}

#[test]
fn a_zero_budget_never_merges() {
    let prev = Grid::new(5, 1);
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(2, 0, "b", 1, red());
    assert_eq!(
        runs_of(&next, &prev, 0),
        [(0, 0, "a".to_string()), (0, 2, "b".to_string())]
    );
}

#[test]
fn a_gap_merge_can_span_a_whole_glyph() {
    let mut prev = Grid::new(8, 1);
    prev.set_glyph(2, 0, WIDE, 2, red());
    let mut next = prev.clone();
    next.set_glyph(0, 0, "a", 1, red());
    next.set_glyph(5, 0, "b", 1, red());
    // The untouched wide glyph is re-printed inside the merged run, head
    // and continuation together.
    assert_eq!(runs_of(&next, &prev, 4), [(0, 0, format!("a {WIDE}_ b"))]);
}

// ── Property-based test (proptest) ───────────────────────────────────────
//
// Oracle: replay the diff's runs onto a copy of the previous frame the way a
// terminal would — draw each non-continuation cell's text at the cursor, then
// advance by that glyph's own width — and require the result to equal the
// next frame exactly. This checks completeness (nothing changed was left
// out) and glyph-boundary safety (no run starts or lands mid-glyph) without
// restating the algorithm the diff uses to find them.

use proptest::prelude::*;

const W: u16 = 8;
const H: u16 = 3;

/// Narrow and wide, single- and multi-byte — enough for cells to collide on
/// equality often, which is what makes gaps and unchanged runs appear.
const TEXTS: [(&str, u8); 4] = [("a", 1), ("é", 1), (WIDE, 2), (WIDE2, 2)];

fn palette(i: usize) -> ResolvedStyle {
    const COLORS: [Rgb; 3] = [Rgb(255, 0, 0), Rgb(0, 255, 0), Rgb(0, 0, 255)];
    ResolvedStyle {
        fg: Some(COLORS[i % COLORS.len()]),
        ..Default::default()
    }
}

#[derive(Clone, Debug)]
enum Op {
    Glyph {
        x: u16,
        y: u16,
        text: usize,
        style: usize,
    },
    Fill {
        y: u16,
        a: u16,
        b: u16,
        style: usize,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..W, 0..H, 0..TEXTS.len(), 0..3usize).prop_map(|(x, y, text, style)| Op::Glyph {
            x,
            y,
            text,
            style
        }),
        (0..H, 0..W, 0..W, 0..3usize).prop_map(|(y, a, b, style)| Op::Fill { y, a, b, style }),
    ]
}

fn apply(grid: &mut Grid, ops: &[Op]) {
    for o in ops {
        match *o {
            Op::Glyph { x, y, text, style } => {
                let (t, advance) = TEXTS[text];
                grid.set_glyph(x, y, t, advance, palette(style));
            }
            Op::Fill { y, a, b, style } => {
                grid.fill_span(y, a.min(b), a.max(b), Cell::blank(palette(style)));
            }
        }
    }
}

proptest! {
    #[test]
    fn replaying_the_runs_reconstructs_the_next_frame(
        first in prop::collection::vec(op(), 0..12),
        second in prop::collection::vec(op(), 0..12),
        max_gap in 0u16..6,
    ) {
        let mut prev = Grid::new(W, H);
        apply(&mut prev, &first);
        let mut next = prev.clone();
        apply(&mut next, &second);

        let runs: Vec<(u16, u16, Vec<Cell>)> = next
            .diff_runs(&prev, max_gap)
            .map(|r| (r.y, r.x, r.cells.to_vec()))
            .collect();

        let mut applied = prev.clone();
        for (y, x, cells) in &runs {
            let mut cx = *x;
            for cell in cells {
                // A continuation is already covered by the glyph before it —
                // the terminal drew both columns at once.
                if cell.is_continuation() {
                    continue;
                }
                applied.set_glyph(cx, *y, cell.text(), cell.advance() as u8, cell.style());
                cx += cell.advance();
            }
        }

        prop_assert_eq!(applied, next);
    }
}
