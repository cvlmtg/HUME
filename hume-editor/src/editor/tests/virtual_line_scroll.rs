// Editor-level integration tests for the two ways a provider can add display
// rows the buffer text alone does not account for, and the requirement that
// the renderer and `cursor::content_pos` agree about both:
//
//   - a VIRTUAL_LINE-kind `DecorationSource`'s `Before`/`After` rows, which
//     occupy whole screen rows (the "virtual-line scroll accounting" risk),
//   - an INLINE-kind `DecorationSource`'s inserts, which take columns and so
//     can push a line onto an extra wrap row.
//
// `PaneVirtualLines` can now emit `Before` too (`set-virtual-lines!`'s
// `'anchor`); these register synthetic
// providers directly on the pane instead, mirroring `cursor/tests.rs`'s and
// `scroll/tests.rs`'s `OneBeforeLine` doubles, to isolate row-counting math
// from the Steel bridge (that path is exercised separately in
// `lsp_virtual_lines.rs`).

use super::*;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_engine::pane::WrapMode;
use hume_engine::providers::{
    Decoration, DecorationKinds, DecorationSource, InlineInsert, VirtualLine, VirtualLineAnchor,
};
use hume_engine::types::ScopeId;
use ratatui::layout::Rect;

/// Emits one `Before(0)` virtual row, texted "V".
struct OneBeforeLine;

impl DecorationSource for OneBeforeLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if line_idx == 0 {
            out.push(Decoration::VirtualLine(VirtualLine {
                anchor: VirtualLineAnchor::Before(0),
                provider_id: 0,
                text: "V".to_string(),
                segments: Vec::new(),
                base_scope: None,
            }));
        }
    }
}

/// Two-line buffer ("x\ny\n"), wrapping on, a `Before(0)` block registered
/// directly on the pane, cursor at line 0's start.
fn editor_with_before_line() -> Editor {
    let buf = Text::from("x\ny\n");
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    let pid = ed.state.focused_pane_id;
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(WrapMode::Soft { width: 0 }),
        saved: None,
    });
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(OneBeforeLine));
    ed
}

fn cell(buf: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
    buf.cell(ratatui::layout::Position { x, y })
        .unwrap()
        .symbol()
        .to_string()
}

#[test]
fn content_pos_agrees_with_the_actual_render_for_a_top_line_before_block() {
    // Cursor on line 0, `Before(0)` block above it, viewport resting at its
    // default (top_line=0, top_row_offset=0, cursor already comfortably
    // visible — no auto-scroll needed). This is exactly the scenario
    // `pane_render.rs` and `cursor.rs` used to disagree about: the renderer
    // draws the block at screen row 0 regardless of which line it's
    // anchored to, so 'x' (line 0's own content) must land at row 1, not
    // row 0.
    let mut ed = editor_with_before_line();
    ed.state.settings.scrolloff = 0; // isolate this from margin-triggered auto-scroll
    // Height 4: the engine reserves row 3 for the statusline, leaving 3 rows
    // of actual pane content (V, x, y).
    let rect = Rect::new(0, 0, 10, 4);
    let buf = ed.render_to_buf(rect);

    assert_eq!(cell(&buf, 0, 0), "V", "sanity: virtual row drawn at row 0");
    assert_eq!(cell(&buf, 0, 1), "x", "line 0's content pushed to row 1");
    assert_eq!(cell(&buf, 0, 2), "y", "line 1 follows at row 2");

    // `render_to_buf` already ran `prepare_frame` (settling the viewport);
    // ask `content_pos` with that same settled state, exactly as production
    // code does after `prepare_frame`.
    let pid = ed.state.focused_pane_id;
    let vp = ed.view.panes[pid].viewport.clone();
    let cursor_char = ed.current_selections().primary().head();
    let mut scratch = hume_engine::format::FormatScratch::new();
    let mut rm = crate::editor::commands::pane_row_map(
        ed.doc(),
        &ed.state.settings,
        &ed.view.panes[pid],
        &mut scratch,
    );

    let pos = crate::editor::cursor::content_pos(&vp, &mut rm, cursor_char);
    assert_eq!(
        pos.map(|(_, row)| row),
        Some(1),
        "content_pos must report the row the renderer actually draws 'x' on"
    );
}

#[test]
fn mouse_wheel_moves_one_row_at_a_time_through_a_before_block() {
    // Wheel scrolling (`scroll_viewport_down`) must walk through the
    // 2-row block ([V, x]) one display row per notch — not skip the whole
    // block in a single notch (the atomic-scroll behavior this fix removed).
    // Viewport height 2 is shorter than the 3-row total content (V, x, y),
    // so there's genuinely something to scroll.
    let mut ed = editor_with_before_line();
    ed.state.settings.mouse_scroll_lines = 1;
    ed.view.panes[ed.state.focused_pane_id].viewport.height = 2;

    let scroll_down = || mouse_wheel(true);

    assert_eq!(ed.viewport().top_line, 0);
    assert_eq!(
        ed.viewport().top_row_offset,
        0,
        "sanity: starts at block row 0"
    );

    ed.handle_input(scroll_down());
    assert_eq!(ed.viewport().top_line, 0);
    assert_eq!(
        ed.viewport().top_row_offset,
        1,
        "one notch skips exactly the virtual row, not the whole 2-row block"
    );

    ed.handle_input(scroll_down());
    assert_eq!(
        ed.viewport().top_line,
        1,
        "second notch exhausts line 0's block, landing on line 1"
    );
    assert_eq!(ed.viewport().top_row_offset, 0);
}

