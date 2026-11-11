use ropey::Rope;

use super::*;
use crate::display_lines::line_store::PaneLineStore;
use crate::folds::{Fold, HiddenLines};
use crate::pane::WrapMode;
use crate::pipeline::PaneSize;
use crate::providers::ProviderSet;
use crate::test_doubles::{ScopeTable, VirtualLineBlock};
use crate::test_support::{map, map_hiding};
use hume_core::Side;

const SHOWN: ContextSettings = ContextSettings {
    enabled: true,
    outer_level: false,
};

const OUTER_LEVEL: ContextSettings = ContextSettings {
    enabled: true,
    outer_level: true,
};

/// A document of `n` one-character lines.
fn doc(n: usize) -> Rope {
    Rope::from_str(&"x\n".repeat(n))
}

fn top(line: usize) -> DisplayLinePos {
    DisplayLinePos::new(ContentLine::new(line), 0)
}

/// The rows of a pane `height` rows tall laid out by `dlm`, scroll-margin
/// 0, whose top is `top` and whose headers come from `inputs`.
fn rows_of(
    dlm: &mut DisplayLineMap<'_>,
    top: DisplayLinePos,
    height: u16,
    inputs: ScopeHeaderInputs<'_>,
) -> PaneRows {
    let geo = PaneSize::new(80, height)
        .geometry(0, inputs)
        .expect("a nonzero-height pane has a geometry");
    PaneRows::walk(dlm, &geo, top)
}

/// The header lines a pane `height` rows tall with `top` at its top shows.
fn walk(
    rope: &Rope,
    providers: &ProviderSet,
    table: &ScopeTable,
    settings: ContextSettings,
    top: DisplayLinePos,
    height: u16,
) -> Vec<usize> {
    let mut store = PaneLineStore::new();
    let mut dlm = map(rope, WrapMode::None, providers, 80, &mut store);
    let inputs = ScopeHeaderInputs {
        source: Some(table),
        settings,
    };
    rows_of(&mut dlm, top, height, inputs)
        .headers()
        .iter()
        .map(|line| line.index())
        .collect()
}

#[test]
fn push_up_drops_a_scope_when_its_closing_line_reaches_the_header_row() {
    let rope = doc(12);
    let table = ScopeTable::default()
        .lines(2..=5, &[0, 1])
        .lines(6..=9, &[0]);
    let providers = ProviderSet::new();
    assert_eq!(walk(&rope, &providers, &table, SHOWN, top(4), 24), [0, 1]);
    assert_eq!(walk(&rope, &providers, &table, SHOWN, top(5), 24), [0]);
}

#[test]
fn cap_drops_outermost_by_default_and_second_with_outer_level() {
    let rope = doc(30);
    let providers = ProviderSet::new();
    let three = ScopeTable::default().lines(3..=20, &[0, 1, 2]);
    assert_eq!(walk(&rope, &providers, &three, SHOWN, top(5), 5), [1, 2]);
    assert_eq!(
        walk(&rope, &providers, &three, OUTER_LEVEL, top(5), 5),
        [0, 2]
    );

    let four = ScopeTable::default().lines(4..=20, &[0, 1, 2, 3]);
    assert_eq!(walk(&rope, &providers, &four, SHOWN, top(5), 24), [1, 2, 3]);
    assert_eq!(
        walk(&rope, &providers, &four, OUTER_LEVEL, top(5), 24),
        [0, 2, 3]
    );
}

#[test]
fn height_one_shows_no_header() {
    let rope = doc(30);
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(3..=20, &[0, 1, 2]);
    assert_eq!(
        walk(&rope, &providers, &table, SHOWN, top(5), 1),
        Vec::<usize>::new()
    );
}

#[test]
fn no_header_without_a_line_under_it() {
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(1..=2, &[0]);
    assert_eq!(walk(&doc(3), &providers, &table, SHOWN, top(1), 24), [0]);
    assert_eq!(
        walk(&doc(2), &providers, &table, SHOWN, top(1), 24),
        Vec::<usize>::new()
    );
}

#[test]
fn virtual_display_line_row_belongs_to_its_line() {
    let rope = doc(10);
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        Side::Before,
        ContentLine::new(4),
        1,
        "V",
    )));
    let table = ScopeTable::default().lines(4..=4, &[0]);
    assert_eq!(walk(&rope, &providers, &table, SHOWN, top(4), 24), [0]);
}

