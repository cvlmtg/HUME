use super::super::*;
use hume_test_fixtures::assert_state;

// ── Brackets ──────────────────────────────────────────────────────────────

#[test]
fn goto_matching_pair_paren_open_to_close() {
    assert_state!(
        "-[(]>hello)\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "(hello-[)]>\n"
    );
}

#[test]
fn goto_matching_pair_paren_close_to_open() {
    assert_state!(
        "(hello-[)]>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[(]>hello)\n"
    );
}

#[test]
fn goto_matching_pair_bracket() {
    assert_state!(
        "-[[]>hello]\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "[hello-[]]>\n"
    );
}

#[test]
fn goto_matching_pair_brace() {
    assert_state!(
        "-[{]>hello}\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "{hello-[}]>\n"
    );
}

#[test]
fn goto_matching_pair_nested() {
    // Cursor on the inner '(' must land on the inner ')', not the outer one.
    assert_state!(
        "(a -[(]>b) c)\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "(a (b-[)]> c)\n"
    );
}

#[test]
fn goto_matching_pair_unmatched_is_noop() {
    assert_state!(
        "-[(]>hello\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[(]>hello\n"
    );
}

#[test]
fn goto_matching_pair_not_on_delimiter_is_noop() {
    // Strict mode: no forward line-scan. Cursor mid-word does nothing.
    assert_state!(
        "hel-[l]>o(x)\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "hel-[l]>o(x)\n"
    );
}

#[test]
fn goto_matching_pair_extend_keeps_anchor() {
    // Head starts on the '(' itself — the motion only fires from a
    // delimiter, so the anchor grows the selection to cover the whole pair.
    assert_state!(
        "-[foo(]>x)\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Extend),
        "-[foo(x)]>\n"
    );
}

// ── Tags ──────────────────────────────────────────────────────────────────

