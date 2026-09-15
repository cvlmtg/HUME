// Editor-level integration tests for the two ways a provider can add display
// lines the buffer text alone does not account for, and the requirement that
// the renderer and `cursor::content_pos` agree about both:
//
//   - a VIRTUAL_LINE-kind `DecorationSource`'s `Before`/`After` lines, which
//     occupy whole screen rows (the "virtual-line scroll accounting" risk),
//   - an INLINE-kind `DecorationSource`'s inserts, which take columns and so
//     can push a line onto an extra wrapped display line.
//
// `PaneVirtualLines` can now emit `Before` too (`set-virtual-lines!`'s
// `'anchor`); these register synthetic
// providers directly on the pane instead, mirroring `cursor/tests.rs`'s and
// `scroll/tests.rs`'s `OneBeforeLine` doubles, to isolate display-line-
// counting math from the Steel bridge (that path is exercised separately in
// `lsp_virtual_lines.rs`).

use super::doubles::{InlineHint, VirtualLineBlock};
use super::*;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_engine::pane::WrapMode;
use hume_engine::providers::VirtualLineAnchor;
use hume_grid::Rect;

/// Two-line buffer ("x\ny\n"), wrapping on, a `Before(0)` block registered
/// directly on the pane, cursor at line 0's start.
fn editor_with_before_line() -> Editor {
    let text = BufferText::from("x\ny\n");
    let sels = SelectionSet::single(Selection::collapsed(co(0)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    let pid = ed.state.focus.id();
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(WrapMode::Soft { width: 0 }),
        saved: None,
    });
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(VirtualLineBlock::uniform(
            VirtualLineAnchor::Before(hume_rope::line::ContentLine::new(0)),
            1,
            "V",
        )));
    ed
}

#[test]
fn content_pos_agrees_with_the_actual_render_for_a_top_line_before_block() {
    // Cursor on line 0, `Before(0)` block above it, viewport resting at its
    // default (top_line=0, top_slot=0, cursor already comfortably
    // visible — no auto-scroll needed). `pane_render.rs` and `cursor.rs`
    // must agree here: the renderer draws the block at screen row 0
    // regardless of which line it's anchored to, so 'x' (line 0's own
    // content) must land at row 1, not row 0.
    let mut ed = editor_with_before_line();
    ed.state.settings.scrolloff = 0; // isolate this from margin-triggered auto-scroll
    // Height 4: the engine reserves row 3 for the statusline, leaving 3 rows
    // of actual pane content (V, x, y).
    let rect = Rect::new(0, 0, 10, 4);
    let buf = ed.render_to_buf(rect);

    assert_eq!(cell(&buf, 0, 0), "V", "sanity: virtual line drawn at row 0");
    assert_eq!(cell(&buf, 0, 1), "x", "line 0's content pushed to row 1");
    assert_eq!(cell(&buf, 0, 2), "y", "line 1 follows at row 2");

    // `render_to_buf` already ran `prepare_frame` (settling the viewport);
    // ask `content_pos` with that same settled state, exactly as production
    // code does after `prepare_frame`.
    let pid = ed.state.focus.id();
    let vp = ed.view.panes[pid].viewport.clone();
    let cursor_char = ed.current_selections().primary().head();
    let bid = ed.view.panes[pid].buffer_id;
    let Editor { state, view, .. } = &mut ed;
    let key = state.format_key(&view.panes[pid]);
    let (mut dlm, _) = crate::editor::commands::pane_display_lines(
        state.buffers.get(bid),
        &mut view.panes[pid],
        key,
    );

    let pos = crate::editor::cursor::content_pos(&vp, &mut dlm, cursor_char);
    assert_eq!(
        pos.map(|(_, row)| row),
        Some(1),
        "content_pos must report the row the renderer actually draws 'x' on"
    );
}

