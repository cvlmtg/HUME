use super::super::*;
use test_fixtures::assert_state;
use test_fixtures::unicode::{COMBINING, IDEO_SPACE, LONE_MARK, NBSP};

type FindObject = fn(&BufferText, ClusterStart) -> Option<ClusterRange>;

// `inner_tag`/`around_tag` are reached from hume-editor's structural
// dispatch as the lexical fallback, not through a `cmd_*` wrapper. This
// drives them through the same `apply_text_object_by_mode` call.
fn check(find: FindObject, mode: MotionMode, input: &str, expected: &str) {
    assert_state!(
        input,
        |(text, sels)| apply_text_object_by_mode(
            test_fixtures::testing::state(text, sels),
            mode,
            find
        ),
        expected
    );
}

fn inner(input: &str, expected: &str) {
    check(inner_tag, MotionMode::Move, input, expected);
}

fn around(input: &str, expected: &str) {
    check(around_tag, MotionMode::Move, input, expected);
}

// ── inner_tag ─────────────────────────────────────────────────────────────

#[test]
fn inner_tag_selects_the_content() {
    inner("<p>he-[l]>lo</p>\n", "<p>-[hello]></p>\n");
}

#[test]
fn inner_tag_spans_every_child() {
    inner(
        "<div><a>1</a> te-[x]>t <b>2</b></div>\n",
        "<div>-[<a>1</a> text <b>2</b>]></div>\n",
    );
}

#[test]
fn inner_tag_trims_surrounding_whitespace() {
    inner(
        "<div>\n  hel-[l]>o\n</div>\n",
        "<div>\n  -[hello]>\n</div>\n",
    );
}

#[test]
fn inner_tag_on_whitespace_outside_the_trimmed_content_selects_the_parent_content() {
    inner("<div><p>-[ ]>x </p></div>\n", "<div>-[<p> x </p>]></div>\n");
}

#[test]
fn inner_tag_on_open_markup_selects_the_parent_content() {
    inner("<div><p-[>]>x</p></div>\n", "<div>-[<p>x</p>]></div>\n");
}

#[test]
fn inner_tag_on_close_markup_selects_the_parent_content() {
    inner("<div><p>x<-[/]>p></div>\n", "<div>-[<p>x</p>]></div>\n");
}

#[test]
fn inner_tag_on_top_level_markup_is_a_noop() {
    inner("<-[p]>>x</p>\n", "<-[p]>>x</p>\n");
}

#[test]
fn inner_tag_outside_any_element_is_a_noop() {
    inner("a-[b]>c <p>x</p>\n", "a-[b]>c <p>x</p>\n");
}

#[test]
fn inner_tag_of_an_empty_element_is_a_noop() {
    inner("<p>-[<]>/p>\n", "<p>-[<]>/p>\n");
}

#[test]
fn inner_tag_of_a_blank_element_is_a_noop() {
    inner("<p> -[ ]> </p>\n", "<p> -[ ]> </p>\n");
}

#[test]
fn inner_tag_nested_same_name_picks_the_innermost() {
    inner(
        "<div><div>-[x]></div></div>\n",
        "<div><div>-[x]></div></div>\n",
    );
    inner(
        "<div><div>x<-[/]>div></div>\n",
        "<div>-[<div>x</div>]></div>\n",
    );
}

#[test]
fn inner_tag_skips_a_void_tag_before_the_cursor() {
    inner("<div><br>te-[x]>t</div>\n", "<div>-[<br>text]></div>\n");
}

#[test]
fn inner_tag_on_a_self_closing_tag_selects_the_parent_content() {
    inner("<div><img-[/]>>x</div>\n", "<div>-[<img/>x]></div>\n");
}

#[test]
fn inner_tag_ignores_a_stray_close_of_another_name() {
    inner(
        "<div></span>te-[x]>t</div>\n",
        "<div>-[</span>text]></div>\n",
    );
}

#[test]
fn inner_tag_skips_a_quoted_gt_in_an_attribute() {
    inner(
        "<a title=\"a>b\">li-[n]>k</a>\n",
        "<a title=\"a>b\">-[link]></a>\n",
    );
}

#[test]
fn inner_tag_skips_a_jsx_arrow_in_an_attribute() {
    inner(
        "<b onClick={() => f()}>g-[o]></b>\n",
        "<b onClick={() => f()}>-[go]></b>\n",
    );
}

