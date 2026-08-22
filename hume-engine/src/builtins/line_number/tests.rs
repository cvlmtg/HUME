use super::*;
use crate::providers::GutterRowCtx;
use crate::types::{EditorMode, RowKind, ScopeId};

const DEFAULT_SCOPE: ScopeId = ScopeId(0);
const SELECTED_SCOPE: ScopeId = ScopeId(1);

/// `LineNumberColumn` never reads `rope`, so an empty rope is fine
/// for every test here — only `primary_head_line` varies.
fn ctx(rope: &ropey::Rope, primary_head_line: usize) -> GutterRowCtx<'_> {
    GutterRowCtx {
        mode: EditorMode::Normal,
        primary_head_line,
        rope,
    }
}

#[test]
fn width_grows_with_line_count() {
    // width(max_line) must fit the 1-based line number max_line+1.
    // digit_count(n+1) + 1 pad.
    let lane = LineNumberColumn::new(DEFAULT_SCOPE, SELECTED_SCOPE);
    assert_eq!(lane.width(0), 2); // max line "1" → 1 digit + 1 pad
    assert_eq!(lane.width(8), 2); // max line "9" → 1 digit + 1 pad
    assert_eq!(lane.width(9), 3); // max line "10" → 2 digits + 1 pad
    assert_eq!(lane.width(10), 3); // max line "11" → 2 digits + 1 pad
    assert_eq!(lane.width(98), 3); // max line "99" → 2 digits + 1 pad
    assert_eq!(lane.width(99), 4); // max line "100" → 3 digits + 1 pad
    assert_eq!(lane.width(100), 4); // max line "101" → 3 digits + 1 pad
}

#[test]
fn absolute_line_numbers() {
    let lane =
        LineNumberColumn::with_style(LineNumberStyle::Absolute, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 4 }, &ctx(&rope, 0))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "5"); // 1-based
}

#[test]
fn hybrid_head_line_shows_absolute() {
    let lane = LineNumberColumn::with_style(LineNumberStyle::Hybrid, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    // Cursor is on line 2 (0-based).
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 2 }, &ctx(&rope, 2))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "3"); // absolute
    assert_eq!(cell.scope, SELECTED_SCOPE);
}

#[test]
fn hybrid_non_head_line_shows_relative() {
    let lane = LineNumberColumn::with_style(LineNumberStyle::Hybrid, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 5 }, &ctx(&rope, 2))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "3"); // |5-2| = 3
}

#[test]
fn wrap_rows_are_blank() {
    let lane = LineNumberColumn::new(DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(
            RowKind::Wrap {
                line_idx: 3,
                wrap_row: 1,
            },
            &ctx(&rope, 0),
        )
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), " "); // blank
}

#[test]
fn virtual_rows_are_blank() {
    let lane = LineNumberColumn::new(DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(
            RowKind::Virtual {
                provider_id: 0,
                anchor_line: 0,
            },
            &ctx(&rope, 0),
        )
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), " ");
}

#[test]
fn relative_line_numbers() {
    let lane =
        LineNumberColumn::with_style(LineNumberStyle::Relative, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    // Cursor at line 5 (0-based). Line 3 is distance 2, line 8 is distance 3.
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 3 }, &ctx(&rope, 5))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "2");
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 8 }, &ctx(&rope, 5))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "3");
}

#[test]
fn relative_head_line_shows_zero() {
    let lane =
        LineNumberColumn::with_style(LineNumberStyle::Relative, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 5 }, &ctx(&rope, 5))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "0");
}

#[test]
fn hybrid_line_below_head_shows_relative() {
    // Cursor at line 5, render line 2 (below in the file, higher index than cursor).
    let lane = LineNumberColumn::with_style(LineNumberStyle::Hybrid, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    let cell = lane
        .render_row_cells(RowKind::LineStart { line_idx: 2 }, &ctx(&rope, 5))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "3"); // |2-5| = 3
}

// ── LineNumberStyle::FromStr ──────────────────────────────────────────

#[test]
fn line_number_style_from_str_all_variants() {
    assert_eq!(
        "absolute".parse::<LineNumberStyle>().unwrap(),
        LineNumberStyle::Absolute
    );
    assert_eq!(
        "relative".parse::<LineNumberStyle>().unwrap(),
        LineNumberStyle::Relative
    );
    assert_eq!(
        "hybrid".parse::<LineNumberStyle>().unwrap(),
        LineNumberStyle::Hybrid
    );
}

#[test]
fn line_number_style_from_str_case_insensitive() {
    assert_eq!(
        "Absolute".parse::<LineNumberStyle>().unwrap(),
        LineNumberStyle::Absolute
    );
    assert_eq!(
        "RELATIVE".parse::<LineNumberStyle>().unwrap(),
        LineNumberStyle::Relative
    );
    assert_eq!(
        "Hybrid".parse::<LineNumberStyle>().unwrap(),
        LineNumberStyle::Hybrid
    );
}

#[test]
fn line_number_style_from_str_error() {
    let err = "invalid".parse::<LineNumberStyle>().unwrap_err();
    assert!(err.contains("invalid"), "error should mention input: {err}");
    assert!(
        err.contains("absolute"),
        "error should list valid values: {err}"
    );
}

#[test]
fn line_number_style_values_round_trip_through_from_str() {
    // Independent-oracle guard: every completion-offered value must
    // actually parse, so `VALUES` can't silently drift from `FromStr`.
    // One-directional: this can't catch a variant added to `FromStr` but
    // left out of `VALUES` (it would just silently vanish from
    // completion) — `line_number_style_from_str_all_variants` above is
    // the closest thing to a reverse check, but it's a second
    // hand-maintained list, not a derived one.
    for v in LineNumberStyle::VALUES {
        assert!(
            v.parse::<LineNumberStyle>().is_ok(),
            "'{v}' should parse as LineNumberStyle"
        );
    }
}

#[test]
fn line_number_style_display_round_trips_through_from_str() {
    for v in LineNumberStyle::VALUES {
        let parsed: LineNumberStyle = v.parse().unwrap();
        assert_eq!(&parsed.to_string(), v);
    }
}

#[test]
fn digit_count_zero_is_one() {
    assert_eq!(LineNumberColumn::digit_count(0), 1);
}

#[test]
fn large_line_number_renders_correctly() {
    let lane =
        LineNumberColumn::with_style(LineNumberStyle::Absolute, DEFAULT_SCOPE, SELECTED_SCOPE);
    let rope = ropey::Rope::new();
    // line_idx = 9_999_998 → display = 9_999_999 (1-based)
    let cell = lane
        .render_row_cells(
            RowKind::LineStart {
                line_idx: 9_999_998,
            },
            &ctx(&rope, 0),
        )
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(cell.as_str(), "9999999");
}
