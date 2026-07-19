use super::*;
use crate::editor::dispatch::ArgSource;
use pretty_assertions::assert_eq;

// ── Visual-line movement ──────────────────────────────────────────────────────
//
// `visual_test_editor` pins settings to `WrapMode::Indent { width: 76 }` with
// tab_width=4 and an 80×24 viewport. For a line with no leading indent, Indent
// wrap is equivalent to Soft wrap (indent_cols = 0), so the wrap boundary is
// simply at column 76.
//
// Test layout:
//   Line 0: 'a' × 80  →  sub-row 0: chars  0..76 (cols 0..75)
//                         sub-row 1: chars 76..80 (cols 0..3) + '\n' at col 4
//   Line 1: "short\n"  →  chars 81..86
//
// Char offsets:
//   0      = first 'a'
//   76     = first 'a' on sub-row 1
//   80     = '\n' at end of line 0
//   81     = 's' (start of "short")
//   85     = 't'
//   86     = '\n' at end of line 1

fn visual_test_editor(head: usize) -> Editor {
    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    // Build manually so we can place the cursor at an exact char offset.
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::Text;
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(head));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    // Pin to 76-column indent-wrap so the char-offset expectations in the tests
    // are stable regardless of terminal size. wrap_mode is pane-owned (SSOT).
    ed.view.panes[ed.state.focused_pane_id].wrap_mode =
        hume_engine::pane::WrapMode::Indent { width: 76 };
    ed
}

/// j moves from sub-row 0 to sub-row 1 of the same buffer line.
#[test]
fn visual_move_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        76,
        "j: sub-row 0 → sub-row 1, col 0 → char 76"
    );
    assert_eq!(
        ed.current_selections().primary().horiz(),
        Some(0),
        "sticky col latched on first j"
    );
}

/// j on the last sub-row crosses to the next buffer line.
#[test]
fn visual_move_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "j: last sub-row → first char of next buffer line"
    );
}

/// k from the first row of a buffer line enters the last sub-row of the previous line.
#[test]
fn visual_move_up_enters_last_subrow_of_previous_line() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        76,
        "k: buffer line n+1 → last sub-row of line n, col 0 → char 76"
    );
}

/// k on sub-row 1 retreats to sub-row 0 of the same buffer line.
#[test]
fn visual_move_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "k: sub-row 1 → sub-row 0, col 0 → char 0"
    );
}

/// k on the first sub-row of the first line stays put.
#[test]
fn visual_move_up_at_top_stays_put() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "k at first row: no-op"
    );
}

/// j on the last sub-row of the last line stays put.
#[test]
fn visual_move_down_at_bottom_stays_put() {
    // Place cursor at "short" (line 1 is last). Line 1 has only 1 sub-row.
    let mut ed = visual_test_editor(81);
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "j at last row: no-op"
    );
}

/// The preferred display column is preserved across consecutive j/k presses
/// and used to find the closest grapheme when the target row is shorter.
#[test]
fn visual_preferred_col_stickiness() {
    // Cursor at char 40 (display col 40) in sub-row 0 of the long line.
    let mut ed = visual_test_editor(40);

    // j: target_col = 40, sub-row 1 has only 4 chars (cols 0..3).
    // Closest to col 40 is char 79 (col 3, last 'a' on sub-row 1).
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        79,
        "j: clamped to last char on short sub-row"
    );
    assert_eq!(
        ed.current_selections().primary().horiz(),
        Some(40),
        "sticky col stays at 40"
    );

    // j again: cross to "short\n" (line 1). target_col=40, "short" has cols 0..4.
    // Closest to 40 is 't' at col 4, char 85.
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        85,
        "j: clamped to last char on short second line"
    );
    assert_eq!(
        ed.current_selections().primary().horiz(),
        Some(40),
        "sticky col still 40"
    );
}

/// Any non-vertical command resets preferred_display_col.
#[test]
fn visual_preferred_col_reset_on_horizontal_motion() {
    let mut ed = visual_test_editor(40);
    ed.handle_key(key('j')); // latches horiz on the selection
    assert!(
        ed.current_selections().primary().horiz().is_some(),
        "j latches sticky col"
    );
    ed.handle_key(key('l')); // horizontal motion — Selection::new() clears horiz
    assert!(
        ed.current_selections().primary().horiz().is_none(),
        "l resets sticky col"
    );
}