#[test]
fn mouse_wheel_moves_one_display_line_at_a_time_through_a_before_block() {
    // Wheel scrolling (`Viewport::scroll_by`) must walk through the
    // 2-display-line block ([V, x]) one display line per notch — never skip
    // the whole block in a single notch. Viewport height 2 is shorter than
    // the 3-display-line total content (V, x, y), so there's genuinely
    // something to scroll — up to the point where `max_scroll_top` puts the
    // document's last display line (y) on the bottom row, which the first
    // notch alone already reaches.
    let mut ed = editor_with_before_line();
    ed.state.settings.mouse_scroll_lines = 1;
    ed.view.panes[ed.state.focus.id()].viewport.height = 2;

    let scroll_down = || mouse_wheel(true);

    assert_eq!(
        ed.viewport().top().line,
        hume_rope::line::ContentLine::new(0)
    );
    assert_eq!(
        ed.viewport().top().slot,
        0,
        "sanity: starts at the block's first (virtual) line"
    );

    ed.handle_input(scroll_down());
    assert_eq!(
        ed.viewport().top().line,
        hume_rope::line::ContentLine::new(0)
    );
    assert_eq!(
        ed.viewport().top().slot,
        1,
        "one notch skips exactly the virtual line, not the whole 2-display-line block — \
         and already reaches max_scroll_top, since the next display line (y) is the document's last"
    );

    ed.handle_input(scroll_down());
    assert_eq!(
        ed.viewport().top().line,
        hume_rope::line::ContentLine::new(0),
        "further scrolling stays clamped — y is already on the bottom row"
    );
    assert_eq!(ed.viewport().top().slot, 1);
}

// ── The wheel and Ctrl+D must pass a mid-buffer ghost block, not stall ─────
//
// The reported bug: opening a file with `:toggle-inline-diff` on and
// scrolling with the mouse wheel through a deletion hunk. Reproduced here
// with a synthetic `After` block instead of the real git-diff plugin —
// same shape, deterministic size.
//
// Interleaving `render_to_buf` between notches is the point: it runs the
// real per-frame scroll pass (`prepare_frame` → `scroll_into_view`), which
// the wheel's own `handle_input` does not run on its own. That pass's
// vertical cursor-follow correction, `Viewport::reveal`, is gated on
// `PaneBufferState::reveal_pending`, which stays unset when `carry` can't
// fully follow a scroll into a virtual block — so the pass must not undo
// the wheel notch just because the cursor couldn't follow it in.

/// 20 content lines, a 4-line `After(8)` block, `mouse-scroll-lines` = 3 (one
/// short of the block). Cursor starts mid-buffer (line 5), away from the
/// document's own top edge — `Viewport::reveal`'s `scrolloff` margin
/// otherwise tempers the *first* scroll away from a document boundary
/// regardless of virtual lines, a separate and expected interaction this
/// test isn't about. Three notches, each followed by a render, must each
/// advance the view — the reported bug repeated the same position forever
/// once the budget landed inside the block — and the cursor must come out
/// the other side of it.
#[test]
fn wheel_passes_a_mid_buffer_ghost_block() {
    let content: String = numbered_lines(20);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 5);
    ed.state.settings.mouse_scroll_lines = 3;
    ed.view.panes[ed.state.focus.id()]
        .providers
        .add_decoration_source(Box::new(VirtualLineBlock::uniform(
            VirtualLineAnchor::After(hume_rope::line::ContentLine::new(8)),
            4,
            "V",
        )));

    let rect = Rect::new(0, 0, 20, 11); // 10 content rows once the statusline takes one
    ed.render_to_buf(rect); // settle: top stays 0, cursor at line 5 is already in view

    // The reported bug's signature: the viewport stops changing at all, well
    // before the document's own end. Capture the resolved position after
    // each notch and assert it strictly advances every time — a stall would
    // repeat a value; a full-block skip is fine and not what this checks.
    let mut positions = Vec::new();
    for _ in 0..3 {
        ed.handle_input(mouse_wheel(true));
        ed.render_to_buf(rect);
        let vp = ed.viewport();
        positions.push(vp.top());
    }
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "each notch must advance the view — a stalled notch (repeating the \
         previous position) is the reported bug — got {positions:?}"
    );
    assert!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head())
            > hume_rope::line::ContentLine::new(8),
        "the cursor must have advanced past the block too, not stayed stuck at its near edge"
    );
}

