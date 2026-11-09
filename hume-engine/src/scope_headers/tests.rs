use ropey::Rope;

use super::*;
use crate::display_lines::line_store::PaneLineStore;
use crate::pane::WrapMode;
use crate::providers::ProviderSet;
use crate::test_doubles::{ScopeTable, VirtualLineBlock};
use crate::test_support::map;
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

/// The header lines a pane `height` rows tall with `top` at its top shows.
fn walk(
    rope: &Rope,
    providers: &ProviderSet,
    table: &ScopeTable,
    settings: ContextSettings,
    top: DisplayLinePos,
    height: usize,
) -> Vec<usize> {
    let mut store = PaneLineStore::new();
    let mut dlm = map(rope, WrapMode::None, providers, 80, &mut store);
    let mut out = Vec::new();
    scope_headers(
        &mut dlm,
        top,
        height,
        ScopeHeaderInputs {
            source: Some(table),
            settings,
        },
        &mut out,
    );
    out.iter().map(|line| line.index()).collect()
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
    assert_eq!(walk(&rope, &providers, &three, SHOWN, top(5), 3), [1, 2]);
    assert_eq!(
        walk(&rope, &providers, &three, OUTER_LEVEL, top(5), 3),
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
fn height_one_shows_none_and_height_three_at_most_two() {
    let rope = doc(30);
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(3..=20, &[0, 1, 2]);
    assert_eq!(
        walk(&rope, &providers, &table, SHOWN, top(5), 1),
        Vec::<usize>::new()
    );
    assert_eq!(walk(&rope, &providers, &table, SHOWN, top(5), 3).len(), 2);
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

#[test]
fn disabled_or_sourceless_yields_nothing() {
    let rope = doc(30);
    let providers = ProviderSet::new();
    let table = ScopeTable::default().lines(3..=20, &[0, 1]);

    let mut store = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, 80, &mut store);
    let mut out = vec![ContentLine::new(7)];
    scope_headers(
        &mut dlm,
        top(5),
        24,
        ScopeHeaderInputs {
            source: Some(&table),
            settings: ContextSettings::OFF,
        },
        &mut out,
    );
    assert!(out.is_empty());

    out.push(ContentLine::new(7));
    scope_headers(
        &mut dlm,
        top(5),
        24,
        ScopeHeaderInputs {
            source: None,
            settings: SHOWN,
        },
        &mut out,
    );
    assert!(out.is_empty());
}

#[test]
fn header_display_line_is_the_lines_first_content_display_line() {
    let rope = doc(10);
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        Side::Before,
        ContentLine::new(4),
        2,
        "V",
    )));
    let mut store = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, 80, &mut store);
    assert_eq!(
        header_display_line(&mut dlm, ContentLine::new(4)),
        DisplayLinePos::new(ContentLine::new(4), 2)
    );
    assert_eq!(
        header_display_line(&mut dlm, ContentLine::new(3)),
        DisplayLinePos::new(ContentLine::new(3), 0)
    );
}