/// WrapMode::None falls back to buffer-line movement.
#[test]
fn visual_move_no_wrap_falls_back_to_buffer_line() {
    let mut ed = visual_test_editor(0);
    // wrap_mode is pane-owned: apply_visual_vertical reads it via the focused pane.
    ed.view.panes[ed.state.focused_pane_id].wrap_mode = hume_engine::pane::WrapMode::None;

    ed.handle_key(key('j'));
    // With no wrapping: j moves by one buffer line (0 → 81 "short").
    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "WrapMode::None: j moves by buffer line"
    );
    assert!(
        ed.current_selections().primary().horiz().is_none(),
        "no sticky col in non-wrap mode"
    );
}

/// count prefix: 2j moves two BUFFER lines (the second hop is a no-op here —
/// buffer line 2 is the phantom trailing empty line — so it lands on buffer
/// line 1, same char offset a bare `j`,`j` would reach by two visual rows in
/// this particular buffer; see `visual_move_down_with_explicit_count_moves_buffer_lines`
/// for a case where the two paths diverge).
#[test]
fn visual_move_down_with_count() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    // 2j from char 0: first hop → char 81 (buffer line 1); second hop is a
    // no-op (buffer line 2 is the phantom trailing line).
    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "2j: buffer-line movement, clamped at the last real line"
    );
}

/// A count prefix means "N buffer lines", not "N visual rows" — even while
/// wrapping is on. `1j` skips straight to the start of buffer line 1, bypassing
/// the sub-row-1 stop that a bare `j` (no count) lands on.
#[test]
fn visual_move_down_with_explicit_count_moves_buffer_lines() {
    let mut ed = visual_test_editor(0); // sub-row 0, col 0
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "1j: one buffer line skips the sub-row-1 stop entirely"
    );
    assert!(
        ed.current_selections().primary().horiz().is_none(),
        "buffer-line path (preferred_col: None) doesn't set sticky visual column"
    );
}

/// A larger explicit count also moves by buffer lines: `2j` from line 0 lands
/// on the (only) next buffer line, not two visual rows past it.
#[test]
fn visual_move_up_with_explicit_count_moves_buffer_lines() {
    let mut ed = visual_test_editor(81); // start of "short" (buffer line 1)
    ed.handle_key(key('1'));
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "1k: one buffer line lands on line 0 col 0, not the last sub-row (char 76)"
    );
}

/// Scroll commands (page/half-page) always move by display rows, regardless of
/// `explicit_count` — the buffer-vs-visual choice is a parameter passed by the
/// caller (`by_buffer_line`), not a global-state read inside the shared core.
/// This guards against `apply_visual_vertical` accidentally reading
/// `state.explicit_count` itself instead of trusting its parameter.
#[test]
fn apply_visual_vertical_ignores_explicit_count_when_caller_forces_visual() {
    use crate::editor::visual_move::apply_visual_vertical;
    use crate::ops::MotionMode;

    let mut ed = visual_test_editor(0);
    ed.state.explicit_count = true; // simulate "a count was typed"
    apply_visual_vertical(
        &mut ed.state,
        &mut ed.view,
        1,
        true,
        MotionMode::Move,
        false,
    );
    assert_eq!(
        ed.current_selections().primary().head(),
        76,
        "by_buffer_line=false must move one visual row even with explicit_count=true"
    );
}

/// Each cursor uses its own sticky column in multi-cursor j/k.
///
/// Text layout (visual_test_editor):
///   sub-row 0: chars  0..76 (cols 0..75)
///   sub-row 1: chars 76..80 (cols 0..3)  ← two cursors placed here
///   line 1:    chars 81..86 "short\n"
///
/// Cursor A at char 76 (col 0), cursor B at char 79 (col 3, primary).
/// j → line 1: A should land at col 0 = char 81, B at col 3 = char 84.
/// k → sub-row 1: A should return to col 0 = char 76, B to col 3 = char 79.
#[test]
fn visual_move_per_selection_sticky_col() {
    use hume_editing::selection::{Selection, SelectionSet};

    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    let buf = hume_editing::text::Text::from(content.as_str());
    // A at col 0, B at col 3 (primary).
    let sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(76), // A — col 0 on sub-row 1
            Selection::collapsed(79), // B — col 3 on sub-row 1
        ],
        1, // primary is B
    );
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    ed.view.panes[ed.state.focused_pane_id].wrap_mode =
        hume_engine::pane::WrapMode::Indent { width: 76 };

    // j: each cursor should use its own column, not the primary's.
    ed.handle_key(key('j'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    // Sorted by start(): A is first.
    let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head()).collect();
    assert_eq!(heads[0], 81, "A (col 0) → char 81 on line 1");
    assert_eq!(heads[1], 84, "B (col 3) → char 84 on line 1");

    // k: sticky cols should bring each cursor back to its original column.
    ed.handle_key(key('k'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head()).collect();
    assert_eq!(heads[0], 76, "A returns to col 0 = char 76 on sub-row 1");
    assert_eq!(heads[1], 79, "B returns to col 3 = char 79 on sub-row 1");
}

// ── Visual-line extend variants ───────────────────────────────────────────────
//
// Extend mode is toggled with `e`. In extend mode `j`/`k` resolve to
// extend-down/extend-up: the anchor stays fixed and only the head moves.

/// extend-down (e+j) within a wrapped line: anchor stays at sub-row 0, head
/// advances to sub-row 1 of the same buffer line.
#[test]
fn visual_extend_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('e')); // enter extend mode
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), 0, "anchor fixed at sub-row 0 col 0");
    assert_eq!(sel.head(), 76, "head extends to sub-row 1 col 0");
}