#[test]
fn goto_matching_pair_tag_open_to_close() {
    assert_state!(
        "-[<]>div>x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div>x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_close_to_open() {
    assert_state!(
        "<div>x-[<]>/div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[<]>div>x</div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_gt_lands_on_partner_lt() {
    // Cursor on the *opening* tag's own '>' still resolves — lands on the
    // closing tag's '<', not its own '>'.
    assert_state!(
        "<div-[>]>x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div>x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_cursor_on_name_resolves() {
    // Cursor inside the tag name, not on '<' or '>' at all — matchit/vim's
    // `%` fires from anywhere in the tag, not just its two delimiters.
    assert_state!(
        "<d-[i]>v class=\"x\">y</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div class=\"x\">y-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_cursor_in_attribute_value_resolves() {
    assert_state!(
        "<div class=\"-[x]>\">y</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div class=\"x\">y-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_self_closing_tag_cursor_inside_is_noop() {
    // Still no-op from inside a self-closing tag's own markup — it has no
    // partner regardless of where the cursor sits within it.
    assert_state!(
        "<b-[r]> class=\"x\"/>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<b-[r]> class=\"x\"/>\n"
    );
}

#[test]
fn goto_matching_pair_element_body_content_is_noop() {
    // Cursor in the element's body content — outside either tag's own
    // markup span — must NOT resolve; widening covers the tag itself, not
    // everything between an open and close tag.
    assert_state!(
        "<div>-[x]></div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div>-[x]></div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_nested_same_name() {
    assert_state!(
        "-[<]>div><div>x</div></div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div><div>x</div>-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_self_closing_tag_is_noop() {
    assert_state!(
        "-[<]>br/>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[<]>br/>\n"
    );
}

#[test]
fn goto_matching_pair_tag_in_comment_is_noop() {
    assert_state!(
        "<!-- -[<]>div> -->\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<!-- -[<]>div> -->\n"
    );
}

#[test]
fn goto_matching_pair_gt_inside_quoted_attribute_is_not_tag_end() {
    // The '>' inside the quoted attribute value doesn't close the tag, so
    // the real terminating '>' (the last char) is what the open tag matches.
    assert_state!(
        "-[<]>div title=\"a>b\">x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div title=\"a>b\">x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_gt_after_quoted_lt_attribute_resolves() {
    // The opening tag's own attribute value contains a literal '<'. `#` on
    // the tag's real closing '>' must still walk past that quoted '<' to
    // find the tag's own '<', not bail out at it.
    assert_state!(
        "<a t=\"<\"-[>]>x</a>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<a t=\"<\">x-[<]>/a>\n"
    );
}

#[test]
fn goto_matching_pair_angle_generic_is_noop() {
    // `Vec<String>` — parses as a plausible open tag lexically, but there is
    // no matching `</String>` anywhere, so it never resolves.
    assert_state!(
        "Vec-[<]>String>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "Vec-[<]>String>\n"
    );
}

#[test]
fn goto_matching_pair_comparison_operators_are_noop() {
    assert_state!(
        "a -[<]> b && c > d\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "a -[<]> b && c > d\n"
    );
}

#[test]
fn goto_matching_pair_doctype_is_noop() {
    assert_state!(
        "-[<]>!DOCTYPE html>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[<]>!DOCTYPE html>\n"
    );
}

#[test]
fn goto_matching_pair_stray_close_does_not_drain_enclosing_open() {
    // A `</span>` with no matching `<span>` must not discard `<div>` off a
    // shared stack — `#` on `<div>` still finds its own `</div>`.
    assert_state!(
        "-[<]>div>\n</span>\n<b>x</b>\n</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div>\n</span>\n<b>x</b>\n-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_unspaced_comparison_does_not_swallow_next_tag() {
    // `a<b` with no space read as a lexical `<` start, but the following
    // `<div>` must still parse as its own tag rather than being consumed as
    // part of `a<b`'s (nonexistent) markup.
    assert_state!(
        "a<b\n-[<]>div>x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "a<b\n<div>x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_doubled_lt_does_not_hide_nested_same_name_open() {
    // The stray second '<' in "<<div>" fails to parse as a tag itself, but
    // scanning must still rediscover the *inner* "<div>" starting right
    // after it — missing that open would leave its depth uncounted, and the
    // outer "</div>" would wrongly resolve to the inner tag's own close.
    assert_state!(
        "-[<]>div><<div>x</div></div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div><<div>x</div>-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_close_to_open_nested_same_name() {
    // Mirror of `_tag_nested_same_name`, in the close-to-open direction:
    // from the outer close, must walk past the inner (already-matched) pair
    // to land on the outer open, not stop at the inner one.
    assert_state!(
        "<div><div>x</div>-[<]>/div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[<]>div><div>x</div></div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_close_to_open_picks_innermost_unclosed_open() {
    // Two opens, one close: the close must pair with the nearer
    // (innermost) open, leaving the outer one permanently unmatched — the
    // "unmatched same-name open earlier in the buffer" case the old from-0
    // scan was designed around.
    assert_state!(
        "<div>\n<div>x-[<]>/div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div>\n-[<]>div>x</div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_close_to_open_doubled_lt_does_not_stop_walk() {
    // Backward mirror of `_doubled_lt_does_not_hide_nested_same_name_open`:
    // the stray second '<' between the outer and inner open must not stop
    // the backward walk before it reaches the outer open.
    assert_state!(
        "<div><<div>x</div>-[<]>/div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "-[<]>div><<div>x</div></div>\n"
    );
}

#[test]
fn goto_matching_pair_tag_quoted_close_tag_before_partner_is_known_limitation() {
    // Known limitation of the backward scan: a `<` inside an enclosing
    // tag's own quoted attribute can itself parse as a well-formed tag when
    // found by walking backward for `<` (the forward lexer never sees it,
    // since the whole attribute is consumed by one `parse_tag` call). Here
    // it throws off the depth count enough that the real open is never
    // reached — accepted rather than fixed; see `prev_tag`'s doc comment.
    assert_state!(
        "<div title=\"</div>\">x-[<]>/div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div title=\"</div>\">x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_unspaced_comparison_body_text_is_noop() {
    // The cursor sits in ordinary body text after an unspaced `a<b` — must
    // not resolve as if it were inside `a<b`'s markup.
    assert_state!(
        "a<b -[t]>hen</b>x</b>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "a<b -[t]>hen</b>x</b>\n"
    );
}

#[test]
fn goto_matching_pair_jsx_expression_attribute_resolves() {
    // The arrow function's own `=>` must not be read as the tag's closing
    // `>` — the real closing `>` is four characters later.
    assert_state!(
        "-[<]>div onClick={() => f()}>x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div onClick={() => f()}>x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_jsx_expression_attribute_ignores_quoted_close_tag() {
    // A later attribute's quoted value contains literal `</div>` text — the
    // partner must be the real closing tag, not the string.
    assert_state!(
        "-[<]>div onClick={() => f()} title=\"</div>\">x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<div onClick={() => f()} title=\"</div>\">x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_abruptly_closed_comment_zero_dashes() {
    // `<!-->` is HTML5's abrupt comment close (zero dashes before `>`) — the
    // comment must not swallow the well-formed tag after it.
    assert_state!(
        "<!-->\n-[<]>div>x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<!-->\n<div>x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_abruptly_closed_comment_one_dash() {
    // `<!--->` is HTML5's other abrupt comment close (one dash before `>`).
    assert_state!(
        "<!--->\n-[<]>div>x</div>\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "<!--->\n<div>x-[<]>/div>\n"
    );
}

#[test]
fn goto_matching_pair_lands_on_grapheme_boundary_not_mid_cluster() {
    // U+0600 (ARABIC NUMBER SIGN) is a `GC_Prepend` codepoint that joins
    // forward with the following ')' into one grapheme cluster. The raw
    // partner offset falls on the ')' itself — one char into that cluster —
    // so it must snap back to the cluster's start rather than land inside it.
    assert_state!(
        "-[(]>\u{0600})\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move),
        "(-[\u{0600}]>)\n"
    );
}

#[test]
fn goto_matching_pair_twice_is_involution_across_prepend_cluster() {
    // A second `#` press from the grapheme-snapped landing position (see
    // the test above) must still resolve — `matching_bracket` has to
    // recognize the bracket even when `pos` sits on the GC_Prepend
    // codepoint leading its cluster, not on the bracket char itself.
    assert_state!(
        "-[(]>\u{0600})\n",
        |(text, sels)| {
            let once = cmd_goto_matching_pair(&text, sels, 1, MotionMode::Move);
            cmd_goto_matching_pair(&text, once, 1, MotionMode::Move)
        },
        "-[(]>\u{0600})\n"
    );
}

#[test]
fn goto_matching_pair_ignores_count() {
    // `#` is an involution — folding it N times would make even counts a
    // no-op and odd counts identical to a bare `#`. Vim's `count%` means "go
    // to N% of the file", a different operation this motion doesn't
    // implement, so count is ignored entirely rather than folded.
    assert_state!(
        "-[(]>hello)\n",
        |(text, sels)| cmd_goto_matching_pair(&text, sels, 2, MotionMode::Move),
        "(hello-[)]>\n"
    );
}
