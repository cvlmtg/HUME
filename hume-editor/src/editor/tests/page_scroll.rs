use super::doubles::VirtualLineBlock;
use super::*;
use hume_engine::providers::VirtualLineAnchor;
use pretty_assertions::assert_eq;

// ── Page scroll ───────────────────────────────────────────────────────────────
//
// page_scroll / half_page_scroll are EditorCmd dispatches, both now
// `commands::scroll_view` — the same viewport-plus-cursor primitive the mouse
// wheel uses (`mouse.rs`), differing only in `count`. These tests verify both
// halves: the cursor moves by the right distance, and — new since the wheel
// and the page keys were unified — the viewport moves with it instead of only
// following once the cursor reaches the `scrolloff` margin.
//
// Viewport height in for_testing = 24 → page = 24, half = 12.
// Text: 30 single-char lines "a\n" (60 chars total). No wrap needed.
// Line N starts at char 2*N. 30 lines is close enough to the 24-row viewport
// that `max_scroll_top` caps a full page at line 9 (default scrolloff 3) —
// see `page_scroll_stops_at_max_scroll_top` below, which exercises that cap
// deliberately; the other tests here stay under it.

fn page_test_editor() -> Editor {
    let content = "a\n".repeat(30);
    // Pin the pane so it doesn't inherit whatever the default buffer/global
    // wrap-mode happens to be — these tests need no-wrap regardless.
    unwrapped_editor(&content, 0)
}

/// A much longer buffer than `page_test_editor`'s, so a full- or half-page
/// scroll from the top never hits `max_scroll_top` — isolating the "did the
/// viewport move by `count`" assertion from the "viewport capped at EOF" one.
fn long_page_test_editor() -> Editor {
    let content: String = numbered_lines(100);
    unwrapped_editor(&content, 0)
}

/// The pane's `(top_line, top_slot)` pair — what these EOF-stall tests
/// compare across renders to tell "the view moved" from "it didn't".
fn viewport_top(ed: &Editor) -> (hume_rope::line::ContentLine, usize) {
    (ed.viewport().top().line, ed.viewport().top().slot)
}

fn key_page_down() -> KeyEvent {
    KeyEvent::new(KeyCode::PageDown, Modifiers::NONE)
}

fn key_page_up() -> KeyEvent {
    KeyEvent::new(KeyCode::PageUp, Modifiers::NONE)
}

/// Ctrl+d (half-page-down) moves cursor down by half the viewport height (12 lines).
#[test]
fn half_page_down_moves_half_viewport() {
    let mut ed = page_test_editor();
    ed.handle_key(key_ctrl('d'));
    // half = 24/2 = 12 lines → line 12 → char 24
    assert_eq!(
        ed.current_selections().primary().head(),
        co(24),
        "half-page-down from line 0: cursor at line 12"
    );
}

/// Ctrl+u (half-page-up) moves cursor up by half the viewport height —
/// except the document's own start saturates both the view and a plain
/// 12-line-up walk at line 0, and landing exactly at the new top (row 0) is
/// what `carry`'s band clamp exists to correct: it pushes the landing back
/// down to row `margin` (3, default `scrolloff`) below the new top, line 3.
#[test]
fn half_page_up_moves_half_viewport() {
    let mut ed = page_test_editor();
    // Place cursor at line 12 first.
    ed.handle_key(key_ctrl('d'));
    assert_eq!(ed.current_selections().primary().head(), co(24));
    ed.handle_key(key_ctrl('u'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(6),
        "half-page-up saturates at the document start, then the band clamp \
         pushes it down to margin (3) below the new top (0), landing on line 3"
    );
}

/// PageDown moves cursor down by a full viewport height (24 lines).
#[test]
fn page_down_moves_full_viewport() {
    let mut ed = page_test_editor();
    ed.handle_key(key_page_down());
    // page = 24 lines → line 24 → char 48
    assert_eq!(
        ed.current_selections().primary().head(),
        co(48),
        "page-down from line 0: cursor at line 24"
    );
}

/// PageUp moves cursor up by a full viewport height — same saturation as
/// `half_page_up_moves_half_viewport`: the document start caps both the
/// view and the 24-line-up walk at line 0, and `carry`'s band clamp then
/// pushes that off-band landing back down to margin (3) below the new top.
#[test]
fn page_up_moves_full_viewport() {
    let mut ed = page_test_editor();
    // Place cursor at line 24 first.
    ed.handle_key(key_page_down());
    assert_eq!(ed.current_selections().primary().head(), co(48));
    ed.handle_key(key_page_up());
    assert_eq!(
        ed.current_selections().primary().head(),
        co(6),
        "page-up saturates at the document start, then the band clamp \
         pushes it down to margin (3) below the new top (0), landing on line 3"
    );
}

// ── Ctrl+D/Ctrl+U/PageDown/PageUp now scroll the view, not just the cursor ──
//
// Before unification, these commands only moved the cursor; the viewport
// followed later, once the cursor reached `scrolloff`. From the top of a
// file that never happened for a single page. Now `scroll_view` writes the
// viewport directly, matching vim/Helix.

/// Half-page-down from the top of a file moves the *view* by `height / 2`
/// display lines, not just the cursor. Previously the viewport stayed at 0
/// (the cursor at line 12 was still comfortably inside the first screen).
///
/// The cursor itself lands at line 15, not 12: landing exactly at the new
/// top (row 0) is `carry`'s band clamp's job to catch, not something a fresh
/// file's degenerate "cursor already at row 0" starting state should be
/// allowed to skip — see `carry`'s own doc. `scrolloff` defaults to 3, so
/// the clamp pushes the landing down to row `margin` (3) below the new top
/// (12), landing on line 15.
#[test]
fn half_page_down_moves_the_view() {
    let mut ed = long_page_test_editor();
    ed.handle_key(key_ctrl('d'));
    assert_eq!(
        ed.viewport().top().line,
        hume_rope::line::ContentLine::new(12),
        "half-page-down from the top must scroll the view by height/2 (12)"
    );
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        hume_rope::line::ContentLine::new(15),
        "cursor lands scrolloff (3) rows below the new top, not pinned to row 0"
    );
}

