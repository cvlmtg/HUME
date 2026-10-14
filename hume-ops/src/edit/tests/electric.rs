use super::super::*;
use hume_editing::changeset::Assoc;
use hume_editing::indent::ElectricTrigger;
use hume_editing::selection::{EditView, SelectionSet};
use hume_editing::tab_style::TabStyle;
use hume_editing::text::BufferText;
use hume_rope::column::BufferLineCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use pretty_assertions::assert_eq;
use test_fixtures::testing::{IntoTestResult, parse_state, serialize_state, state};

/// Each selection's record: its line's start, the line's whitespace up to
/// `ws_len` chars, and `armed` as the text that whitespace was armed with.
fn armed_at<'a>(
    text: &BufferText,
    sels: &SelectionSet,
    ws_len: usize,
    armed: &'a str,
) -> Vec<ArmedIndent<'a>> {
    EditView::bind(text, sels)
        .iter()
        .map(|sel| {
            let line = text.char_to_line(sel.head().offset());
            let start = text.lines().start(line).offset();
            ArmedIndent {
                range: ExclusiveRange::new(start, start.shift(ws_len as isize)),
                text: armed,
            }
        })
        .collect()
}

/// Plans and applies an electric dedent with a provider that logs every
/// (line, cursor) it is asked about and answers `width`, in tab width 4.
fn dedent(
    src: &str,
    armed: &str,
    style: TabStyle,
    width: Option<u32>,
    asked: &mut Vec<(usize, usize)>,
) -> (String, Vec<(usize, usize)>) {
    let (text, sels) = parse_state(src);
    let records = armed_at(&text, &sels, armed.chars().count(), armed);
    let candidates = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::Typed,
    );
    let plan = plan_electric_dedent(&text, candidates, style, 4, |line, cursor, _| {
        asked.push((line.index(), cursor.offset().index()));
        width.map(BufferLineCol::new)
    });
    let rewritten = plan
        .rewritten()
        .iter()
        .map(|&(index, line)| (index, line.index()))
        .collect();
    let (text, sels) = apply_electric_plan(state(text, sels), plan).into_test_result();
    (serialize_state(&text, &sels), rewritten)
}

#[test]
fn a_line_whose_whitespace_is_still_the_armed_text_is_dedented() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "fn f() {\n    }-[\n]>",
            "    ",
            TabStyle::Soft,
            Some(0),
            &mut asked
        ),
        ("fn f() {\n}-[\n]>".to_owned(), vec![(0, 1)])
    );
    assert_eq!(asked, [(1, 14)]);
}

#[test]
fn a_line_whose_whitespace_differs_from_the_armed_text_is_not_a_candidate() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "fn f() {\n    }-[\n]>",
            "\t",
            TabStyle::Soft,
            Some(0),
            &mut asked
        ),
        ("fn f() {\n    }-[\n]>".to_owned(), vec![])
    );
    assert!(asked.is_empty());
}

/// Backspace took two of the four armed spaces: the record still starts
/// at the line start, but the whitespace differs from what it armed.
#[test]
fn whitespace_shrunk_by_backspace_is_not_a_candidate() {
    let (text, sels) = parse_state("fn f() {\n  }-[\n]>");
    let records = armed_at(&text, &sels, 2, "    ");
    assert!(
        electric_candidates(
            EditView::bind(&text, &sels),
            &records,
            ElectricTrigger::Typed
        )
        .is_empty()
    );
}

#[test]
fn a_record_starting_elsewhere_is_not_a_candidate() {
    let (text, sels) = parse_state("fn f() {\n    }-[\n]>");
    let records = [ArmedIndent {
        range: ExclusiveRange::new(CharOffset::new(0), CharOffset::new(4)),
        text: "    ",
    }];
    assert!(
        electric_candidates(
            EditView::bind(&text, &sels),
            &records,
            ElectricTrigger::Typed
        )
        .is_empty()
    );
}

#[test]
fn a_selection_without_a_record_is_not_a_candidate() {
    let (text, sels) = parse_state("fn f() {\n    }-[\n]>");
    assert!(
        electric_candidates(EditView::bind(&text, &sels), &[], ElectricTrigger::Typed).is_empty()
    );
}

#[test]
fn a_blank_line_is_not_a_candidate() {
    let (text, sels) = parse_state("fn f() {\n    -[\n]>");
    let records = armed_at(&text, &sels, 4, "    ");
    assert!(
        electric_candidates(
            EditView::bind(&text, &sels),
            &records,
            ElectricTrigger::Typed
        )
        .is_empty()
    );
}

#[test]
fn a_computed_width_not_below_the_current_one_changes_nothing() {
    for width in [4, 8] {
        let mut asked = Vec::new();
        assert_eq!(
            dedent(
                "fn f() {\n    }-[\n]>",
                "    ",
                TabStyle::Soft,
                Some(width),
                &mut asked
            ),
            ("fn f() {\n    }-[\n]>".to_owned(), vec![]),
            "{width}"
        );
    }
}

#[test]
fn no_computed_width_changes_nothing() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "fn f() {\n    }-[\n]>",
            "    ",
            TabStyle::Soft,
            None,
            &mut asked
        ),
        ("fn f() {\n    }-[\n]>".to_owned(), vec![])
    );
    assert_eq!(asked, [(1, 14)]);
}

/// Both cursors' records name the line; it is asked about and rewritten
/// once, and both selections count as rewritten.
#[test]
fn two_cursors_on_one_line_rewrite_it_once() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "{\n    -{)}>-[\n]>",
            "    ",
            TabStyle::Soft,
            Some(0),
            &mut asked
        ),
        ("{\n-{)}>-[\n]>".to_owned(), vec![(0, 1), (1, 1)])
    );
    assert_eq!(asked, [(1, 6)]);
}

