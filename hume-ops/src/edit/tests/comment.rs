use super::super::*;
use hume_core::{BlockCommentTokens, CommentTokens, LineCommentTokens};
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use test_fixtures::assert_state;
use test_fixtures::testing::{IntoTestResult, parse_state, state};

struct Lang {
    line: Option<LineCommentTokens>,
    block: Option<BlockCommentTokens>,
}

impl Lang {
    fn tokens(&self) -> CommentTokens<'_> {
        CommentTokens::new(self.line.as_ref(), self.block.as_ref())
            .expect("a test language has comment tokens")
    }
}

fn line(tokens: &[&str]) -> Option<LineCommentTokens> {
    Some(LineCommentTokens::from_strs(tokens))
}

fn block(pairs: &[(&str, &str)]) -> Option<BlockCommentTokens> {
    Some(BlockCommentTokens::from_pairs(pairs))
}

fn rust() -> Lang {
    Lang {
        line: line(&["//", "///", "//!"]),
        block: block(&[("/*", "*/"), ("/**", "*/")]),
    }
}

fn html() -> Lang {
    Lang {
        line: None,
        block: block(&[("<!--", "-->")]),
    }
}

fn sh() -> Lang {
    Lang {
        line: line(&["#"]),
        block: None,
    }
}

/// [`plan_comment_toggle`] at tab width 4, with `tokens` for every unit.
fn plan<'t, E: Copy>(
    state: &EditState,
    multi_line: MultiLineComments,
    tokens: Result<CommentTokens<'t>, E>,
) -> Result<CommentPlan, CommentPlanError<E>> {
    plan_comment_toggle(state.view(), multi_line, 4, |_| tokens)
}

fn toggle(
    lang: &Lang,
    multi_line: MultiLineComments,
) -> impl Fn((BufferText, SelectionSet)) -> Edited + '_ {
    move |(text, sels)| {
        let state = state(text, sels);
        let plan =
            plan(&state, multi_line, Ok::<_, ()>(lang.tokens())).expect("the resolver never fails");
        apply_comment_plan(state, plan)
    }
}

fn rust_block() -> impl Fn((BufferText, SelectionSet)) -> Edited {
    |input| toggle(&rust(), MultiLineComments::Block)(input)
}

// ── Line comments ────────────────────────────────────────────────────────

#[test]
fn a_cursor_comments_its_line_at_the_indent() {
    assert_state!("    -[f]>oo\n", rust_block(), "    // -[f]>oo\n");
}

#[test]
fn a_cursor_uncomments_its_line() {
    assert_state!("    // -[f]>oo\n", rust_block(), "    -[f]>oo\n");
}

#[test]
fn a_cursor_on_the_token_lands_where_the_token_was() {
    assert_state!("-[/]>/ foo\n", rust_block(), "-[f]>oo\n");
}

#[test]
fn uncommenting_removes_the_longest_listed_token() {
    assert_state!("-[/]>// x\n", rust_block(), "-[x]>\n");
    assert_state!("-[/]>/!x\n", rust_block(), "-[x]>\n");
}

#[test]
fn a_token_with_no_space_after_it_is_removed_alone() {
    assert_state!("//-[x]>\n", rust_block(), "-[x]>\n");
}

#[test]
fn line_comments_go_at_the_least_indent_and_skip_blank_lines() {
    assert_state!(
        "-[  a\n\n    b]>\n",
        toggle(&rust(), MultiLineComments::Line),
        "-[  // a\n\n  //   b]>\n"
    );
}

#[test]
fn tab_and_space_indents_of_one_width_take_the_token_at_one_column() {
    assert_state!(
        "-[\tfoo\n    bar]>\n",
        toggle(&rust(), MultiLineComments::Line),
        "-[\t// foo\n    // bar]>\n"
    );
}

#[test]
fn a_tab_wider_than_the_least_indent_moves_every_token_to_a_column_all_lines_reach() {
    assert_state!(
        "-[  a\n\tb]>\n",
        toggle(&rust(), MultiLineComments::Line),
        "// -[  a\n// \tb]>\n"
    );
}

#[test]
fn a_line_already_commented_gets_a_second_token_when_another_line_is_not() {
    assert_state!(
        "-[// a\nb]>\n",
        toggle(&rust(), MultiLineComments::Line),
        "// -[// a\n// b]>\n"
    );
}

// ── Block comments ───────────────────────────────────────────────────────

#[test]
fn a_multi_line_selection_is_wrapped_and_keeps_its_text() {
    assert_state!("-[  a\n  b]>\n", rust_block(), "-[  /* a\n  b]> */\n");
}

#[test]
fn a_cursor_on_a_block_comment_uncomments_it() {
    assert_state!("-[/]>* x */\n", rust_block(), "-[x]>\n");
}

#[test]
fn block_uncommenting_removes_one_space_inside_each_token() {
    assert_state!("-[/]>*  x  */\n", rust_block(), "-[ ]>x \n");
}

#[test]
fn an_empty_block_comment_uncomments_to_nothing() {
    assert_state!("-[/]>**/\n", rust_block(), "-[\n]>");
}

#[test]
fn overlapping_start_and_end_tokens_are_not_a_block_comment() {
    assert_state!("-[/]>*/\n", rust_block(), "// -[/]>*/\n");
}

