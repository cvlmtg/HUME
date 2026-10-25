//! Recipes composed by match/surround, select-all and keep-primary, and failed steps.

use super::*;
use pretty_assertions::assert_eq;

/// `ms(` (select surrounding `()` delimiters) is a selection-building step and
/// must be recorded in the dot-repeat recipe, so `.` re-runs `ms(` + `d` on the
/// next pair rather than just deleting the current selection.
///
/// Buffer "(foo) (bar) baz": cursor on the first 'o' of "foo", `ms(` selects
/// the two parens, `d` deletes them → "foo (bar) baz". Collapse to one cursor,
/// move to "bar", `.` must replay `ms(` + `d` on "(bar)" → "foo bar baz".
#[test]
fn dot_repeat_of_match_surround_deletes_both_parens() {
    let mut ed = editor_from("(-[f]>oo) (bar) baz\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('s'));
    ed.feed_key(key('(')); // two cursors on the delimiters
    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        "foo (bar) baz\n",
        "ms( then d must remove the surrounding parens"
    );

    ed.feed_key(key(','));
    ed.feed_key(key('w'));
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // replay ms( + d on "(bar)"
    assert_eq!(
        ed.doc().text().to_string(),
        "foo bar baz\n",
        "`.` must replay ms( + d and strip the (bar) parens",
    );
}

/// `ma(` (select around `()`, including the delimiters) on `(foo)` leaves a
/// non-collapsed selection, unlike `ms(`
/// (`dot_repeat_of_match_surround_deletes_both_parens` above). This pins that
/// it replays correctly under `SelectionTracking::Establishes`, which
/// restarts the dot-repeat recipe in Move mode (see `update_recipe`).
#[test]
fn dot_repeat_of_match_around_deletes_content() {
    let mut ed = editor_from("(-[f]>oo) (bar) baz\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('a'));
    ed.feed_key(key('('));
    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        " (bar) baz\n",
        "ma( then d must remove (foo)"
    );

    ed.feed_key(key('w'));
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // replay ma( + d on "(bar)"
    assert_eq!(
        ed.doc().text().to_string(),
        "  baz\n",
        "`.` must replay ma( + d and remove (bar)",
    );
}

/// `mi(` (select inner `()`, excluding the delimiters). Same characterization
/// note as `dot_repeat_of_match_around_deletes_content` above: on `(foo)` it
/// already leaves a non-collapsed selection, so this is coverage for
/// `SelectionTracking::Establishes` rather than a regression test for it.
#[test]
fn dot_repeat_of_match_inner_deletes_content() {
    let mut ed = editor_from("(-[f]>oo) (bar) baz\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('('));
    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        "() (bar) baz\n",
        "mi( then d must remove 'foo'"
    );

    ed.feed_key(key('w'));
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // replay mi( + d on "(bar)"
    assert_eq!(
        ed.doc().text().to_string(),
        "() () baz\n",
        "`.` must replay mi( + d and remove 'bar'",
    );
}

#[test]
fn dot_repeat_of_select_all_matches_deletes_content() {
    let mut ed = editor_from("-[f]>oo bar baz foo bar baz\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/'));

    assert_eq!(state(&ed), "-{foo}> bar baz -[foo]> bar baz\n");
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "select-all-matches must establish a 1-step recipe"
    );

    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        " bar baz  bar baz\n",
        "m/ after a search then d must remove all occurrences of 'foo'"
    );

    ed = ed.with_search_regex("bar");
    ed.feed_key(key('.')); // replay m/ + d
    assert_eq!(
        ed.doc().text().to_string(),
        "  baz   baz\n",
        "`.` must replay m/ + d and remove all occurrences of 'bar'"
    );
}