/// extend-down crosses to the next buffer line when already on the last sub-row.
#[test]
fn visual_extend_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // last sub-row of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), 76, "anchor fixed at last sub-row");
    assert_eq!(
        sel.head(),
        81,
        "head crosses to first char of next buffer line"
    );
}

/// extend-up (e+k) within a wrapped line: head retreats from sub-row 1 to sub-row 0.
#[test]
fn visual_extend_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), 76, "anchor fixed at sub-row 1");
    assert_eq!(sel.head(), 0, "head retreats to sub-row 0 col 0");
}

/// extend-up enters the last sub-row of the previous buffer line.
#[test]
fn visual_extend_up_enters_previous_line_last_subrow() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), 81, "anchor fixed at line 1 start");
    assert_eq!(
        sel.head(),
        76,
        "head enters last sub-row of previous buffer line"
    );
}

// ── select-word-nearest-on-line: wrap-aware bounds ───────────────────────────
//
// Buffer layout (wrap=76):
//   Line 0: 75 'a's + "+ ratatui\n"  (total 85 chars, 0..84)
//            sub-row 0: chars  0..75  (75 'a's and '+' at col 75)
//            sub-row 1: chars 76..84  (' ' at col 0, "ratatui", '\n')
//   Line 1: "short\n"  (chars 85..90)
//
// Char map:
//   75  = '+'
//   76  = ' '  (leading whitespace of sub-row 1 — the wrap-breaking space)
//   77  = 'r'  (start of "ratatui")
//   83  = 'i'  (end of "ratatui")
//   84  = '\n'
//   85  = 's'  (start of "short")

fn word_wrap_editor() -> Editor {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::Text;
    let content = format!("{}+ ratatui\nshort\n", "a".repeat(75));
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    ed.view.panes[ed.state.focused_pane_id].wrap_mode =
        hume_engine::pane::WrapMode::Indent { width: 76 };
    ed
}

/// `select-word-nearest-on-line` in wrap mode must snap to the word *on the
/// current visual sub-row*, not across the wrap boundary.
///
/// After `move-down` from col 0 of sub-row 0, head lands on the leading space
/// of sub-row 1 (char 76). The nearest-word scan must find "ratatui" (forward,
/// same sub-row), NOT '+' (backward, previous sub-row).
#[test]
fn select_word_nearest_scopes_to_visual_subrow() {
    let mut ed = word_wrap_editor();

    // j: head moves to char 76 (leading space of sub-row 1).
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head(), 76);

    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
        ArgSource::Keymap,
    );

    let sel = ed.current_selections().primary();
    assert_ne!(
        sel.head(),
        75,
        "must NOT snap to '+' across the wrap boundary"
    );
    assert_eq!(
        sel.head(),
        83,
        "must snap to 'ratatui' (last char = 'i' at char 83)"
    );
    assert_eq!(sel.horiz(), Some(0), "horiz preserved through snap");
}

/// Two consecutive `j` + `select-word-nearest-on-line` sequences must advance
/// the head forward — no oscillation. The bug this guards against was:
///   j → head=76 (space); select → head=75 ('+', wrong row);
///   j → head=76 again;   select → head=75 again. (oscillation)
/// With the fix the second select must land strictly past the first.
#[test]
fn select_word_nearest_no_oscillation_on_repeated_j() {
    let mut ed = word_wrap_editor();

    let call_select = |ed: &mut Editor| {
        ed.execute_keymap_command(
            std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
            Some(1),
            false,
            ArgSource::Keymap,
        )
    };

    // First j + select: lands on "ratatui" (head = 83).
    ed.handle_key(key('j'));
    call_select(&mut ed);
    let head_after_first_select = ed.current_selections().primary().head();
    assert_eq!(head_after_first_select, 83);

    // Second j: must advance past 83 (crosses to line 1, sub-row 0 → 's' at 85).
    ed.handle_key(key('j'));
    let head_after_second_j = ed.current_selections().primary().head();
    assert!(
        head_after_second_j > head_after_first_select,
        "second j must advance past {head_after_first_select}; got {head_after_second_j}"
    );

    // Second select: must land strictly past the first select — never back.
    call_select(&mut ed);
    let head_after_second_select = ed.current_selections().primary().head();
    assert!(
        head_after_second_select > head_after_first_select,
        "second select must advance past {head_after_first_select}; got {head_after_second_select} (oscillation)"
    );
}