#[test]
fn the_longest_matching_block_start_is_removed() {
    assert_state!("-[/]>** doc */\n", rust_block(), "-[d]>oc\n");
}

#[test]
fn a_span_holding_the_end_token_inside_is_not_a_block_comment() {
    assert_state!(
        "-[/]>* a */ x /* b */\n",
        rust_block(),
        "// -[/]>* a */ x /* b */\n"
    );
}

#[test]
fn a_span_holding_the_block_end_takes_line_comments() {
    assert_state!(
        "-[a(); /* x */\nb();]>\n",
        rust_block(),
        "// -[a(); /* x */\n// b();]>\n"
    );
}

#[test]
fn toggling_a_span_holding_the_block_end_twice_restores_it() {
    assert_state!(
        "-[a(); /* x */\nb();]>\n",
        |input| rust_block()(rust_block()(input).into_test_result()),
        "-[a(); /* x */\nb();]>\n"
    );
}

#[test]
fn a_span_holding_the_block_end_with_no_line_tokens_is_refused() {
    let html = html();
    let (text, sels) = parse_state("-[a <!-- x -->\nb]>\n");
    let state = state(text, sels);
    let result = plan(&state, MultiLineComments::Block, Ok::<_, ()>(html.tokens()));
    assert_eq!(
        result.err(),
        Some(CommentPlanError::BlockEndInside("-->".to_owned()))
    );
}

#[test]
fn a_language_with_only_block_tokens_wraps_the_trimmed_line() {
    assert_state!(
        "  -[a]>  \n",
        toggle(&html(), MultiLineComments::Block),
        "  <!-- -[a]> -->  \n"
    );
}

#[test]
fn a_block_starts_on_the_first_line_that_is_not_blank() {
    assert_state!("-[\n  a\n  b]>\n", rust_block(), "-[\n  /* a\n  b]> */\n");
}

// ── Choice of kind ───────────────────────────────────────────────────────

#[test]
fn preferring_line_comments_puts_one_token_per_line() {
    assert_state!(
        "-[a\nb]>\n",
        toggle(&rust(), MultiLineComments::Line),
        "// -[a\n// b]>\n"
    );
}

#[test]
fn a_language_without_block_tokens_comments_each_line() {
    assert_state!(
        "-[a\nb]>\n",
        toggle(&sh(), MultiLineComments::Block),
        "# -[a\n# b]>\n"
    );
}

#[test]
fn line_comments_under_a_block_preference_are_uncommented() {
    assert_state!("-[// a\n// b]>\n", rust_block(), "-[a\nb]>\n");
}

// ── Several selections ───────────────────────────────────────────────────

#[test]
fn a_commented_primary_uncomments_every_selection() {
    assert_state!("// -{a}>\n-[b]>\n", rust_block(), "-{a}>\n-[b]>\n");
}

#[test]
fn an_uncommented_primary_comments_every_selection() {
    assert_state!("// -[a]>\n-{b}>\n", rust_block(), "// // -[a]>\n// -{b}>\n");
}

#[test]
fn a_blank_primary_leaves_the_choice_to_the_first_unit_that_is_not_blank() {
    assert_state!(
        "// -[a]>\n// -[b]>\n-{\n}>",
        rust_block(),
        "-[a]>\n-[b]>\n-{\n}>"
    );
}

#[test]
fn selections_on_blank_lines_only_change_nothing() {
    assert_state!("-{\n}>-[\n]>", rust_block(), "-{\n}>-[\n]>");
}

#[test]
fn two_cursors_on_one_line_comment_it_once() {
    assert_state!("-{a}> -[b]>\n", rust_block(), "// -{a}> -[b]>\n");
}

#[test]
fn a_linewise_selection_stays_linewise_under_a_line_comment() {
    assert_state!("-[foo\n]>bar\n", rust_block(), "-[// foo\n]>bar\n");
}

#[test]
fn a_linewise_selection_stays_linewise_under_a_block_comment() {
    assert_state!("-[a\nb\n]>", rust_block(), "-[/* a\nb */\n]>");
}

// ── Grapheme clusters ────────────────────────────────────────────────────

#[test]
fn a_token_ending_inside_a_cluster_does_not_match() {
    assert_state!("-[/\u{301}]>/ x\n", rust_block(), "// -[/\u{301}]>/ x\n");
}

#[test]
fn a_space_carrying_a_combining_mark_is_not_removed() {
    assert_state!("-[/]>/ \u{301}x\n", rust_block(), "-[ \u{301}]>x\n");
}

// ── Contract ─────────────────────────────────────────────────────────────

#[test]
fn a_resolver_error_is_returned() {
    let (text, sels) = parse_state("-[a]>\n");
    let state = state(text, sels);
    let result = plan(&state, MultiLineComments::Block, Err("no tokens"));
    assert_eq!(result.err(), Some(CommentPlanError::Tokens("no tokens")));
}

#[test]
#[should_panic(expected = "computed for another text")]
fn applying_a_plan_to_another_text_panics() {
    let rust = rust();
    let (text, sels) = parse_state("-[a]>\n");
    let planned = state(text, sels);
    let plan = plan(
        &planned,
        MultiLineComments::Block,
        Ok::<_, ()>(rust.tokens()),
    )
    .unwrap();
    let (other_text, other_sels) = parse_state("-[b]>\n");
    apply_comment_plan(state(other_text, other_sels), plan);
}