// ── A trailing ghost block is fully reachable ─────────────────────────────
//
// Reachable by *no* keyboard command: `Ctrl+D`/`PageDown` cap the view at
// `scrolloff` rows below the last content line (`Viewport::scroll_by`'s own
// `max_scroll_top` bound). Only a view-led scroll can go further — once the
// cursor reaches the document's last content line it can advance no further
// (`carry`'s overshoot can't escape a document edge), so the selection it
// returns is unchanged and `PaneBufferState::reveal_pending` is never raised
// — the cursor-follow gate in `scroll_into_view` stops re-running
// `Viewport::reveal`, and the wheel's direct viewport write is free to keep
// advancing up to `max_scroll_top`.
//
// `max_scroll_top` itself leaves `scrolloff` rows of look-ahead past the
// block's last virtual line, exactly like a real buffer line — matching
// where `Ctrl+D`/an ordinary cursor motion would independently settle once
// the cursor reaches the document's end, so a scroll all the way down and
// the very next unrelated cursor move land on the same top.

/// 10 content lines, a 5-line `After(9)` (last line) block. Repeated wheel
/// notches must reach the point where the block's last virtual line renders
/// `scrolloff` (default 3) rows above the bottom of the pane — not pinned to
/// the bottom row itself — and then stay there.
#[test]
fn wheel_reaches_a_trailing_after_last_line_block() {
    let content: String = numbered_lines(10);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 3);
    ed.state.settings.mouse_scroll_lines = 3;
    ed.view.panes[ed.state.focus.id()]
        .providers
        .add_decoration_source(Box::new(VirtualLineBlock::uniform(
            VirtualLineAnchor::After(hume_rope::line::ContentLine::new(9)),
            5,
            "V",
        )));

    let rect = Rect::new(0, 0, 20, 9); // 8 content rows once the statusline takes one
    ed.render_to_buf(rect);
    for _ in 0..10 {
        ed.handle_input(mouse_wheel(true));
        ed.render_to_buf(rect);
    }

    let settled = ed.render_to_buf(rect);
    // margin = min(3, (8-1)/2) = 3; the block's last virtual line lands at
    // row 8-3-1 = 4, leaving rows 5..7 (3 rows) as scrolloff padding below it.
    assert_eq!(
        cell(&settled, 0, 4),
        "V",
        "the block's last virtual line must land scrolloff rows above the bottom"
    );
    assert_eq!(
        cell(&settled, 0, 7), // bottom content row
        "~",
        "the scrolloff margin past the block's end must render as filler, not more ghost lines"
    );

    // Further scrolling must stay clamped there, not stall short of it or
    // overshoot past it.
    let (top_before, slot_before) = (ed.viewport().top().line, ed.viewport().top().slot);
    ed.handle_input(mouse_wheel(true));
    ed.render_to_buf(rect);
    assert_eq!(
        (ed.viewport().top().line, ed.viewport().top().slot),
        (top_before, slot_before),
        "once the margin below the block's end is reached, further scrolling is a no-op"
    );
}

// ── Bottom bound ─────────────────────────────────────────────────────────

/// A plain (no virtual lines) 30-line buffer. Many wheel notches must settle
/// with the document's own last line `scrolloff` rows above the bottom row —
/// matching where an ordinary cursor motion would independently place it —
/// never scrolled further, and never pinned to the very bottom row.
#[test]
fn wheel_never_scrolls_past_the_documents_last_display_line() {
    let content: String = (0..30).map(|i| format!("line{i}\n")).collect();
    let mut ed = unwrapped_editor(&content, 0);
    ed.state.settings.mouse_scroll_lines = 3;
    let rect = Rect::new(0, 0, 20, 25); // 24 content rows
    ed.render_to_buf(rect);

    for _ in 0..100 {
        ed.handle_input(mouse_wheel(true));
        ed.render_to_buf(rect);
    }

    let (top_before, slot_before) = (ed.viewport().top().line, ed.viewport().top().slot);
    ed.handle_input(mouse_wheel(true));
    let settled = ed.render_to_buf(rect);
    assert_eq!(
        (ed.viewport().top().line, ed.viewport().top().slot),
        (top_before, slot_before),
        "the view must have settled after 100 notches — no further scroll past the end"
    );
    // margin = min(3, (24-1)/2) = 3; line 29 lands at row 24-3-1 = 20.
    assert_eq!(
        cell(&settled, 0, 20),
        "l", // "line29"'s first character
        "the document's last line must sit scrolloff rows above the bottom"
    );
    assert_eq!(
        cell(&settled, 0, 23), // bottom content row
        "~",
        "the scrolloff margin past the last line must render as filler"
    );
}

