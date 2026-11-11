//! A map with closed folds: hidden lines have no display line.

use super::*;
use crate::folds::{Fold, HiddenLines};
use crate::pipeline::PaneSize;
use crate::test_support::{map_hiding, plain_geo};

/// Closed folds over `first ..= last`, each hiding through its last line.
fn hiding(folds: &[(usize, usize)]) -> HiddenLines {
    HiddenLines::from_folds(folds.iter().map(|&(first, last)| {
        Fold::new(ContentLine::new(first), ContentLine::new(last), false)
            .expect("each fold hides a line")
    }))
}

fn pos(line: usize, slot: usize) -> DisplayLinePos {
    DisplayLinePos::new(ContentLine::new(line), slot)
}

/// Every display line, first to last.
fn walk_forward(dlm: &mut DisplayLineMap<'_>) -> Vec<DisplayLinePos> {
    let mut positions = vec![DisplayLinePos::default()];
    while let Some(next) = dlm.next(*positions.last().expect("seeded above")) {
        positions.push(next);
    }
    positions
}

fn lines_of(positions: &[DisplayLinePos]) -> Vec<usize> {
    let mut lines: Vec<usize> = positions.iter().map(|p| p.line.index()).collect();
    lines.dedup();
    lines
}

#[test]
fn stepping_skips_hidden_lines_both_ways() {
    let rope = Rope::from_str(&"abcdefgh\n".repeat(10));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        Side::After,
        ContentLine::new(2),
        2,
        "V",
    )));
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        Side::Before,
        ContentLine::new(4),
        1,
        "V",
    )));
    for (wrap, width) in [(WrapMode::None, 80), (WrapMode::Soft { width: 0 }, 4)] {
        let mut store = PaneLineStore::new();
        let mut dlm = map_hiding(
            &rope,
            wrap,
            &providers,
            width,
            hiding(&[(2, 5)]),
            &mut store,
        );
        let forward = walk_forward(&mut dlm);
        assert_eq!(lines_of(&forward), [0, 1, 2, 6, 7, 8, 9], "{wrap:?}");
        let fold_line = forward.iter().filter(|p| p.line.index() == 2).count();
        assert_eq!(
            fold_line,
            dlm.block(ContentLine::new(2)).total(),
            "{wrap:?}"
        );
        for pair in forward.windows(2) {
            assert_eq!(dlm.prev(pair[1]), Some(pair[0]), "{wrap:?}");
        }
    }
}

#[test]
fn a_fold_hiding_the_last_lines_ends_the_document() {
    let rope = Rope::from_str(&"x\n".repeat(8));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        hiding(&[(5, 7)]),
        &mut store,
    );
    assert_eq!(dlm.next(pos(5, 0)), None);
    assert_eq!(lines_of(&walk_forward(&mut dlm)), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn distance_crosses_a_fold_longer_than_its_cap() {
    let rope = Rope::from_str(&"x\n".repeat(30));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        hiding(&[(1, 25)]),
        &mut store,
    );
    assert_eq!(dlm.distance(pos(0, 0), pos(27, 0), 5), Some(3));
    assert_eq!(dlm.distance(pos(0, 0), pos(29, 0), 3), None);
}

#[test]
fn scroll_by_crosses_a_fold_longer_than_the_pane() {
    let rope = Rope::from_str(&"x\n".repeat(40));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        hiding(&[(1, 30)]),
        &mut store,
    );
    let geo = plain_geo(PaneSize::new(80, 5), 0);
    let mut viewport = crate::pane::Viewport::default();
    viewport.scroll_by(&mut dlm, geo, 2);
    assert_eq!(viewport.top(), pos(31, 0));
}

#[test]
fn max_scroll_top_measures_back_from_the_last_shown_line() {
    let rope = Rope::from_str(&"x\n".repeat(20));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        hiding(&[(10, 19)]),
        &mut store,
    );
    let geo = plain_geo(PaneSize::new(80, 4), 0);
    let expected = 10 - geo.target;
    assert_eq!(dlm.max_scroll_top(geo), pos(expected, 0));
}

#[test]
fn clamp_moves_a_hidden_address_to_its_fold_line() {
    let rope = Rope::from_str(&"x\n".repeat(10));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        hiding(&[(2, 6)]),
        &mut store,
    );
    assert_eq!(dlm.clamp(pos(4, 3)), pos(2, 0));
    assert_eq!(dlm.clamp(pos(7, 0)), pos(7, 0));
}

#[test]
fn a_position_on_a_hidden_line_locates_at_its_fold_line() {
    let rope = Rope::from_str("aaa\nbbb\nccc\nddd\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        Side::Before,
        ContentLine::new(1),
        1,
        "V",
    )));
    for wrap in [WrapMode::None, WrapMode::Soft { width: 0 }] {
        let mut store = PaneLineStore::new();
        let mut dlm = map_hiding(&rope, wrap, &providers, 80, hiding(&[(1, 3)]), &mut store);
        let in_fold = at(&rope, 9);
        assert_eq!(dlm.locate(in_fold), (pos(1, 1), dc(0)), "{wrap:?}");
        assert_eq!(dlm.locate_display_line(in_fold), pos(1, 1), "{wrap:?}");
        assert_eq!(
            dlm.first_content_display_line(ContentLine::new(3)),
            pos(1, 1),
            "{wrap:?}"
        );
    }
}

#[test]
fn carry_steps_over_a_fold() {
    let rope = Rope::from_str(&"x\n".repeat(20));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();
    let mut dlm = map_hiding(
        &rope,
        WrapMode::None,
        &providers,
        80,
        hiding(&[(1, 5)]),
        &mut store,
    );
    let geo = plain_geo(PaneSize::new(80, 10), 0);
    let top = geo.carry_top(&mut dlm, pos(0, 0));
    assert_eq!(carry(&mut dlm, geo, top, pos(0, 0), 2), Some(pos(6, 0)));
}