/// The band clamp's whole point: `frame.rs`'s `Viewport::reveal` correction
/// runs on the next settled frame regardless (the cursor move above raises
/// `PaneBufferState::reveal_pending` unconditionally), but it must find
/// nothing left to do — the viewport must already sit exactly where `carry`
/// landed it in `half_page_down_moves_the_view` above, not move a second
/// time once a real frame settles.
#[test]
fn scroll_view_leaves_nothing_for_reveal_to_correct() {
    let mut ed = long_page_test_editor();
    ed.handle_key(key_ctrl('d'));
    let top_after_scroll = ed.viewport().top();

    let rect = hume_grid::Rect::new(0, 0, 80, 25);
    ed.render_to_buf(rect); // settles, running Viewport::reveal if it has anything to do

    assert_eq!(
        ed.viewport().top(),
        top_after_scroll,
        "reveal must find nothing to correct — carry already left the \
         cursor inside the scrolloff band"
    );
}

// ── Wheel and Ctrl+D are the same action ────────────────────────────────────

/// A wheel notch and `Ctrl+D` are `scroll_view` with different `count`s —
/// with `mouse-scroll-lines` set to match `height / 2`, one notch must land
/// on exactly the same viewport and cursor as one `Ctrl+D`.
#[test]
fn wheel_and_half_page_down_agree() {
    let mut wheel_ed = long_page_test_editor();
    wheel_ed.state.settings.mouse_scroll_lines = 12; // height (24) / 2
    wheel_ed.handle_input(mouse_wheel(true));

    let mut key_ed = long_page_test_editor();
    key_ed.handle_key(key_ctrl('d'));

    assert_eq!(
        wheel_ed.viewport().top().line,
        key_ed.viewport().top().line,
        "wheel and Ctrl+D must scroll the view by the same amount"
    );
    assert_eq!(wheel_ed.viewport().top().slot, key_ed.viewport().top().slot);
    assert_eq!(
        wheel_ed.current_selections().primary().head(),
        key_ed.current_selections().primary().head(),
        "wheel and Ctrl+D must carry the cursor the same distance"
    );
}

// ── Bottom bound ─────────────────────────────────────────────────────────────

/// A page_test_editor's 30 lines nearly fill the 24-row viewport. Line 29
/// (the last) settles `scrolloff` (default 3) rows above the bottom row —
/// row 20 of 0..23 — not pinned to the bottom itself, so top stops at line 9
/// (29 - 20), matching where `Ctrl+D`/an ordinary cursor motion would
/// independently settle once the cursor reaches line 29.
#[test]
fn page_scroll_stops_at_max_scroll_top() {
    let mut ed = page_test_editor();
    for _ in 0..5 {
        ed.handle_key(key_page_down());
    }
    assert_eq!(
        ed.viewport().top().line,
        hume_rope::line::ContentLine::new(9),
        "the view must stop scrolloff rows short of the last line reaching the bottom row"
    );
    assert_eq!(ed.viewport().top().slot, 0);
}

/// A collapsed pane (0 rows) makes `scroll_page`'s `count = height` (the
/// non-half branch has no `.max(1)`) exactly 0, reaching `move_vertical`
/// with `count == 0` — the one case its own `remaining > 0 || (count > 0 &&
/// last_content == start)` loop condition must treat as a strict no-op
/// rather than walking anyway (the second disjunct is trivially true before
/// the loop ever runs).
#[test]
fn page_down_in_a_zero_height_pane_moves_neither_cursor_nor_view() {
    let mut ed = page_test_editor();
    let pid = ed.state.focus.id();
    ed.view.panes[pid].viewport.height = 0;
    let head_before = ed.current_selections().primary().head();

    ed.handle_key(key_page_down());

    assert_eq!(ed.current_selections().primary().head(), head_before);
    assert_eq!(
        ed.view.panes[pid].viewport.top().line,
        hume_rope::line::ContentLine::new(0)
    );
}

