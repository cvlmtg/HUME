// Numeric count-prefix accumulation (`3w`, `12j`, …) — see
// `Editor::handle_normal`'s "Count prefix accumulation" block.
use super::*;

#[test]
fn count_prefix_accumulates_multiple_digits() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('1'));
    ed.handle_key(key('2'));
    assert_eq!(ed.state.count, Some(12));
}

#[test]
fn zero_is_a_digit_only_inside_a_count() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('0'));
    assert_eq!(
        ed.state.count, None,
        "a bare 0 is not a count digit — falls through to the trie"
    );

    ed.handle_key(key('1'));
    ed.handle_key(key('0'));
    assert_eq!(
        ed.state.count,
        Some(10),
        "0 after an existing count multiplies it"
    );
}

/// Unbounded digit entry (`999999999999999999999w`) overflows `usize`
/// arithmetic in the accumulator before this cap existed — this is the
/// resource-safety floor for every command that loops `count` times.
#[test]
fn count_prefix_caps_at_max_count() {
    let mut ed = editor_from("-[a]>bcd\n");
    for _ in 0..25 {
        ed.handle_key(key('9'));
    }
    assert_eq!(ed.state.count, Some(10_000));
}
