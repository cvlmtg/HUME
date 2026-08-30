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