/// `page_test_editor`, scrolled to EOF with 10×`Ctrl+D` and settled — the
/// shared starting point for the EOF-stall regression tests below.
fn ctrl_d_to_eof() -> (Editor, hume_grid::Rect) {
    let mut ed = page_test_editor();
    let rect = hume_grid::Rect::new(0, 0, 80, 25); // 24 content rows
    ed.render_to_buf(rect); // settle before the first Ctrl+D

    for _ in 0..10 {
        ed.handle_key(key_ctrl('d'));
        ed.render_to_buf(rect);
    }
    (ed, rect)
}

/// Scrolling all the way to EOF with `Ctrl+D`, then moving the cursor with
/// an ordinary motion (`k`), must not jump the view: `Viewport::scroll_by`'s
/// `max_scroll_top` bound and `Viewport::reveal`'s `geo.target` settle point
/// both derive from the same `Viewport::geometry`, so the cursor motion's
/// reveal correction lands exactly where the EOF stall already parked the
/// view.
#[test]
fn ctrl_d_to_eof_then_an_ordinary_motion_does_not_jump_the_view() {
    let (mut ed, rect) = ctrl_d_to_eof();
    let before = viewport_top(&ed);
    assert_eq!(
        before.0,
        hume_rope::line::ContentLine::new(9),
        "sanity: settled at the same scrolloff-aware bound as page_scroll_stops_at_max_scroll_top"
    );

    ed.handle_key(key('k'));
    ed.render_to_buf(rect);

    assert_eq!(
        viewport_top(&ed),
        before,
        "moving the cursor right after reaching EOF must not move the view"
    );
}

/// The park a `Ctrl+D`/wheel stall leaves behind (every selection's head
/// unchanged, so `PaneBufferState::reveal_pending` was never raised) must
/// survive more than one idle frame — a signal that got set anyway on that
/// first idle render would let the very next one snap the view back onto
/// the stalled cursor. Render several frames with no input at all in
/// between and confirm nothing moves.
#[test]
fn a_stalled_scroll_survives_repeated_idle_frames() {
    let (mut ed, rect) = ctrl_d_to_eof();
    let before = viewport_top(&ed);

    // No input between these renders — a `reveal_pending` raised by the
    // first of them would let the second snap back.
    ed.render_to_buf(rect);
    ed.render_to_buf(rect);
    ed.render_to_buf(rect);

    assert_eq!(
        viewport_top(&ed),
        before,
        "repeated idle frames must not re-center the view onto the stalled cursor"
    );
}

/// `z k` (`top-view-on-cursor`) only writes the viewport — the cursor is
/// unmoved by construction, and writes no selection, so it raises no
/// `reveal_pending` of its own; `scroll_cursor_to_display_line` must apply
/// the scrolloff clamp itself, since no follow-up `Viewport::reveal` will.
#[test]
fn view_top_lands_the_cursor_at_scrolloff_through_the_real_frame() {
    let content: String = numbered_lines(50);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 25);
    let rect = hume_grid::Rect::new(0, 0, 80, 25); // 24 content rows

    ed.render_to_buf(rect); // settle before `z k` writes the viewport

    ed.execute_keymap_command("top-view-on-cursor".into(), None, false);
    ed.render_to_buf(rect);

    assert_eq!(
        ed.viewport().top().line,
        hume_rope::line::ContentLine::new(22),
        "top-view-on-cursor must settle scrolloff (3) rows above the cursor, \
         not pin it to screen row 0"
    );
}

// ── The cursor must make progress through a virtual-line block ─────────────
//
// `carry` (mouse wheel, Ctrl+D/Ctrl+U, PageDown/PageUp) walks display lines,
// virtual ones included, against its `delta` budget. A block taller than the
// budget would otherwise swallow it whole and leave the cursor exactly where
// it started — which, since the wheel and Ctrl+D also write the viewport
// directly, would let the view scroll on every notch while the cursor (and
// the caret it drives) stayed frozen on the block's near edge. `carry`'s own
// doc explains the overshoot this section tests for.

/// A 4-line `After(1)` virtual block, `Ctrl+D` with a budget of 3 (height 6,
/// half 3) — one shy of the block. The cursor must overshoot the block to
/// land on line 2, not stall on line 1.
#[test]
fn half_page_down_overshoots_a_virtual_line_block_taller_than_the_budget() {
    // 6 content lines ("0".."5"), cursor at the start of line 1 (char 2).
    let content: String = numbered_lines(6);
    let mut ed = unwrapped_editor(&content, 2);
    let pid = ed.state.focus.id();
    ed.view.panes[pid].viewport.height = 6; // half-page budget = 3
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(VirtualLineBlock::uniform(
            VirtualLineAnchor::After(hume_rope::line::ContentLine::new(1)),
            4,
            "V",
        )));

    ed.handle_key(key_ctrl('d'));

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        hume_rope::line::ContentLine::new(2),
        "half-page-down over a 4-line virtual block (budget 3) must overshoot to line 2, not stall on line 1"
    );
}
