use super::*;
use pretty_assertions::assert_eq;

// ── View-trie scroll (z z / z k / z j) ────────────────────────────────────────
//
// `z z` centres the cursor row, `z k` puts it at the top, `z j` puts it at
// the bottom. Cursor position is unchanged — only the viewport moves.
//
// for_testing gives an 80×24 viewport. With 50 single-char lines "a\n" the
// content is 100 chars and char 2*N is the start of line N.

fn view_test_editor() -> Editor {
    let content = "a\n".repeat(50);
    unwrapped_editor(&content, 0)
}

// ── Unwrapped mode ────────────────────────────────────────────────────────────

#[test]
fn zz_centres_cursor_in_viewport() {
    let mut ed = view_test_editor();
    seek_to_line(&mut ed, 25);
    ed.handle_key(key('z'));
    ed.handle_key(key('z'));
    // height=24, target=12; cursor on line 25 → top_line = 25 - 12 = 13.
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(13)
    );
    assert_eq!(ed.viewport().top_slot, 0);
    // Cursor is unchanged.
    assert_eq!(
        ed.current_selections().primary().head(),
        co(hume_rope::lines::line_start_char(
            ed.doc().text().rope(),
            hume_rope::line::RopeyLine::new(25)
        )
        .index()),
    );
}

#[test]
fn zz_clamps_at_top_of_buffer() {
    let mut ed = view_test_editor();
    seek_to_line(&mut ed, 2);
    ed.handle_key(key('z'));
    ed.handle_key(key('z'));
    // saturating_sub: 2 - 12 = 0.
    assert_eq!(ed.viewport().top_line, hume_rope::line::ContentLine::new(0));
}

#[test]
fn zz_allows_scrolling_past_eof() {
    let mut ed = view_test_editor();
    seek_to_line(&mut ed, 48);
    ed.handle_key(key('z'));
    ed.handle_key(key('z'));
    // 50 lines total, cursor on line 48, target=12 → top_line=36.
    // No bottom clamp: 36 + 24 = 60 > 50, trailing tildes are intentional.
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(36)
    );
}

#[test]
fn zk_puts_cursor_at_top() {
    let mut ed = view_test_editor();
    seek_to_line(&mut ed, 25);
    ed.handle_key(key('z'));
    ed.handle_key(key('k'));
    // target_row = 0 → top_line = cursor_line.
    assert_eq!(
        ed.viewport().top_line,
        hume_rope::line::ContentLine::new(25)
    );
    assert_eq!(ed.viewport().top_slot, 0);
}

#[test]
fn zj_puts_cursor_at_bottom() {
    let mut ed = view_test_editor();
    seek_to_line(&mut ed, 25);
    ed.handle_key(key('z'));
    ed.handle_key(key('j'));
    // height=24, target=23; cursor on line 25 → top_line = 25 - 23 = 2.
    assert_eq!(ed.viewport().top_line, hume_rope::line::ContentLine::new(2));
    assert_eq!(ed.viewport().top_slot, 0);
}

// ── Wrap mode ─────────────────────────────────────────────────────────────────

#[test]
fn zz_in_wrap_mode_walks_display_lines() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    // Three buffer lines, the middle one wraps to 4 rows under Soft{4}:
    //   line 0: "line0"          → 2 rows ("line", "0")
    //   line 1: "abcdefghijklmnop" → 4 rows ("abcd", "efgh", "ijkl", "mnop")
    //   line 2: "line2"          → 2 rows
    let content = "line0\nabcdefghijklmnop\nline2\n";
    let text = BufferText::from(content);

    // Cursor on "i" (line 1, char 8 within line; chars 8-11 are sub-row 2).
    let head = co(hume_rope::lines::line_start_char(
        text.rope(),
        hume_rope::line::RopeyLine::new(1),
    )
    .index()
        + 8);
    let sels = SelectionSet::single(Selection::collapsed(head));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 4 }),
        saved: None,
    });

    // Override viewport height to 4 so target_row = 2 (height / 2).
    ed.viewport_mut().height = 4;

    ed.handle_key(key('z'));
    ed.handle_key(key('z'));

    // From (line=1, sub=2), walking backward 2 rows lands at (line=1, sub=0).
    assert_eq!(ed.viewport().top_line, hume_rope::line::ContentLine::new(1));
    assert_eq!(ed.viewport().top_slot, 0);
}

#[test]
fn zk_in_wrap_mode_anchors_cursor_display_line_at_top() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    let content = "line0\nabcdefghijklmnop\nline2\n";
    let text = BufferText::from(content);

    // Cursor on "j" (line 1, char 9; chars 8-11 → sub-row 2).
    let head = co(hume_rope::lines::line_start_char(
        text.rope(),
        hume_rope::line::RopeyLine::new(1),
    )
    .index()
        + 9);
    let sels = SelectionSet::single(Selection::collapsed(head));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 4 }),
        saved: None,
    });
    ed.viewport_mut().height = 4;

    ed.handle_key(key('z'));
    ed.handle_key(key('k'));

    // target_row = 0 → top_line = cursor_line, top_slot = cursor_sub.
    assert_eq!(ed.viewport().top_line, hume_rope::line::ContentLine::new(1));
    assert_eq!(ed.viewport().top_slot, 2);
}

// ── Keymap wiring ─────────────────────────────────────────────────────────────

#[test]
fn z_alone_does_not_dispatch() {
    let mut ed = view_test_editor();
    seek_to_line(&mut ed, 25);
    let top_before = ed.viewport().top_line;
    ed.handle_key(key('z'));
    // After the first `z`, the trie is mid-walk — no command has fired yet.
    assert_eq!(ed.viewport().top_line, top_before);
}
