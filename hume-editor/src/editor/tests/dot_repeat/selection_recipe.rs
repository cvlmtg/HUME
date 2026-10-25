//! The selection recipe `.` replays before the edit.

use super::*;
use pretty_assertions::assert_eq;

// ── Selection-recipe dot-repeat tests ────────────────────────────────────────

/// `x d` (select-line, delete) records recipe `[select-line F]`. Pressing `.`
/// on the next line re-selects the whole line and deletes it.
///
/// Three-line buffer: the first `x d` leaves two lines and the second leaves
/// one.
#[test]
fn dot_repeats_select_line_delete() {
    // Three lines; cursor on 'a'.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select-line → "aaa\n" selected
    ed.feed_key(key('d')); // delete "aaa\n" → "bbb\nccc\n", cursor on 'b'
    assert_eq!(ed.doc().text().to_string(), "bbb\nccc\n");

    // The recipe must be [select-line] (not empty).
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        1,
        "recipe must contain select-line step"
    );

    // `.` replays: re-select current line ("bbb\n"), delete it.
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ccc\n",
        "`.` must re-select the current line and delete it"
    );
}

/// `x Ctrl-x d` selects two lines (one establish + one extend) and deletes them.
/// `.` replays the full two-step recipe, deleting the next two lines.
///
/// Four-line buffer: the first `x Ctrl-x d` leaves two lines, and the replay
/// deletes both, so one structural line remains.
#[test]
fn dot_repeats_extend_select_delete() {
    // Four lines; cursor on 'a'.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('x')); // select "aaa\n"
    ed.feed_key(key_ctrl('x')); // extend to "aaa\nbbb\n"
    ed.feed_key(key('d')); // delete → "ccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "ccc\nddd\n");

    // Recipe must be [select-line F, select-line T] (establish + one extend).
    let recipe = &ed.state.last_action().unwrap().selection_recipe;
    assert_eq!(recipe.len(), 2, "recipe must have 2 steps");
    assert!(!recipe[0].extend, "first step must be Move (establish)");
    assert!(recipe[1].extend, "second step must be Extend");

    // `.` replays: x (select "ccc\n") + Ctrl-x (extend to "ccc\nddd\n") + d.
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "`.` must re-select two lines and delete them"
    );
}

/// Navigation (`j` = move-down) before `x d` must NOT appear in the recipe.
/// Only the `x` (establish) step is recorded; `.` does NOT move down first.
///
/// Four-line buffer; `j x d` deletes line 1 ("bbb\n"),
/// leaving "aaa\nccc\nddd\n". `.` must re-select line 1 ("ccc\n") and delete
/// it, giving "aaa\nddd\n".
///
/// The `!ctx.extend` check in `update_recipe`'s `SelectionTracking::Extends`
/// arm keeps `j` (a Motion in Move mode) out of the recipe.
#[test]
fn dot_repeat_navigation_not_in_recipe() {
    // Four lines so that the last-line structural-'\n' protection does not
    // interfere with any of the delete operations below.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('j')); // move-down: collapsed cursor on 'b', recipe cleared
    ed.feed_key(key('x')); // select-line "bbb\n": recipe = [select-line F]
    ed.feed_key(key('d')); // delete "bbb\n" → "aaa\nccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "aaa\nccc\nddd\n");

    // Recipe must have exactly one step (x only; j must have been cleared).
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        1,
        "navigation (j) must not appear in the selection recipe"
    );

    // `.` replays x (selects current line "ccc\n") + d (deletes it).
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "aaa\nddd\n",
        "`.` must re-select and delete the current line, NOT move down first"
    );
}

/// `j d` (navigate then delete a collapsed single-char selection) records an
/// empty recipe. `.` replays just the delete on whatever the current selection is.
#[test]
fn dot_repeat_collapsed_cursor_empty_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('j')); // move-down: collapsed on 'b', recipe cleared
    ed.feed_key(key('d')); // delete 'b' (1-char selection), recipe = []
    assert_eq!(ed.doc().text().to_string(), "aaa\nbb\nccc\n");

    // Recipe must be empty.
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        0,
        "empty recipe for plain single-char delete"
    );

    // `.` repeats just the delete (no line reselection).
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "aaa\nb\nccc\n",
        "`.` must delete the current 1-char selection, not reselect a whole line"
    );
}