/// Line 4 wraps over three rows at width 10, so the first two header rows
/// both lie over it and its scopes are read once.
#[test]
fn a_line_under_several_header_rows_is_queried_once() {
    let rope = Rope::from_str(&format!("x\nx\nx\nx\n{}\nx\nx\n", "y".repeat(25)));
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(2..=5, &[0, 1]);
    let mut store = PaneLineStore::new();
    let mut dlm = map(
        &rope,
        WrapMode::Soft { width: 10 },
        &providers,
        10,
        &mut store,
    );
    let inputs = ScopeHeaderInputs {
        source: Some(&table),
        settings: SHOWN,
    };
    let rows = rows_of(&mut dlm, top(4), 24, inputs);
    assert_eq!(rows.headers(), [ContentLine::new(0), ContentLine::new(1)]);
    assert_eq!(table.header_queries(), 1);
}

/// A 5-row pane whose top line 4 wraps over three rows at width 10, under
/// two headers: rows 0 and 1 draw the headers over line 4's first two
/// display lines, rows 2 to 4 show line 4's third display line, line 5 and
/// line 6.
#[test]
fn rows_map_header_rows_to_their_lines_and_the_rest_to_display_lines() {
    let rope = Rope::from_str(&format!(
        "x\nx\nx\nx\n{}\n{}",
        "y".repeat(25),
        "x\n".repeat(5)
    ));
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(2..=8, &[0, 1]);
    let mut store = PaneLineStore::new();
    let mut dlm = map(
        &rope,
        WrapMode::Soft { width: 10 },
        &providers,
        10,
        &mut store,
    );
    let inputs = ScopeHeaderInputs {
        source: Some(&table),
        settings: SHOWN,
    };
    let rows = rows_of(&mut dlm, top(4), 5, inputs);
    let at = |line: usize, slot: usize| DisplayLinePos::new(ContentLine::new(line), slot);

    assert_eq!(rows.headers(), [ContentLine::new(0), ContentLine::new(1)]);
    assert_eq!(rows.body_top(&mut dlm), at(4, 2));
    assert_eq!(
        (0..5)
            .map(|row| rows.row_at(&mut dlm, row))
            .collect::<Vec<_>>(),
        [
            PaneRow::Header(ContentLine::new(0)),
            PaneRow::Header(ContentLine::new(1)),
            PaneRow::Body(at(4, 2)),
            PaneRow::Body(at(5, 0)),
            PaneRow::Body(at(6, 0)),
        ]
    );

    assert_eq!(rows.row_of(&mut dlm, at(3, 0)), None, "above the top");
    assert_eq!(rows.row_of(&mut dlm, at(4, 1)), None, "under header row 1");
    assert_eq!(rows.row_of(&mut dlm, at(4, 2)), Some(2));
    assert_eq!(rows.row_of(&mut dlm, at(6, 0)), Some(4));
    assert_eq!(rows.row_of(&mut dlm, at(7, 0)), None, "past the last row");
}

#[test]
fn disabled_or_sourceless_yields_nothing() {
    let rope = doc(30);
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(3..=20, &[0, 1]);

    let mut store = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, 80, &mut store);
    let disabled = ScopeHeaderInputs {
        source: Some(&table),
        settings: ContextSettings::OFF,
    };
    assert_eq!(rows_of(&mut dlm, top(5), 24, disabled).headers(), []);
    let sourceless = ScopeHeaderInputs {
        source: None,
        settings: SHOWN,
    };
    assert_eq!(rows_of(&mut dlm, top(5), 24, sourceless).headers(), []);
}

#[test]
fn a_header_line_a_fold_hides_is_not_shown() {
    let rope = doc(30);
    let table = ScopeTable::default().lines(11..=20, &[0, 5]);
    let providers = ProviderSet::new();
    let fold =
        Fold::new(ContentLine::new(1), ContentLine::new(10), false).expect("the fold hides a line");
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        HiddenLines::from_folds([fold]),
        &mut store,
    );
    let inputs = ScopeHeaderInputs {
        source: Some(&table),
        settings: SHOWN,
    };
    let rows = rows_of(&mut dlm, top(11), 24, inputs);
    assert_eq!(rows.headers(), [ContentLine::new(0)]);
}