/// `,` (`keep-primary-selection`) after `m/` must APPEND onto the recipe `m/`
/// already built, not reset it: it reduces whatever is staged to the
/// primary selection rather than establishing a fresh extent of its own.
///
/// Buffer "foo bar foo bar foo bar": `m/` selects all three "foo" matches,
/// `,` keeps only the first (primary), `d` deletes it. `.` must replay
/// `m/` + `,` + `d`, re-selecting all remaining "foo" matches, keeping the
/// (new) primary, and deleting it too.
#[test]
fn dot_repeat_of_keep_primary_selection_composes_onto_the_prior_recipe() {
    let mut ed = editor_from("-[f]>oo bar foo bar foo bar\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/')); // select all three "foo" matches
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "m/ must establish a 1-step recipe"
    );

    ed.feed_key(key(',')); // keep-primary-selection: reduce to the first match
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        ", must compose onto the m/ step, not reset it"
    );
    assert_eq!(
        ed.state.selection_recipe[0].command.as_ref(),
        "select-all-matches"
    );
    assert_eq!(
        ed.state.selection_recipe[1].command.as_ref(),
        "keep-primary-selection"
    );

    ed.feed_key(key('d')); // delete the first "foo"
    assert_eq!(ed.doc().text().to_string(), " bar foo bar foo bar\n");

    ed.feed_key(key('.')); // replay m/ + , + d
    assert_eq!(
        ed.doc().text().to_string(),
        " bar  bar foo bar\n",
        "`.` must replay m/ + , + d and remove the next 'foo'"
    );
}

/// `S` (`split-selection-on-newlines`) after `x` + `Ctrl-x` must APPEND onto
/// the recipe those steps already built, not reset it: it splits whatever
/// multi-line extent is staged into per-line pieces rather than establishing
/// a fresh extent of its own.
///
/// Buffer "aaa\nbbb\nccc\nddd\n": `x` selects "aaa\n", `Ctrl-x` extends to
/// "aaa\nbbb\n", `S` splits into two per-line selections ("aaa" and "bbb\n").
/// `d` deletes both: the first piece excludes its line's newline (so "aaa"
/// → "", newline kept), the second piece runs through its own trailing
/// newline (so "bbb\n" is removed whole). Net effect: one line disappears:
/// "\nccc\nddd\n". `.` must replay `x` + `Ctrl-x` + `S` + `d` on the next two
/// lines too, removing another: "\n\n".
#[test]
fn dot_repeat_of_split_selection_on_newlines_composes_onto_the_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key_ctrl('x')); // extend: "aaa\nbbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        "setup: x + Ctrl-x must push two recipe steps"
    );

    ed.feed_key(key('S')); // split into "aaa" + "bbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        3,
        "S must compose onto the x/Ctrl-x steps, not reset them"
    );
    assert_eq!(
        ed.state.selection_recipe[2].command.as_ref(),
        "split-selection-on-newlines"
    );

    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "\nccc\nddd\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay x + Ctrl-x + S + d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n\n",
        "`.` must replay x + Ctrl-x + S + d and clear 'ccc'/'ddd'"
    );
}

/// `mii` (`select-last-insertion`) is an `EditorCmd` that does not opt into
/// the dot-repeat recipe via `.establishes_selection()` (unlike
/// `select-all-matches`; see `editor_cmds.rs`), so unlike `ms(`/`mm` it can never push itself onto
/// `state.selection_recipe`, and (being non-repeatable) it can never overwrite
/// the recorded change either. Running it between an insert and `.` must
/// therefore be inert: `.` still replays the original insert verbatim, not
/// some "reselect + act" recipe.
///
/// `i "ab" Esc` on "x" gives "abx"; `mii` re-selects "ab"
/// (the just-typed span, unrelated to where `.` will act); replaying the
/// insert places "ab" again at the selection's start → "ab" + "ab" + "x" =
/// "ababx". The buffer only grows (no delete ever happens), which is the
/// signal that `.` ran the insert and nothing resembling "mii + d".
#[test]
fn dot_repeat_after_select_last_insertion_still_repeats_the_insert() {
    let mut ed = editor_from("-[x]>\n");

    type_text(&mut ed, "ab"); // insert-before, cursor collapses to start; buffer is "abx"
    assert_eq!(ed.doc().text().to_string(), "abx\n");

    // Move off "ab" first: `insert-select-on-exit` (default on) already left
    // it selected, so `mii` right away would change nothing.
    set_cursor(&mut ed, 2); // onto 'x'
    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i')); // mii: re-select "ab", the last insertion
    assert_eq!(state(&ed), "-[ab]>x\n");

    ed.feed_key(key('d'));
    assert_eq!(state(&ed), "-[x]>\n");

    type_text(&mut ed, "ab");
    assert_eq!(ed.doc().text().to_string(), "abx\n");

    ed.feed_key(key('.')); // must replay the insert, not act on the mii selection
    assert_eq!(ed.doc().text().to_string(), "ababx\n");
}