/// `x c <text> Esc` records recipe=[select-line F] + transcript=[...]. `.` on
/// another line re-selects the full line, runs change, and retypes the text.
///
/// Three-line buffer; `x c z Esc` on line 0 replaces "aaa" with 'z' and keeps
/// the line's '\n' → "z\nbbb\nccc\n". Then `.` on line 1 re-selects "bbb\n"
/// via the recipe, runs change, and inserts 'z' → "z\nz\nccc\n".
#[test]
fn dot_repeats_change_reselects_line() {
    // Three lines; `c` after select-line removes the content but keeps the `\n`.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select "aaa\n", head on '\n'
    ed.feed_key(key('c')); // change: delete "aaa" (not '\n') → "\nbbb\nccc\n", cursor at 0
    ed.feed_key(key('z')); // type 'z' → "z\nbbb\nccc\n"
    ed.feed_key(key_esc()); // back to Normal, cursor on 'z'
    assert_eq!(ed.doc().text().to_string(), "z\nbbb\nccc\n");

    // Recipe must be [select-line]; transcript must be ['z', Esc].
    {
        let action = ed.state.last_action().unwrap();
        assert_eq!(
            action.selection_recipe.len(),
            1,
            "recipe must have select-line step"
        );
        assert_eq!(
            action.transcript.len(),
            2,
            "transcript must capture the typed char and the Esc binding"
        );
    }

    // Move to 'b' and press `.`.
    ed.feed_key(key('j')); // move-down to 'b' (line 1)
    ed.feed_key(key('.')); // re-select "bbb\n" via recipe, change, retype 'z'

    // Recipe re-selects "bbb\n", change deletes "bbb" (keeps '\n'),
    // inserts 'z' → "z\nz\nccc\n".
    assert_eq!(
        ed.doc().text().to_string(),
        "z\nz\nccc\n",
        "`.` must re-select the full line, change it, and replay transcript"
    );
}

/// `undo` must clear the selection-recipe buffer so a stale `select-line` does
/// not leak into the next edit's recipe.
///
/// `undo` is `Untracked`, and the `SelectionTracking::Untracked` early-return
/// in `update_recipe` (`replay/recipe.rs`) is what clears the recipe.
#[test]
fn undo_clears_selection_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\n");

    // Establish a recipe via `x`.
    ed.feed_key(key('x'));
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "setup: x must establish a 1-step recipe"
    );

    // `u` (undo) is a non-repeatable EditorCmd → must clear the recipe.
    ed.feed_key(key('u'));
    assert_eq!(
        ed.state.selection_recipe.len(),
        0,
        "undo must clear the selection recipe buffer"
    );
}

/// `x C` (select-line, then duplicate the selection onto the next line) must
/// APPEND `copy-selection-on-next-line` onto the recipe `x` already built,
/// not reset it to a single `[C]` step.
///
/// `copy-selection-on-next-line` duplicates whatever selection is already
/// there: it establishes nothing on its own. It is registered with
/// `.composes_selection()` (`SelectionTracking::Composes`) because it
/// transforms the staged extent, so the `x` that built that extent stays in
/// the recipe (see `registry/defaults/selections.rs`, and
/// `dot_repeat_of_copy_selection_replays_the_whole_recipe` below for the
/// end-to-end replay).
#[test]
fn copy_selection_on_next_line_appends_to_the_selection_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\n");

    ed.feed_key(key('x')); // select-line: "aaa\n" selected
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "setup: select-line must establish a 1-step recipe"
    );

    ed.feed_key(key('C')); // duplicate the selection onto "bbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        "copy-selection-on-next-line must append onto the prior x step"
    );
    assert_eq!(ed.state.selection_recipe[0].command.as_ref(), "select-line");
    assert_eq!(
        ed.state.selection_recipe[1].command.as_ref(),
        "copy-selection-on-next-line"
    );
}

/// `mm C d` then `.` must replay the WHOLE recipe (`mm`, `C`, `d`), not just
/// `d` against whatever selection happens to remain.
///
/// `mm` (word-select) is used as the establish step rather than `x`
/// (select-line): `select-line` includes the line's trailing structural
/// newline in the selection, and `copy-selection-on-next-line`'s
/// `DisplayColTarget::NearestContent` clamps a display column landing past
/// content back onto the last content char, so a `select-line` head (which
/// sits ON that newline) does not reproduce the newline in the copy. That's
/// existing, correct `C` behavior, orthogonal to this test; a word selection
/// (all content chars, no newline in range) sidesteps it entirely.
///
/// On `"foo\nbar\nfoo\nbar\n"`, `mm` selects "foo" (line
/// 0), `C` duplicates it onto "bar" (line 1) at the same columns, `d`
/// deletes both: each line's own newline survives, leaving
/// `"\n\nfoo\nbar\n"`. Move down two buffer lines to the second "foo\nbar\n"
/// pair; `.` must replay `mm` + `C` + `d` there too, leaving `"\n\n\n\n"`.
#[test]
fn dot_repeat_of_copy_selection_replays_the_whole_recipe() {
    let mut ed = editor_from("-[f]>oo\nbar\nfoo\nbar\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('m')); // select-word: "foo" (line 0)
    ed.feed_key(key('C')); // + "bar" (line 1)
    ed.feed_key(key('d')); // delete both
    assert_eq!(ed.doc().text().to_string(), "\n\nfoo\nbar\n");

    ed.feed_key(key('j'));
    ed.feed_key(key('j')); // down to the second "foo" line, not in the recipe
    ed.feed_key(key('.')); // replay mm, C, d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n\n\n\n",
        "`.` must replay the full mm/C/d recipe, not just d"
    );
}