#[test]
fn cursors_on_different_lines_are_each_rewritten() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "{\n    }-{\n}>{\n    }-[\n]>",
            "    ",
            TabStyle::Soft,
            Some(0),
            &mut asked
        ),
        ("{\n}-{\n}>{\n}-[\n]>".to_owned(), vec![(0, 1), (1, 3)])
    );
    assert_eq!(asked, [(1, 7), (3, 15)]);
}

/// Width 6 under Hard tab width 4 renders as one tab and two spaces. The
/// first tab is kept, the second replaced. The old indent's end, where a
/// typed run's anchor sits, maps past the inserted spaces to the new
/// indent's end.
#[test]
fn the_old_indent_end_maps_to_the_new_indent_end() {
    let (text, sels) = parse_state("\t\t}-[\n]>");
    let records = armed_at(&text, &sels, 2, "\t\t");
    let candidates = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::Typed,
    );
    let plan = plan_electric_dedent(&text, candidates, TabStyle::Hard, 4, |_, _, _| {
        Some(BufferLineCol::new(6))
    });
    let edited = apply_electric_plan(state(text, sels), plan);
    let mut anchor = [CharOffset::new(2)];
    edited.changes().map_positions(&mut anchor, Assoc::Before);
    assert_eq!(anchor, [CharOffset::new(3)]);
    let (text, sels) = edited.into_test_result();
    assert_eq!(serialize_state(&text, &sels), "\t  }-[\n]>");
}

#[test]
#[should_panic(expected = "computed for another text")]
fn applying_an_electric_plan_to_another_text_panics() {
    let (text, sels) = parse_state("    }-[\n]>");
    let records = armed_at(&text, &sels, 4, "    ");
    let candidates = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::Typed,
    );
    let plan = plan_electric_dedent(&text, candidates, TabStyle::Soft, 4, |_, _, _| {
        Some(BufferLineCol::new(0))
    });
    let (other_text, other_sels) = parse_state("    ]-[\n]>");
    apply_electric_plan(state(other_text, other_sels), plan);
}

#[test]
#[should_panic(expected = "computed for another text")]
fn planning_with_candidates_of_another_text_panics() {
    let (text, sels) = parse_state("    }-[\n]>");
    let records = armed_at(&text, &sels, 4, "    ");
    let candidates = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::Typed,
    );
    let (other_text, _) = parse_state("    ]-[\n]>");
    plan_electric_dedent(&other_text, candidates, TabStyle::Soft, 4, |_, _, _| None);
}

/// U+3000 is one char 2 columns wide, and `é` is `e` plus a combining
/// accent: the armed text is compared as text and the provider is asked
/// about the head, the `\n` at char 4.
#[test]
fn a_wide_blank_armed_indent_is_compared_as_text() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "\u{3000}e\u{301}-[\n]>",
            "\u{3000}",
            TabStyle::Soft,
            None,
            &mut asked,
        ),
        ("\u{3000}e\u{301}-[\n]>".to_owned(), vec![])
    );
    assert_eq!(asked, [(0, 3)]);
}

/// Two U+3000 blanks are 4 columns; width 2 renders as two spaces, which
/// share no leading cluster with the old indent, so all of it is replaced.
#[test]
fn an_indent_sharing_no_leading_cluster_is_replaced_whole() {
    let mut asked = Vec::new();
    assert_eq!(
        dedent(
            "\u{3000}\u{3000}}-[\n]>",
            "\u{3000}\u{3000}",
            TabStyle::Soft,
            Some(2),
            &mut asked,
        ),
        ("  }-[\n]>".to_owned(), vec![(0, 0)])
    );
}

/// Enter replaces a selection with its break, which lands at the
/// selection's start: the line it leaves is the start's, line 1, and the
/// provider is asked about the start, the `a` at char 7. Typing is asked
/// about the head's line, line 2, which has no record.
#[test]
fn a_line_break_is_asked_about_a_selections_start() {
    let (text, sels) = parse_state("{\n    }-[a\nb]>\n");
    let records = [ArmedIndent {
        range: ExclusiveRange::new(CharOffset::new(2), CharOffset::new(6)),
        text: "    ",
    }];
    let typed = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::Typed,
    );
    assert!(typed.is_empty());
    let candidates = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::LineBreak,
    );
    let mut asked = Vec::new();
    plan_electric_dedent(&text, candidates, TabStyle::Soft, 4, |line, at, trigger| {
        asked.push((line.index(), at.offset().index(), trigger));
        None
    });
    assert_eq!(asked, [(1, 7, ElectricTrigger::LineBreak)]);
}

/// The line a break leaves is the selection's start's, line 1, not the
/// head's line 2: that is the line rewritten and reported.
#[test]
fn a_line_break_rewrites_the_line_of_a_selections_start() {
    let (text, sels) = parse_state("{\n    }-[a\nb]>\n");
    let records = [ArmedIndent {
        range: ExclusiveRange::new(CharOffset::new(2), CharOffset::new(6)),
        text: "    ",
    }];
    let candidates = electric_candidates(
        EditView::bind(&text, &sels),
        &records,
        ElectricTrigger::LineBreak,
    );
    let plan = plan_electric_dedent(&text, candidates, TabStyle::Soft, 4, |_, _, _| {
        Some(BufferLineCol::new(0))
    });
    assert_eq!(plan.rewritten(), [(0, ContentLine::new(1))]);
    let (text, sels) = apply_electric_plan(state(text, sels), plan).into_test_result();
    assert_eq!(serialize_state(&text, &sels), "{\n}-[a\nb]>\n");
}