/// `select-word-nearest-on-line` is the wrap-aware `EditorCmd` twin of
/// `select-word` (`mm`, a `Selection` → `Establishes`), with the same in-place
/// establishing semantics, `EditorCmd` only because it needs a `DisplayLineMap`.
/// Unbound by default, so this drives it directly via `execute_keymap_command`
/// rather than a keypress. It must be replayable: `.` re-runs the selection
/// step before repeating the edit, not just re-delete whatever selection
/// happens to remain at the new cursor.
///
/// Buffer "foo bar\n": select-word-nearest-on-line selects "foo " (word plus
/// trailing whitespace, the `word-selects-whitespace` default), `d` deletes
/// it, leaving "bar\n" with the cursor collapsed to a 1-char selection on
/// 'b'. `.` must replay select-word-nearest-on-line + d from there too,
/// re-selecting the whole word "bar" rather than deleting just that 1-char
/// cursor.
#[test]
fn dot_repeat_of_select_word_nearest_on_line_deletes_the_word() {
    let mut ed = editor_from("-[f]>oo bar\n");

    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "bar\n");

    ed.feed_key(key('.')); // replay select-word-nearest-on-line + d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "`.` must replay select-word-nearest-on-line + d and remove the whole word 'bar'"
    );
}

/// `x m/ d` where `m/`'s pattern matches nothing must record `[select-line,
/// delete]`, not `[select-all-matches, delete]`: `select-all-matches`
/// errors on no match (`cmd_select_all_matches` returns `Err("no matches")`)
/// and leaves the selection untouched, so it established no replayable
/// extent of its own and must not overwrite the `x` step that did.
///
/// Buffer "aaa\nbbb\nccc\n": `x` selects "aaa\n"; search pattern "zzz"
/// matches nothing; `m/` no-ops (still "aaa\n" selected); `d` deletes it,
/// leaving "bbb\nccc\n". Move to "ccc"; `.` must replay `select-line` + `d`
/// there too, leaving "bbb\n".
///
/// `update_recipe`'s `selection_changed` gate leaves the recipe at
/// `[select-line]` after `m/`'s failed run.
#[test]
fn dot_repeat_of_failed_select_all_matches_preserves_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n").with_search_regex("zzz");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key('m'));
    ed.feed_key(key('/')); // no match: selection untouched, command errors
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "a failed select-all-matches must not overwrite the prior x step"
    );
    assert_eq!(ed.state.selection_recipe[0].command.as_ref(), "select-line");

    ed.feed_key(key('d')); // delete "aaa\n"
    assert_eq!(ed.doc().text().to_string(), "bbb\nccc\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay select-line + d
    assert_eq!(
        ed.doc().text().to_string(),
        "bbb\n",
        "`.` must replay select-line + d and remove the whole 'ccc' line"
    );
}

/// `m/ d` recorded under one pattern, replayed under a pattern with no match:
/// the recipe's `select-all-matches` fails on replay, so `.` must not run `d`
/// on the cursor it left behind, and leaves no undo step of its own.
#[test]
fn dot_repeat_whose_recipe_step_fails_leaves_text_and_selections_unchanged() {
    let mut ed = editor_from("-[f]>oo bar baz\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/'));
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), " bar baz\n");

    ed = ed.with_search_regex("qux");
    let before = state(&ed);
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), " bar baz\n");
    assert_eq!(state(&ed), before);
    assert!(
        status(&ed).contains("no matches"),
        "status: {:?}",
        status(&ed)
    );

    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "foo bar baz\n");
}