/// With the default `word-selects-whitespace = true`, the snapped word's
/// leading whitespace bookend is absorbed into the selection (matching `mm`)
/// — even in wrap mode, since the absorbed space (char 76) is buffer-adjacent
/// to "ratatui", not a crossing into a different word.
#[test]
fn select_word_nearest_absorbs_whitespace_bookend_by_default() {
    let mut ed = word_wrap_editor();

    ed.handle_key(key('j')); // head -> char 76 (leading space of sub-row 1)
    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
        ArgSource::Keymap,
    );

    let sel = ed.current_selections().primary();
    assert_eq!(
        sel.anchor(),
        76,
        "leading space must be absorbed, matching mm's word_unit_at rule"
    );
    assert_eq!(sel.head(), 83, "still snaps to 'ratatui'");
}

/// With `word-selects-whitespace` off, the selection stays a bare inner word
/// — no whitespace bookend — even in wrap mode.
#[test]
fn select_word_nearest_respects_word_selects_whitespace_off() {
    let mut ed = word_wrap_editor();
    ed.state.settings.word_selects_whitespace = false;

    ed.handle_key(key('j')); // head -> char 76 (leading space of sub-row 1)
    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
        ArgSource::Keymap,
    );

    let sel = ed.current_selections().primary();
    assert_eq!(
        sel.anchor(),
        77,
        "inner word only — leading space must not be absorbed"
    );
    assert_eq!(sel.head(), 83, "still snaps to 'ratatui'");
}

// ── Dispatch-origin count semantics ────────────────────────────────────────
//
// `move-down`/`move-up`'s buffer-line-vs-visual-row choice comes from
// `CmdCtx.count: Option<usize>` — `None` means "as if no count was typed".
// The keymap trie leaves / WaitChar arm produce `None` for a bare keypress.
// Steel can produce the same `None` explicitly: a script passes a count of
// `0` (the Scheme spelling of "no count"), which `parse_count_extend` decodes
// to `None` before it ever reaches `CmdCtx`. A no-arg `call!`/typed `:move-down`
// still defaults to `Some(1)` (buffer line) — the script has to opt into
// visual-row movement by name (passing `0`), it never happens implicitly.

/// Scripted dispatch (`run_command_sync`, the path behind Steel's `call!`) with
/// an explicit `Some(count)` moves by buffer line — unlike a bare keyboard `j`,
/// which stops at the wrap boundary (see `visual_move_down_within_wrapped_line`,
/// char 76). A script can pass `None` (Steel count `0`) to get visual-row
/// movement instead — see `steel_call_move_down_zero_count_moves_visual_row`.
#[test]
fn run_command_sync_some_count_moves_buffer_line() {
    use hume_scripting::host::CommandHost;

    let mut ed = visual_test_editor(0);
    {
        let mut host = live_host!(ed);
        host.run_command_sync("move-down", Some(1), false, None)
            .expect("run_command_sync must not error for move-down");
    }
    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "scripted move-down (Some(1)) must move a full buffer line, not stop at the wrap boundary"
    );
}