/// Emits `self.1` distinct `After(self.0)` rows.
struct MultiAfterLine(usize, usize);

impl DecorationSource for MultiAfterLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if line_idx == self.0 {
            for i in 0..self.1 {
                out.push(Decoration::VirtualLine(VirtualLine {
                    anchor: VirtualLineAnchor::After(self.0),
                    provider_id: 0,
                    text: (i + 1).to_string(),
                    segments: Vec::new(),
                    base_scope: None,
                }));
            }
        }
    }
}

/// Screen-relative cursor-follow (`VerticalUnit::ScreenRow` — mouse wheel,
/// page/half-page scroll) must count virtual rows toward its display-row
/// budget: moving "5 display rows" down through a 3-row `After(1)` block
/// only advances the cursor 2 REAL lines (0 → 1 → 2), not 5 — matching
/// where the viewport itself would land, in either wrap mode. Plain `j`/`k`
/// (`VerticalUnit::ContentRow`, exercised elsewhere) are unaffected: virtual
/// rows stay free for those.
#[test]
fn screen_row_cursor_follow_counts_virtual_rows_toward_its_budget() {
    use crate::editor::visual_move::{VerticalUnit, apply_visual_vertical};
    use hume_ops::MotionMode;

    let content: String = (0..6).map(|i| format!("{i}\n")).collect();
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 0 }] {
        let mut ed = Editor::for_testing(Buffer::new(buf.clone(), sels.clone()));
        let pid = ed.state.focused_pane_id;
        ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
            mode: Some(wrap),
            saved: None,
        });
        ed.view.panes[pid]
            .providers
            .add_decoration_source(Box::new(MultiAfterLine(1, 3)));

        apply_visual_vertical(
            &mut ed.state,
            &mut ed.view,
            5,
            true,
            MotionMode::Move,
            VerticalUnit::ScreenRow,
        );
        let cursor_line = ed
            .doc()
            .text()
            .char_to_line(ed.current_selections().primary().head());
        assert_eq!(
            cursor_line, 2,
            "5 display rows crosses 3 virtual After(1) rows, landing on real line 2, not 5 ({wrap:?})"
        );
    }
}

// ── Inline decorations count toward the wrap row budget ──────────────────
//
// The other axis of the same "counted rows must equal rendered rows"
// requirement. An inline insert takes columns, so it participates in
// wrapping: a line that fits on one row without it can need two with it.
// Row counting that formats without inserts (as it did before `RowMap`)
// reports one row where the renderer draws two, and everything below the
// hint lands one row off.

/// Emits one 6-column inline insert at the head of line 0. Holds an
/// already-interned scope, the same contract real providers follow.
struct HintOnLine0(ScopeId);

impl DecorationSource for HintOnLine0 {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if line_idx == 0 {
            out.push(Decoration::Inline(InlineInsert {
                byte_offset: 0,
                text: "HHHHHH".to_string(),
                scope: self.0,
            }));
        }
    }
}

#[test]
fn content_pos_counts_an_inline_hints_extra_wrap_row() {
    // Line 0 is "abcdef" — 6 columns, which fits the 10-column content width
    // on its own. The 6-column hint makes 12, wrapping it onto a second row:
    //
    //   row 0  HHHHHHabcd
    //   row 1  ef
    //   row 2  y            ← line 1, pushed down by the hint's wrap row
    let buf = Text::from("abcdef\ny\n");
    // Cursor on line 1 (char 7), below the wrap the hint causes.
    let sels = SelectionSet::single(Selection::collapsed(7));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    ed.state.settings.scrolloff = 0;
    let pid = ed.state.focused_pane_id;
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(WrapMode::Soft { width: 0 }),
        saved: None,
    });
    let scope = ed.view.registry.intern("ui.virtual_text");
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(HintOnLine0(scope)));

    // Height 4 leaves 3 content rows once the statusline takes one.
    let rendered = ed.render_to_buf(Rect::new(0, 0, 10, 4));
    assert_eq!(cell(&rendered, 0, 0), "H", "sanity: hint drawn at row 0");
    assert_eq!(
        cell(&rendered, 0, 1),
        "e",
        "the hint pushed 'ef' onto a second wrap row"
    );
    assert_eq!(cell(&rendered, 0, 2), "y", "line 1 follows at row 2");

    let vp = ed.view.panes[pid].viewport.clone();
    let cursor_char = ed.current_selections().primary().head();
    let mut scratch = hume_engine::format::FormatScratch::new();
    let mut rm = crate::editor::commands::pane_row_map(
        ed.doc(),
        &ed.state.settings,
        &ed.view.panes[pid],
        &mut scratch,
    );
    assert_eq!(
        crate::editor::cursor::content_pos(&vp, &mut rm, cursor_char).map(|(_, row)| row),
        Some(2),
        "content_pos must count the hint's wrap row, as the renderer does"
    );
}
