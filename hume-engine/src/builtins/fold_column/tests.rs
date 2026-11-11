use super::*;
use crate::folds::{Fold, HiddenLines};
use crate::providers::{GutterCellContent, GutterCtx, GutterInputs, GutterRow};
use crate::test_doubles::ScopeTable;
use crate::types::{DisplayLineKind, EditorMode, ScopeId};
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::offset::InclusiveRange;

const DEFAULT_SCOPE: ScopeId = ScopeId(0);
const SELECTED_SCOPE: ScopeId = ScopeId(1);

fn shown() -> GutterInputs {
    GutterInputs {
        folds: true,
        ..GutterInputs::default()
    }
}

fn ctx(rope: &ropey::Rope, fold: FoldMark, row: GutterRow) -> GutterCtx<'_> {
    GutterCtx {
        mode: EditorMode::Normal,
        row,
        rope,
        inputs: shown(),
        fold,
    }
}

fn body(primary_head_line: usize) -> GutterRow {
    GutterRow::Body {
        primary_head_line: ContentLine::new(primary_head_line),
    }
}

fn line_start(line: usize) -> DisplayLineKind {
    DisplayLineKind::LineStart {
        line_idx: RopeyLine::new(line),
    }
}

fn cell(kind: DisplayLineKind, fold: FoldMark, row: GutterRow) -> GutterCell {
    let rope = ropey::Rope::new();
    let mut cells =
        FoldColumn::new(DEFAULT_SCOPE, SELECTED_SCOPE).render_cells(kind, &ctx(&rope, fold, row));
    assert_eq!(cells.len(), 1);
    cells.remove(0)
}

#[test]
fn the_column_takes_room_only_while_shown() {
    let lane = FoldColumn::new(DEFAULT_SCOPE, SELECTED_SCOPE);
    assert_eq!(lane.width(RopeyLine::new(99), shown()), 2);
    assert_eq!(lane.width(RopeyLine::new(99), GutterInputs::default()), 0);
}

#[test]
fn a_line_start_draws_its_fold_mark() {
    let drawn = [FoldMark::Open, FoldMark::Closed, FoldMark::None]
        .map(|fold| cell(line_start(3), fold, body(0)).as_str().to_owned())
        .join("|");
    insta::assert_snapshot!(drawn, @"▾|▸| ");
}

#[test]
fn only_a_line_start_draws_a_mark() {
    let kinds = [
        DisplayLineKind::Wrap {
            line_idx: RopeyLine::new(3),
            wrap_index: 1,
        },
        DisplayLineKind::Virtual {
            provider_id: 0,
            anchor_line: RopeyLine::new(3),
        },
        DisplayLineKind::Filler,
    ];
    for kind in kinds {
        let content = cell(kind, FoldMark::Closed, body(0)).content;
        assert!(matches!(content, GutterCellContent::Blank), "{kind:?}");
    }
}

#[test]
fn the_primary_head_line_draws_selected() {
    assert_eq!(
        cell(line_start(3), FoldMark::Open, body(3)).scope,
        SELECTED_SCOPE
    );
    assert_eq!(
        cell(line_start(3), FoldMark::Open, body(2)).scope,
        DEFAULT_SCOPE
    );
    assert_eq!(
        cell(line_start(3), FoldMark::Open, GutterRow::Header).scope,
        DEFAULT_SCOPE
    );
}

#[test]
fn marks_come_from_the_closed_folds_and_the_shown_lines_read() {
    let rope = ropey::Rope::from_str(&"x\n".repeat(12));
    let line = ContentLine::new;
    let table = ScopeTable::default()
        .fold_start(2)
        .fold_start(4)
        .fold_start(8);
    let closed = Fold::new(line(2), line(6), false).expect("the fold hides a line");
    let hidden = HiddenLines::from_folds([closed]);
    let mut starts = FoldStarts::default();
    starts.reset(hidden.clone());
    starts.read([InclusiveRange::new(line(0), line(11))], &table, &rope);
    assert_eq!(starts.mark(line(2)), FoldMark::Closed);
    assert_eq!(starts.mark(line(8)), FoldMark::Open);
    assert_eq!(starts.mark(line(0)), FoldMark::None);
    assert_eq!(
        starts.mark(line(4)),
        FoldMark::None,
        "a region starting on a hidden line is not read"
    );

    let mut unread = FoldStarts::default();
    unread.reset(hidden);
    assert_eq!(unread.mark(line(2)), FoldMark::Closed);
    assert_eq!(unread.mark(line(8)), FoldMark::None);
}
