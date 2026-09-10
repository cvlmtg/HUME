// `object-jump-align` — re-aligning the viewport after a forward object jump
// (`}`, `goto-next-<kind>`). See `ObjectJumpAlign`'s own doc
// (`hume-editor/src/settings.rs`) for why only the forward motions read it.

use super::*;

/// `n` one-line paragraphs, each followed by a blank line — paragraph `i`
/// starts at buffer line `2 * i`. Unwrapped, so a display row is a buffer
/// line and `viewport.top_line` is directly comparable to it.
fn paragraph_editor(n: usize) -> Editor {
    let content: String = (0..n).map(|i| format!("para{i}\n\n")).collect();
    unwrapped_editor(&content, 0)
}

/// Types a decimal `count` as individual digit keys, then `ch`.
fn key_count(ed: &mut Editor, count: usize, ch: char) {
    for digit in count.to_string().chars() {
        ed.handle_key(key(digit));
    }
    ed.handle_key(key(ch));
}

#[test]
fn goto_next_paragraph_centers_view_by_default() {
    // 20 paragraphs (40 lines); default viewport is 80×24 (`for_testing`).
    let mut ed = paragraph_editor(20);
    // 15 forward paragraph steps from paragraph 0 land on paragraph 15,
    // whose content line is 2*15 = 30.
    key_count(&mut ed, 15, '}');

    let head = ed.current_selections().primary().head();
    assert_eq!(
        head,
        co(ed.doc().text().rope().line_to_char(30)),
        "sanity: head lands on paragraph 15's first line"
    );
    // height=24, target=height/2=12 → top_line = 30 - 12 = 18.
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(18)
    );
    assert_eq!(ed.viewport().top_row_offset, 0);
}

#[test]
fn goto_next_paragraph_centers_view_in_extend_mode() {
    // `aligns_view` doesn't gate on `MotionMode` — Extend mode re-centers
    // too. Unlike Move mode (which lands the head on each found paragraph's
    // *start*), Extend unions each step's span into the growing selection
    // and lands the head on the *end* of the last one reached — 15 steps
    // from paragraph 0 lands one char before paragraph 16 begins, at the
    // end of paragraph 15's own block (its trailing gap included).
    let mut ed = paragraph_editor(20);
    ed.execute_keymap_command("goto-next-paragraph".into(), Some(15), true);

    let head = ed.current_selections().primary().head();
    assert_eq!(
        head,
        co(ed.doc().text().rope().line_to_char(32) - 1),
        "sanity: head lands one char before paragraph 16 begins"
    );
    // head's line is 31 (paragraph 15's own gap line); height=24,
    // target=height/2=12 → top_line = 31 - 12 = 19.
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(19)
    );
}

#[test]
fn goto_next_paragraph_count_past_the_last_paragraph_still_centers() {
    // 20 paragraphs (last one's content line is 2*19 = 38); a count of 50
    // exhausts the forward jumps after the 19th — `apply_object_motion`
    // breaks out of its loop on the first `None` — landing on the last
    // paragraph rather than erroring or overshooting past it.
    let mut ed = paragraph_editor(20);
    ed.execute_keymap_command("goto-next-paragraph".into(), Some(50), false);

    let head = ed.current_selections().primary().head();
    assert_eq!(
        head,
        co(ed.doc().text().rope().line_to_char(38)),
        "sanity: a count past the last paragraph clamps to it"
    );
    // height=24, target=height/2=12 → top_line = 38 - 12 = 26.
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(26)
    );
}

#[test]
fn goto_prev_paragraph_never_aligns_view() {
    let mut ed = paragraph_editor(20);
    seek_to_line(&mut ed, 30);
    let top_before = ed.viewport().top_line;

    ed.handle_key(key('{'));

    // `{` (goto-prev-paragraph) doesn't set `CmdMeta::aligns_view` — no
    // synchronous viewport write happens on this keypress (unlike `}`
    // above, which writes it directly without needing a frame).
    assert_eq!(
        ed.viewport().top_line,
        top_before,
        "a backward paragraph jump must not touch the viewport synchronously"
    );
}

