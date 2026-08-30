use super::*;
use pretty_assertions::assert_eq;

// ── word-selects-whitespace (mm/MM, w/W/b/B around-word default) ──────────
//
// Full-dispatch coverage of the default flip: the ops-level tests in
// hume-ops/src/motion/tests/ and hume-ops/src/text_object/tests/ cover the span math
// (leading-preferred, trailing fallback for the first word of a line); these
// confirm the setting actually gates behavior through the real
// keymap/registry/dispatch path (:set, direct field write, and replay).

#[test]
fn w_default_selects_leading_space() {
    let mut ed = editor_from("-[f]>oo bar baz\n");
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo-[ bar]> baz\n");
}

#[test]
fn w_with_setting_off_selects_bare_word() {
    let mut ed = editor_from("-[f]>oo bar baz\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo -[bar]> baz\n");
}

#[test]
fn w_set_buffer_off_selects_bare_word() {
    // Exercises the typed-command path (:set), not just a direct field write.
    let mut ed = editor_from("-[f]>oo bar baz\n");
    type_cmd(&mut ed, ":set buffer word-selects-whitespace=false");
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo -[bar]> baz\n");
}

#[test]
#[allow(non_snake_case)]
fn W_default_selects_leading_space() {
    let mut ed = editor_from("-[f]>oo, bar baz\n");
    ed.feed_key(key('W'));
    assert_eq!(state(&ed), "foo,-[ bar]> baz\n");
}

#[test]
fn b_default_selects_leading_space() {
    let mut ed = editor_from("foo bar -[b]>az\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "foo-[ bar]> baz\n");
}

/// Regression: pressing `b` repeatedly must walk back through distinct
/// words, not get stuck re-selecting the same one. Backward motions search
/// from `start()` rather than `head()`: after a first-word-of-line landing
/// (the final press here, onto "one"), the around-expansion absorbs
/// trailing whitespace and leaves head on that space — just outside the
/// word's own bounds — which would defeat `select_prev_word`'s "am I still
/// on the word I just found" check and re-return the same word. See
/// apply_word_select's `backward` parameter in hume-ops/src/motion/word.rs.
#[test]
fn b_b_walks_back_through_distinct_words() {
    let mut ed = editor_from("one two three -[f]>our\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "one two-[ three]> four\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "one-[ two]> three four\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "-[one ]>two three four\n");
}

#[test]
fn mm_default_matches_around_word() {
    // mm and maw share the same word_unit_at body and always select the
    // identical span; "hello" is the first word of the buffer here, so both
    // fall back to trailing absorption.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "-[hello ]>world\n");
}

#[test]
fn mm_mid_line_matches_maw() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "foo-[ bar]> baz\n");

    let mut ed2 = editor_from("foo -[b]>ar baz\n");
    ed2.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(state(&ed2), "foo-[ bar]> baz\n");
}

#[test]
fn mm_with_setting_off_matches_inner_word() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "-[hello]> world\n");
}

#[test]
fn mm_default_on_whitespace_extends_to_adjacent_word() {
    // word_unit_at's on-whitespace rule: cursor on the space snaps to the
    // following word, whose normal unit re-absorbs that space as its leading
    // run — same as pressing maw there.
    let mut ed = editor_from("foo-[ ]>bar\n");
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "foo-[ bar]>\n");
}

#[test]
#[allow(non_snake_case)]
fn MM_default_matches_around_uppercase_word() {
    let mut ed = editor_from("-[h]>ello.world foo\n");
    ed.feed_keys([key('M'), key('M')]);
    assert_eq!(state(&ed), "-[hello.world ]>foo\n");
}