/// `f`/`t` are NOT repeatable (they have `=`/`-` for that). Pressing `.`
/// after a find/till motion should be a no-op.
#[test]
fn dot_after_find_is_noop() {
    let mut ed = editor_from("-[h]>ello world\n");

    // `f` + `o` moves cursor to the first 'o' in "hello".
    ed.feed_key(key('f'));
    ed.feed_key(key('o'));
    let state_after_find = state(&ed);

    // `.` should have nothing recorded and leave state unchanged.
    assert!(ed.state.last_action().is_none());
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), state_after_find);
}

/// A dot-repeated delete is itself a fresh capture: a bare `p` right after
/// `.` reads what `.` just deleted, not the clipboard.
///
/// This is a deliberate consequence of `PasteStamp` having no dot-repeat
/// special case at all: `replay_dot` runs the replayed edit through the
/// ordinary `commands::run_body` → `route_kill` → `capture_to_ring`
/// path, which writes the stamp at the replay's own
/// `BufferStore::edit_seq()` exactly as a live `d` would: nothing treats a
/// replayed delete differently from a typed one.
#[test]
fn dot_repeat_of_delete_leaves_ring_fresh_for_paste() {
    let mut ed = editor_from("-[foo]> bar\n");
    // Seed the clipboard with a sentinel value distinct from anything the
    // ring could hold, so a wrong clipboard read is unambiguous.
    seed_clipboard(&mut ed, "CLIP");

    ed.feed_key(key('d')); // delete "foo" → ring head = ["foo"]
    ed.feed_key(key('w')); // move to "bar"
    ed.feed_key(key('.')); // repeat; replay_dot deletes "bar" → ring head = ["bar"]

    ed.feed_key(key('p')); // bare p: must read the ring, not the clipboard
    let text = state(&ed);
    // "bar" is independently known from the scenario itself (the `.` replays
    // a delete on the word we navigated to with `w`), not read back from
    // `kill_ring.head()`.
    assert!(
        text.contains("bar"),
        "p after dot-repeat delete must paste the ring head \"bar\"; buf={text:?}"
    );
    assert!(
        !text.contains("CLIP"),
        "p after dot-repeat delete must not read the clipboard; buf={text:?}"
    );
}

/// `w d` (word motion select then delete) must record an empty selection
/// recipe. A bare `.` then deletes the *current* selection, NOT the next word.
///
/// The `!ctx.extend` exclusion in `update_recipe`'s
/// `SelectionTracking::Extends` arm keeps `w` (a `Motion`, always `Extends`)
/// out of the recipe, so `.` never re-runs `select-next-word` past the
/// intended word.
///
/// Buffer "a  foo bar baz\n". `w` selects "foo", `d` deletes it →
/// "a   bar baz\n". Then `w` selects "bar". `.` must delete "bar" (current
/// selection), leaving "a    baz\n".
#[test]
fn dot_repeat_word_motion_acts_on_current_selection() {
    let mut ed = editor_from("-[a]>  foo bar baz\n");
    // This test is about the recipe/replay mechanism, not word-span shape.
    // Pin bare-word selection so the buffer arithmetic in the doc comment
    // above holds regardless of word-selects-whitespace's default.
    ed.state.settings.word_selects_whitespace = WordSelectsWhitespace::Off;

    ed.feed_key(key('w')); // select "foo" (Move mode: Extends, not Establishes)
    ed.feed_key(key('d')); // delete "foo" → "a   bar baz\n"

    // Recipe must be empty: Move-mode `w` must not create an establish step.
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        0,
        "Move-mode select-next-word must not push an establish step"
    );

    ed.feed_key(key('w')); // select "bar" (cursor now on 'b')
    ed.feed_key(key('.')); // replay: empty recipe → delete current selection ("bar")

    let text = ed.doc().text().to_string();
    assert!(
        !text.contains("bar"),
        "`.` must delete 'bar' (current selection), not advance past it; got: {text:?}"
    );
    assert!(
        text.contains("baz"),
        "`.` must not advance past 'bar' and delete 'baz'; got: {text:?}"
    );
}