/// A Steel command's own internal `(call! "move-down")` always moves by
/// buffer line regardless of the *outer* key's typed count — the two are
/// dispatched separately, each through its own `run_native_body` call, so
/// the inner one can't inherit the outer's explicitness. This also proves
/// `state.explicit_count` is restored (not left `true`) once the whole
/// dispatch — outer Steel command plus its nested native call — completes.
#[test]
fn steel_call_move_down_ignores_outer_keystrokes_count() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // The body passes no count to `move-down` — if it inherited the outer
    // key's count-or-lack-thereof, this would move a visual row instead.
    host.eval_source_returning_defs(
        r#"(define-command! "steel-move-down" ""
                 (lambda () (call! "move-down")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    // Simulates `5<key>` bound to "steel-move-down": the outer count (5) must
    // have no bearing on the inner call's buffer-line-vs-visual-row choice.
    ed.execute_keymap_command("steel-move-down".into(), Some(5), false, ArgSource::Keymap);

    assert_eq!(
        ed.current_selections().primary().head(),
        81,
        "inner (call! \"move-down\") must move one buffer line, not one visual row \
         and not the outer key's count of 5 buffer lines"
    );
    assert!(
        !ed.state.explicit_count,
        "explicit_count must be restored to its pre-dispatch value (false) after \
         the outer Steel command and its nested native call both complete"
    );
}

/// A Steel wrapper that forwards its own `count`/`extend` params straight into
/// `(call! "move-down" count extend)` must preserve bare-press visual-row
/// movement: dispatching it with `None` (as a keymap trie leaf would for a
/// bare keypress) injects `count = 0` into the lambda, which round-trips back
/// to `None` through `parse_count_extend` — the count-forwarding contract
/// documented in `plugins.md`'s "Calling other commands" section.
///
/// Fail oracle: a `run_steel_command` that injects `ctx.count.unwrap_or(1)`
/// instead of `0` would make the lambda see `1` and always move the buffer
/// line (head 81), never 76.
#[test]
fn steel_wrapper_bare_dispatch_moves_visual_row() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);
    let mut host = ScriptingHost::new();
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-jk" ""
                 (lambda (count extend) (call! "move-down" count extend)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    // Simulates a bare `<key>` bound to "steel-jk": no count was typed.
    ed.execute_keymap_command("steel-jk".into(), None, false, ArgSource::Keymap);

    assert_eq!(
        ed.current_selections().primary().head(),
        76,
        "bare dispatch through a forwarding Steel wrapper must move one visual \
         row (char 76), not one buffer line (char 81)"
    );
}

/// The same wrapper with an explicit count still moves by buffer line —
/// forwarding preserves both behaviors, not just the visual-row one.
///
/// Buffer: wrapped 80-char line 0, then three short lines "b"/"c"/"d" (chars
/// 81/83/85). From char 0, 3 buffer lines lands on 'd' (85); 3 *visual* rows
/// (sub-row 1, then "b", then "c") would land on 'c' (83) instead — the two
/// outcomes are distinguishable, so this pins `by_buffer_line`, not just count.
#[test]
fn steel_wrapper_explicit_count_moves_buffer_lines() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::Text;
    use hume_scripting::ScriptingHost;

    let line0: String = "a".repeat(80);
    let content = format!("{line0}\nb\nc\nd\n");
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    ed.view.panes[ed.state.focused_pane_id].wrap_mode =
        hume_engine::pane::WrapMode::Indent { width: 76 };

    let mut host = ScriptingHost::new();
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-jk" ""
                 (lambda (count extend) (call! "move-down" count extend)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("steel-jk".into(), Some(3), false, ArgSource::Keymap);

    assert_eq!(
        ed.current_selections().primary().head(),
        85,
        "3<key> through the forwarding wrapper must move 3 buffer lines (char \
         85), not 3 visual rows (char 83)"
    );
}

/// `(call! "move-down" 0)` from inside a Steel command body moves by visual
/// row regardless of the *outer* dispatch's count — a script can ask for
/// visual-row movement explicitly, not just by forwarding a bare keypress.
/// Also confirms `explicit_count` is restored afterward (mirrors
/// `steel_call_move_down_ignores_outer_keystrokes_count`).
#[test]
fn steel_call_move_down_zero_count_moves_visual_row() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);
    let mut host = ScriptingHost::new();
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-vis" ""
                 (lambda () (call! "move-down" 0)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    // Outer count is Some(1) — irrelevant, since the body hardcodes 0.
    ed.execute_keymap_command("steel-vis".into(), Some(1), false, ArgSource::Keymap);

    assert_eq!(
        ed.current_selections().primary().head(),
        76,
        "(call! \"move-down\" 0) must move one visual row (char 76), not one \
         buffer line (char 81)"
    );
    assert!(
        !ed.state.explicit_count,
        "explicit_count must be restored to its pre-dispatch value (false) \
         after the command completes"
    );
}

/// The bare-name wrapper generated by `register_command_names` is variadic —
/// `(move-down 0)` (no `call!`, no wrapper lambda) must also decode `0` to
/// visual-row movement. This exercises the generated
/// `(lambda args (%dispatch-command "move-down" args))` binding directly,
/// distinct from the `call!`-macro path the other tests use.
#[test]
fn generated_bare_name_wrapper_accepts_zero_count() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);
    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-vis-direct" ""
                 (lambda () (move-down 0)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("steel-vis-direct".into(), Some(1), false, ArgSource::Keymap);

    assert_eq!(
        ed.current_selections().primary().head(),
        76,
        "(move-down 0) via the generated variadic wrapper must move one \
         visual row (char 76), not one buffer line (char 81)"
    );
}
