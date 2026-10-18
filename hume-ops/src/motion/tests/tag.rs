use super::super::*;
use hume_editing::state::EditState;
use test_fixtures::assert_state;

// `adjacent_tag` is reached from hume-editor's structural dispatch as the
// lexical `goto-next-tag`/`goto-prev-tag` finder. This drives it through
// the same `apply_object_motion` call.
fn cmd_goto_tag(state: EditState, count: usize, mode: MotionMode, backward: bool) -> EditState {
    let text = state.text().clone();
    apply_object_motion(state, mode, count, backward, |pos| {
        adjacent_tag(&text, pos, backward)
    })
}

fn goto(input: &str, count: usize, mode: MotionMode, backward: bool, expected: &str) {
    assert_state!(
        input,
        |(text, sels)| cmd_goto_tag(
            test_fixtures::testing::state(text, sels),
            count,
            mode,
            backward
        ),
        expected
    );
}

// ── Forward ───────────────────────────────────────────────────────────────

#[test]
fn next_tag_selects_the_next_element() {
    goto(
        "-[a]> <p>x</p> <b>y</b>\n",
        1,
        MotionMode::Move,
        false,
        "a <[<p>x</p>]- <b>y</b>\n",
    );
}

#[test]
fn next_tag_from_a_selected_element_selects_its_next_sibling() {
    goto(
        "a <[<p>x</p>]- <b>y</b>\n",
        1,
        MotionMode::Move,
        false,
        "a <p>x</p> <[<b>y</b>]-\n",
    );
}

#[test]
fn next_tag_from_a_selected_element_skips_its_children() {
    goto(
        "<[<div><p>x</p></div>]-<b>y</b>\n",
        1,
        MotionMode::Move,
        false,
        "<div><p>x</p></div><[<b>y</b>]-\n",
    );
}

#[test]
fn next_tag_from_open_markup_selects_the_first_child() {
    goto(
        "<d-[i]>v><p>x</p></div>\n",
        1,
        MotionMode::Move,
        false,
        "<div><[<p>x</p>]-</div>\n",
    );
}

#[test]
fn next_tag_skips_closing_tags() {
    goto(
        "<p>-[x]></p><b>y</b>\n",
        1,
        MotionMode::Move,
        false,
        "<p>x</p><[<b>y</b>]-\n",
    );
}

#[test]
fn next_tag_counts_void_and_self_closing_tags_as_elements() {
    goto(
        "-[a]><br>b<img/>c\n",
        1,
        MotionMode::Move,
        false,
        "a<[<br>]-b<img/>c\n",
    );
    goto(
        "-[a]><br>b<img/>c\n",
        2,
        MotionMode::Move,
        false,
        "a<br>b<[<img/>]-c\n",
    );
}

#[test]
fn next_tag_skips_a_comment() {
    goto(
        "-[a]><!-- <p> --><b>y</b>\n",
        1,
        MotionMode::Move,
        false,
        "a<!-- <p> --><[<b>y</b>]-\n",
    );
}

#[test]
fn next_tag_after_the_last_element_is_a_noop() {
    goto(
        "<p>x</p> -[a]>\n",
        1,
        MotionMode::Move,
        false,
        "<p>x</p> -[a]>\n",
    );
}

#[test]
fn next_tag_extend_unions_with_the_selection() {
    goto(
        "-[a]> <p>x</p>\n",
        1,
        MotionMode::Extend,
        false,
        "-[a <p>x</p>]>\n",
    );
}

#[test]
fn next_tag_with_a_unicode_name() {
    goto(
        "-[a]><\u{6f22}>x</\u{6f22}>\n",
        1,
        MotionMode::Move,
        false,
        "a<[<\u{6f22}>x</\u{6f22}>]-\n",
    );
}

/// U+0600 is a Prepend character: it forms one cluster with the `<` after
/// it, so the element starting in the cursor's own cluster is not "next".
#[test]
fn next_tag_makes_progress_past_a_prepend_cluster() {
    goto(
        "-[\u{600}<]>p>x</p><b>y</b>\n",
        1,
        MotionMode::Move,
        false,
        "\u{600}<p>x</p><[<b>y</b>]-\n",
    );
}

// ── Backward ──────────────────────────────────────────────────────────────

#[test]
fn prev_tag_selects_the_previous_element() {
    goto(
        "<p>x</p> <b>y</b> -[a]>\n",
        1,
        MotionMode::Move,
        true,
        "<p>x</p> <[<b>y</b>]- a\n",
    );
}

#[test]
fn prev_tag_from_a_selected_element_selects_the_one_before() {
    goto(
        "<p>x</p> <[<b>y</b>]- a\n",
        1,
        MotionMode::Move,
        true,
        "<[<p>x</p>]- <b>y</b> a\n",
    );
}

#[test]
fn prev_tag_from_inside_lands_on_the_element_own_start() {
    goto(
        "<p>x-[y]></p>\n",
        1,
        MotionMode::Move,
        true,
        "<[<p>xy</p>]-\n",
    );
}

#[test]
fn prev_tag_count_two() {
    goto(
        "<p>x</p> <b>y</b> -[a]>\n",
        2,
        MotionMode::Move,
        true,
        "<[<p>x</p>]- <b>y</b> a\n",
    );
}

#[test]
fn prev_tag_before_the_first_element_is_a_noop() {
    goto(
        "-[a]> <p>x</p>\n",
        1,
        MotionMode::Move,
        true,
        "-[a]> <p>x</p>\n",
    );
}
