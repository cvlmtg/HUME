use super::*;
use pretty_assertions::assert_eq;

/// Independent oracle: forward(old) == new and inverse(new) == old.
fn assert_round_trip(old: &str, new: &str) {
    let old_t = Text::from(old);
    let new_t = Text::from(new);
    let (fwd, inv) = changesets_from_line_diff(&old_t, &new_t);

    let applied_fwd = fwd.apply(&old_t).expect("forward apply");
    assert_eq!(
        applied_fwd.to_string(),
        new_t.to_string(),
        "forward(old) must equal new",
    );
    let applied_inv = inv.apply(&new_t).expect("inverse apply");
    assert_eq!(
        applied_inv.to_string(),
        old_t.to_string(),
        "inverse(new) must equal old",
    );

    // Length invariants: forward sized to old, inverse sized to new.
    assert_eq!(fwd.len_before, old_t.len_chars());
    assert_eq!(fwd.len_after, new_t.len_chars());
    assert_eq!(inv.len_before, new_t.len_chars());
    assert_eq!(inv.len_after, old_t.len_chars());
}

#[test]
fn identical_inputs() {
    assert_round_trip("hello\nworld\n", "hello\nworld\n");
    assert_round_trip("a\n", "a\n");
}

#[test]
fn pure_insert_at_head() {
    assert_round_trip("b\nc\n", "a\nb\nc\n");
}

#[test]
fn pure_insert_in_middle() {
    assert_round_trip("a\nc\n", "a\nb\nc\n");
}

#[test]
fn pure_insert_at_tail() {
    assert_round_trip("a\nb\n", "a\nb\nc\n");
}

#[test]
fn pure_delete_at_head() {
    assert_round_trip("a\nb\nc\n", "b\nc\n");
}

#[test]
fn pure_delete_in_middle() {
    assert_round_trip("a\nb\nc\n", "a\nc\n");
}

#[test]
fn pure_delete_at_tail() {
    assert_round_trip("a\nb\nc\n", "a\nb\n");
}

#[test]
fn replace_block_in_middle() {
    assert_round_trip("a\n1\n2\n3\nz\n", "a\nX\nY\nz\n");
}

#[test]
fn internal_empty_lines() {
    // Internal empty lines carry their own `\n`; the trailing-empty token
    // is `""` on both sides. Round-trips across all three boundaries.
    assert_round_trip("a\n\nb\n", "a\n\n\nc\n");
    assert_round_trip("a\n\nb\n", "a\nb\n");
    assert_round_trip("\n\na\n", "\na\n");
}

#[test]
fn trailing_newline_buffer() {
    // The trailing-empty token is `""`; the line before it is `"a\n"`.
    assert_round_trip("a\n", "a\n");
    assert_round_trip("a\n", "b\n");
    assert_round_trip("a\nb\n", "a\n");
    // Regression for the original oracle failure: a single-content-line
    // buffer vs a single-`\n` buffer. The bare-`split('\n')` approach
    // desynced the cursors by one `\n` here.
    assert_round_trip("a\n", "\n");
    assert_round_trip("\n", "a\n");
}

#[test]
fn zwj_emoji_inside_equal_run() {
    // The family emoji is a ZWJ sequence of 5 chars. It lives inside an
    // Equal line so the line diff must treat it as opaque content; the
    // char-offset walk carries it through unchanged.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let old = format!("x {family} y\nshared\n");
    let new = format!("x {family} z\nshared\n");
    assert_round_trip(&old, &new);
}

#[test]
fn completely_different_inputs() {
    assert_round_trip("aaa\nbbb\n", "xxx\nyyy\nzzz\n");
}

#[test]
fn myers_coarse_replace_still_round_trips() {
    // Duration::ZERO forces the histogram pass to bail and Myers to
    // return the coarsest single-Replace result. The hunk partition
    // invariant still holds, so the round-trip must succeed regardless.
    let old: String = (0..40).map(|i| format!("line-{i}-pad-{}", i % 3)).collect();
    let new: String = (0..40)
        .map(|i| format!("line-{i}-pad-{}", (i + 1) % 5))
        .collect();
    let old_t = Text::from(old.as_str());
    let new_t = Text::from(new.as_str());
    let (fwd, inv) = changesets_from_line_diff_with_deadline(&old_t, &new_t, Duration::ZERO);

    let applied_fwd = fwd.apply(&old_t).expect("forward apply");
    assert_eq!(applied_fwd.to_string(), new_t.to_string());
    let applied_inv = inv.apply(&new_t).expect("inverse apply");
    assert_eq!(applied_inv.to_string(), old_t.to_string());
}

#[test]
fn inverse_is_fine_grained_for_single_line_change() {
    // Pins the "fine-grained, not coarse" property: a single-line edit's
    // inverse must carry only the changed line, not a full-buffer delete.
    let old = Text::from("alpha\nbeta\ngamma\n");
    let new = Text::from("alpha\nBETA\ngamma\n");
    let (_fwd, inv) = changesets_from_line_diff(&old, &new);

    let inv_text = inv.apply(&new).expect("inverse apply");
    assert_eq!(inv_text.to_string(), "alpha\nbeta\ngamma\n");
    // Coarse inverse would be Delete(18) | Insert("alpha\nbeta\ngamma\n").
    // The fine-grained inverse re-inserts only the changed line.
    let has_small_insert = inv.ops().iter().any(|op| match op {
        super::super::Operation::Insert(s) => s == "beta\n",
        _ => false,
    });
    assert!(
        has_small_insert,
        "inverse should re-insert only the changed line, got {:?}",
        inv.ops(),
    );
}

// ── property-based round-trip ────────────────────────────────────────────

use proptest::prelude::*;

fn arb_small_text(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['a', 'b', 'c', '\n', '\u{0301}']),
        0..max_len,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

proptest! {
    #[test]
    fn prop_round_trip(old in arb_small_text(16), new in arb_small_text(16)) {
        let old_t = Text::from(old.as_str());
        let new_t = Text::from(new.as_str());
        let (fwd, inv) = changesets_from_line_diff(&old_t, &new_t);

        let applied_fwd = fwd.apply(&old_t).expect("forward apply");
        prop_assert_eq!(applied_fwd.to_string(), new_t.to_string());

        let applied_inv = inv.apply(&new_t).expect("inverse apply");
        prop_assert_eq!(applied_inv.to_string(), old_t.to_string());
    }
}