/// Screen-relative cursor-follow (`commands::scroll_view` — mouse wheel,
/// page/half-page scroll) must count virtual lines toward its display-line
/// budget: moving "5 display lines" down through a 3-display-line `After(1)`
/// block only advances the cursor 2 REAL lines (0 → 1 → 2), not 5 — matching
/// where the viewport itself would land, in either wrap mode. Plain `j`/`k`
/// (`apply_visual_vertical`'s `ContentDisplayLine`, exercised elsewhere) are
/// unaffected: virtual lines stay free for those.
#[test]
fn view_scroll_cursor_follow_counts_virtual_lines_toward_its_budget() {
    use crate::editor::commands::scroll_view;
    use hume_ops::MotionMode;

    let content: String = numbered_lines(6);
    let text = BufferText::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(co(0)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 0 }] {
        let mut ed = Editor::for_testing(Buffer::new(text.clone(), sels.clone()));
        let pid = ed.state.focus.id();
        ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
            mode: Some(wrap),
            saved: None,
        });
        ed.view.panes[pid]
            .providers
            .add_decoration_source(Box::new(VirtualLineBlock::numbered(
                VirtualLineAnchor::After(hume_rope::line::ContentLine::new(1)),
                3,
            )));

        scroll_view(&mut ed.state, &mut ed.view, pid, 5, true, MotionMode::Move);
        let cursor_line = ed
            .doc()
            .text()
            .char_to_line(ed.current_selections().primary().head());
        assert_eq!(
            cursor_line,
            hume_rope::line::ContentLine::new(2),
            "5 display lines crosses 3 virtual After(1) lines, landing on real line 2, not 5 ({wrap:?})"
        );
    }
}

// ── Inline decorations count toward the display-line budget ──────────────
//
// The other axis of the same "counted display lines must match rendered
// rows" requirement. An inline insert takes columns, so it participates in
// wrapping: a line that fits on one display line without it can need two
// with it. Display-line counting that formats without inserts reports one
// display line where the renderer draws two, and everything below the hint
// lands one row off.

#[test]
fn content_pos_counts_an_inline_hints_extra_wrap_display_line() {
    // Line 0 is "abcdef" — 6 columns, which fits the 10-column content width
    // on its own. The 6-column hint makes 12, wrapping it onto a second
    // display line:
    //
    //   row 0  HHHHHHabcd
    //   row 1  ef
    //   row 2  y            ← line 1, pushed down by the hint's extra display line
    let text = BufferText::from("abcdef\ny\n");
    // Cursor on line 1 (char 7), below the wrap the hint causes.
    let sels = SelectionSet::single(Selection::collapsed(co(7)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.state.settings.scrolloff = 0;
    let pid = ed.state.focus.id();
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(WrapMode::Soft { width: 0 }),
        saved: None,
    });
    let scope = ed.view.registry.intern("ui.virtual_text");
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(InlineHint::new(0, 0, "HHHHHH").with_scope(scope)));

    // Height 4 leaves 3 content rows once the statusline takes one.
    let rendered = ed.render_to_buf(Rect::new(0, 0, 10, 4));
    assert_eq!(cell(&rendered, 0, 0), "H", "sanity: hint drawn at row 0");
    assert_eq!(
        cell(&rendered, 0, 1),
        "e",
        "the hint pushed 'ef' onto a second display line"
    );
    assert_eq!(cell(&rendered, 0, 2), "y", "line 1 follows at row 2");

    let vp = ed.view.panes[pid].viewport.clone();
    let cursor_char = ed.current_selections().primary().head();
    let bid = ed.view.panes[pid].buffer_id;
    let Editor { state, view, .. } = &mut ed;
    let key = state.format_key(&view.panes[pid]);
    let (mut dlm, _) = crate::editor::commands::pane_display_lines(
        state.buffers.get(bid),
        &mut view.panes[pid],
        key,
    );
    assert_eq!(
        crate::editor::cursor::content_pos(&vp, &mut dlm, cursor_char).map(|(_, row)| row),
        Some(2),
        "content_pos must count the hint's extra display line, as the renderer does"
    );
}