/// `m/ d` replayed with no search pattern at all: `select-all-matches`
/// refuses, so `.` must not run `d` on the cursor.
#[test]
fn dot_repeat_whose_recipe_step_has_no_search_pattern_leaves_text_unchanged() {
    let mut ed = editor_from("-[f]>oo bar baz\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/'));
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), " bar baz\n");

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).search_pattern = None;
    assert_eq!(ed.state.registers.search_register(), None);
    let before = state(&ed);
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), " bar baz\n");
    assert_eq!(state(&ed), before);
    assert!(
        status(&ed).contains("no search pattern"),
        "status: {:?}",
        status(&ed)
    );
}

/// `x ms( d` where `ms(` finds no surrounding pair must record
/// `[select-line, delete]`, not `[surround-paren, delete]`: `surround-paren`
/// is infallible and leaves the selection unchanged on no-match (see
/// `select_surround` in `hume-ops/src/surround.rs`), so like the
/// `select-all-matches` case above it established no replayable extent and
/// must not overwrite the prior `x` step.
///
/// Buffer "aaa\nbbb\nccc\n": `x` selects "aaa\n" (no parens on that line, so
/// `ms(` on it would no-op); `d` deletes it, leaving "bbb\nccc\n". Move to
/// "ccc"; `.` must replay `select-line` + `d` there too, leaving "bbb\n".
///
/// `update_recipe`'s `selection_changed` gate leaves the recipe at
/// `[select-line]` after the no-op `ms(`.
#[test]
fn dot_repeat_of_noop_surround_preserves_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key('m'));
    ed.feed_key(key('s'));
    ed.feed_key(key('(')); // no parens on this line: selection untouched
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "a no-op surround-paren must not overwrite the prior x step"
    );
    assert_eq!(ed.state.selection_recipe[0].command.as_ref(), "select-line");

    ed.feed_key(key('d')); // delete "aaa\n"
    assert_eq!(ed.doc().text().to_string(), "bbb\nccc\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay select-line + d
    assert_eq!(
        ed.doc().text().to_string(),
        "bbb\n",
        "`.` must replay select-line + d and remove the whole 'ccc' line"
    );
}

/// `.` replaying a paste-ring cycle (`[`/`]`) must actually cycle the ring
/// again, not no-op.
///
/// `repeat-last-action`'s own dispatch must not commit (and thereby close)
/// the still-open paste session before `replay_dot` looks at what it is
/// replaying. See `.defers_paste_commit()` on
/// `repeat-last-action`'s registration (`registry/defaults/editor_cmds.rs`)
/// and `Editor::replay_dot`'s own paste-commit step.
///
/// `KillRing::cycle_position()` is the ring's own cursor,
/// read directly rather than inferred from which text ended up pasted.
#[test]
fn dot_repeat_of_paste_ring_cycle_advances_the_ring() {
    let mut ed = editor_from("-[aaa]> bbb ccc\n");

    ed.feed_key(key('y')); // yank "aaa" → ring: [aaa]
    ed.feed_key(key('w')); // select "bbb"
    ed.feed_key(key('y')); // yank "bbb" → ring: [bbb, aaa]
    ed.feed_key(key('w')); // select "ccc"
    ed.feed_key(key('y')); // yank "ccc" → ring: [ccc, bbb, aaa]

    ed.feed_key(key('p')); // bare paste: reads ring slot 0 ("ccc"), seeds cycle Some(0)
    assert_eq!(
        ed.state.kill_ring.cycle_position(),
        Some(0),
        "setup: p must seed the cycle at slot 0"
    );

    ed.feed_key(key('[')); // paste-ring-older: Some(0) -> Some(1) ("bbb")
    assert_eq!(
        ed.state.kill_ring.cycle_position(),
        Some(1),
        "setup: [ must cycle to slot 1"
    );

    ed.feed_key(key('.')); // replay paste-ring-older: must cycle Some(1) -> Some(2) ("aaa")
    assert_eq!(
        ed.state.kill_ring.cycle_position(),
        Some(2),
        "`.` must replay paste-ring-older as a second cycle step, not no-op"
    );
}