#[test]
fn goto_next_paragraph_at_end_of_buffer_does_not_move_the_viewport() {
    let mut ed = paragraph_editor(20);
    // Land on the last paragraph (line 38) — this jump centers.
    key_count(&mut ed, 19, '}');
    // Push the viewport somewhere centering would visibly undo, so a missing
    // `moved` guard has something to disagree with.
    ed.execute_keymap_command("top-view-on-cursor".into(), None, false);
    let top_before = ed.viewport().top_line;

    ed.handle_key(key('}')); // no paragraph below — a true no-op

    assert_eq!(
        ed.viewport().top_line,
        top_before,
        "a no-op `}}` press (already at the last paragraph) must not \
         yank the viewport around"
    );
}

#[test]
fn object_jump_align_top_setting() {
    let mut ed = paragraph_editor(20);
    run_set(&mut ed, "global object-jump-align=top")
        .expect("object-jump-align=top must be accepted");

    key_count(&mut ed, 15, '}');

    // target_row = 0 → top_line = the cursor's own line, no scrolloff
    // applied yet (that only happens on the next frame — see below).
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(30)
    );
    assert_eq!(ed.viewport().top_row_offset, 0);

    // The next frame's per-pane scroll (`scrolloff`) pulls the cursor back
    // down from row 0 to row `scrolloff`, exactly as `z k` already settles
    // — `Top` is not a stable resting point the way `Center` is.
    frame(&mut ed, 80, 24);
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(30 - ed.state.settings.scrolloff)
    );
}

#[test]
fn object_jump_align_off_setting_restores_old_behavior() {
    let mut ed = paragraph_editor(20);
    run_set(&mut ed, "global object-jump-align=off")
        .expect("object-jump-align=off must be accepted");

    key_count(&mut ed, 15, '}');

    // No synchronous viewport write at all — the dispatch pipeline's
    // `step_align_view` is a no-op under `Off`.
    assert_eq!(ed.viewport().top_line, hume_rope::line::ContentLine::new(0));

    // The old (pre-feature) per-frame `scrolloff` scroll still runs and
    // still parks the cursor at `height - scrolloff - 1` rows from the top.
    frame(&mut ed, 80, 24);
    let scrolloff = ed.state.settings.scrolloff;
    let height = ed.viewport().height as usize;
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(30 - (height - scrolloff - 1))
    );
}

#[test]
fn goto_next_function_centers_view_by_default() {
    // Reuses `structural.rs`'s own `rust` grammar + textobjects.scm fixture
    // — the same one `goto-next-function`'s own correctness tests use —
    // rather than re-deriving fixture-loading logic here.
    let filler: String = "// filler\n".repeat(20);
    let src = format!("-[/]>/ x\n{filler}fn target() {{}}\n");
    let mut ed = super::structural::rust_editor(&src);
    // Every line here is short enough that the global wrap default wouldn't
    // actually wrap it, but pin explicitly so the `top_line` assertion below
    // doesn't silently start depending on that coincidence.
    pin_no_wrap(&mut ed);

    ed.execute_keymap_command("goto-next-function".into(), None, false);

    assert_eq!(
        ed.current_selections().primary().head(),
        co(ed.doc().text().rope().line_to_char(21)),
        "sanity: head lands on the target function's first line"
    );
    // height=24, target=height/2=12 → top_line = 21 - 12 = 9.
    assert_eq!(ed.viewport().top_line, hume_rope::line::ContentLine::new(9));
    assert_eq!(ed.viewport().top_row_offset, 0);
}

#[test]
fn object_jump_align_rejects_invalid_value() {
    let mut ed = paragraph_editor(1);
    let result = run_set(&mut ed, "global object-jump-align=middle");
    assert!(
        result.is_err(),
        "an unrecognized alignment must be rejected"
    );
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("top") && msg.contains("center") && msg.contains("off"),
        "error must name the three valid values; got: {msg}"
    );
}