#[test]
fn inner_tag_extend_grows_to_the_parent_content() {
    check(
        inner_tag,
        MotionMode::Extend,
        "<div><p>-[x]></p></div>\n",
        "<div>-[<p>x</p>]></div>\n",
    );
}

#[test]
fn inner_tag_with_unicode_names() {
    inner(
        "<caf\u{e9}>-[x]></caf\u{e9}>\n",
        "<caf\u{e9}>-[x]></caf\u{e9}>\n",
    );
    inner(
        "<cafe\u{301}>a-[b]></cafe\u{301}>\n",
        "<cafe\u{301}>-[ab]></cafe\u{301}>\n",
    );
    inner(
        "<\u{6f22}>a-[b]></\u{6f22}>\n",
        "<\u{6f22}>-[ab]></\u{6f22}>\n",
    );
}

#[test]
fn inner_tag_trims_unicode_blanks() {
    inner(
        &format!("<p>{NBSP}a-[b]>{IDEO_SPACE}</p>\n"),
        &format!("<p>{NBSP}-[ab]>{IDEO_SPACE}</p>\n"),
    );
}

#[test]
fn inner_tag_leaves_a_mark_on_the_gt_with_the_markup() {
    inner(
        &format!("<p>{LONE_MARK}a-[b]></p>\n"),
        &format!("<p>{LONE_MARK}-[ab]></p>\n"),
    );
}

#[test]
fn inner_tag_keeps_a_combining_sequence_whole() {
    inner(
        &format!("<p>-[{COMBINING}]>x</p>\n"),
        &format!("<p>-[{COMBINING}x]></p>\n"),
    );
}

#[test]
fn inner_tag_skips_a_close_tag_inside_a_comment() {
    inner(
        "<div><!-- </div> -->te-[x]>t</div>\n",
        "<div>-[<!-- </div> -->text]></div>\n",
    );
}

// Known limits of the lexical scan, pinned so a change in them is visible.

#[test]
fn inner_tag_does_not_imply_html_end_tags() {
    inner("<ul><li>a<li>-[b]></ul>\n", "<ul>-[<li>a<li>b]></ul>\n");
}

#[test]
fn inner_tag_compares_names_case_sensitively() {
    inner("<DIV>x-[y]></div>\n", "<DIV>x-[y]></div>\n");
}

// ── around_tag ────────────────────────────────────────────────────────────

#[test]
fn around_tag_from_the_content() {
    around("<p>he-[l]>lo</p>\n", "-[<p>hello</p>]>\n");
}

#[test]
fn around_tag_from_open_markup() {
    around(
        "<div><p cl-[a]>ss=\"c\">x</p></div>\n",
        "<div>-[<p class=\"c\">x</p>]></div>\n",
    );
}

#[test]
fn around_tag_from_close_markup() {
    around("<div><p>x</-[p]>></div>\n", "<div>-[<p>x</p>]></div>\n");
}

#[test]
fn around_tag_nested_picks_the_innermost() {
    around(
        "<div><div>-[x]></div></div>\n",
        "<div>-[<div>x</div>]></div>\n",
    );
}

#[test]
fn around_tag_spans_lines() {
    around(
        "<ul>\n  <li>-[a]></li>\n</ul>\n",
        "<ul>\n  -[<li>a</li>]>\n</ul>\n",
    );
}

#[test]
fn around_tag_on_a_void_tag_is_the_tag() {
    around("<p>a<b-[r]>>b</p>\n", "<p>a-[<br>]>b</p>\n");
}

#[test]
fn around_tag_on_a_self_closing_tag_is_the_tag() {
    around("<p>a<im-[g]> />b</p>\n", "<p>a-[<img />]>b</p>\n");
}

#[test]
fn around_tag_on_a_stray_close_selects_the_enclosing_element() {
    around("<div>a<-[/]>span>b</div>\n", "-[<div>a</span>b</div>]>\n");
}

#[test]
fn around_tag_outside_any_element_is_a_noop() {
    around("a-[b]>c <p>x</p>\n", "a-[b]>c <p>x</p>\n");
}

/// U+0600 is a Prepend character: it forms one cluster with the `<` after it.
#[test]
fn around_tag_widens_over_a_prepend_cluster_before_the_lt() {
    around("\u{600}<p>-[x]></p>\n", "-[\u{600}<p>x</p>]>\n");
}

#[test]
fn around_tag_extend_grows_to_the_parent() {
    check(
        around_tag,
        MotionMode::Extend,
        "<div>-[<p>x</p>]></div>\n",
        "-[<div><p>x</p></div>]>\n",
    );
}