#[test]
#[allow(non_snake_case)]
fn MM_with_setting_off_matches_inner_uppercase_word() {
    let mut ed = editor_from("-[h]>ello.world foo\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_keys([key('M'), key('M')]);
    assert_eq!(state(&ed), "-[hello.world]> foo\n");
}

/// Direct side-by-side comparison, mirroring `mm_mid_line_matches_maw` but
/// for the uppercase pair — the lowercase tests above only check `MM`
/// against a hardcoded literal, never `MM` against a typed `m A W` on the
/// same input.
#[test]
#[allow(non_snake_case)]
fn MM_mid_line_matches_maW() {
    let mut ed = editor_from("foo.bar -[b]>az.qux quux\n");
    ed.feed_keys([key('M'), key('M')]);
    assert_eq!(state(&ed), "foo.bar-[ baz.qux]> quux\n");

    let mut ed2 = editor_from("foo.bar -[b]>az.qux quux\n");
    ed2.feed_keys([key('m'), key('a'), key('W')]);
    assert_eq!(state(&ed2), "foo.bar-[ baz.qux]> quux\n");
}

/// Same, with the setting off — `MM` against a typed `m i W`.
#[test]
#[allow(non_snake_case)]
fn MM_with_setting_off_matches_miW() {
    let mut ed = editor_from("foo.bar -[b]>az.qux quux\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_keys([key('M'), key('M')]);
    assert_eq!(state(&ed), "foo.bar -[baz.qux]> quux\n");

    let mut ed2 = editor_from("foo.bar -[b]>az.qux quux\n");
    ed2.state.settings.word_selects_whitespace = false;
    ed2.feed_keys([key('m'), key('i'), key('W')]);
    assert_eq!(state(&ed2), "foo.bar -[baz.qux]> quux\n");
}

/// Every equivalence test above runs in Move mode only — `mm`/`MM` dispatch
/// through the same `around_fun.unwrap_or(fun)` gate regardless of
/// `MotionMode`, so this checks the pairing also holds once an existing
/// selection is being *grown* (Extend), not just replaced.
#[test]
fn mm_extend_mode_matches_maw_extend() {
    let mut ed = editor_from("one -[t]>wo three\n");
    ed.state.mode = Mode::Extend;
    ed.feed_keys([key('m'), key('m')]);
    let mm_state = state(&ed);

    let mut ed2 = editor_from("one -[t]>wo three\n");
    ed2.state.mode = Mode::Extend;
    ed2.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(
        mm_state,
        state(&ed2),
        "mm vs maw must agree in Extend mode too"
    );
}

#[test]
#[allow(non_snake_case)]
fn MM_extend_mode_matches_maW_extend() {
    let mut ed = editor_from("one.zero -[t]>wo.zero three\n");
    ed.state.mode = Mode::Extend;
    ed.feed_keys([key('M'), key('M')]);
    let mm_state = state(&ed);

    let mut ed2 = editor_from("one.zero -[t]>wo.zero three\n");
    ed2.state.mode = Mode::Extend;
    ed2.feed_keys([key('m'), key('a'), key('W')]);
    assert_eq!(
        mm_state,
        state(&ed2),
        "MM vs maW must agree in Extend mode too"
    );
}

#[test]
fn miw_unaffected_by_setting() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('i'), key('w')]);
    assert_eq!(state(&ed), "-[hello]> world\n");

    let mut ed2 = editor_from("-[h]>ello world\n");
    ed2.state.settings.word_selects_whitespace = false;
    ed2.feed_keys([key('m'), key('i'), key('w')]);
    assert_eq!(state(&ed2), "-[hello]> world\n");
}

#[test]
fn maw_unaffected_by_setting() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(state(&ed), "-[hello ]>world\n");

    let mut ed2 = editor_from("-[h]>ello world\n");
    ed2.state.settings.word_selects_whitespace = false;
    ed2.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(state(&ed2), "-[hello ]>world\n");
}

/// `select-word` (`mm`) is a Selection command (`SelectionTracking::Establishes`),
/// so it pushes an establish step onto the dot-repeat recipe (unlike the word
/// motions, which are `Extends`) — replay re-runs it via `run_native_body`,
/// which must re-resolve
/// `word-selects-whitespace` fresh each time rather than baking in whatever
/// was true at the original keypress.
#[test]
fn dot_repeat_of_mm_delete_reresolves_word_selects_whitespace() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('m')]); // select "hello " (around, default on)
    ed.feed_key(key('d')); // delete "hello " -> "world\n"
    assert_eq!(ed.doc().text().to_string(), "world\n");

    ed.state.settings.word_selects_whitespace = false;
    ed.feed_key(key('.')); // replay: re-establishes via mm (now bare), then deletes

    assert_eq!(ed.doc().text().to_string(), "\n");
}
