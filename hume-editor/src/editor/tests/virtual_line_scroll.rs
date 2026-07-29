// Editor-level integration tests for the top-line `Before`-block fix
// (docs/GIT-DIFF.md's former "virtual-line scroll accounting" open risk):
// the renderer and `cursor::screen_pos` must agree on where a `Before(0)`
// virtual block places buffer content, and wheel scrolling must move
// through such a block one display row at a time. No production
// `VirtualLineSource` emits `Before` yet (docs/GIT-DIFF.md Phase 4.5) — these
// register a synthetic one directly on the pane, mirroring `cursor/tests.rs`'s
// and `scroll/tests.rs`'s `OneBeforeLine` doubles.

use super::*;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_engine::pane::WrapMode;
use hume_engine::providers::{VirtualLine, VirtualLineAnchor, VirtualLineSource};
use ratatui::layout::Rect;
use termina::event::{Event, MouseEvent, MouseEventKind};

/// Emits one `Before(0)` virtual row, texted "V".
struct OneBeforeLine;

impl VirtualLineSource for OneBeforeLine {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<VirtualLine>,
    ) {
        if visible_lines.contains(&0) {
            out.push(VirtualLine {
                anchor: VirtualLineAnchor::Before(0),
                provider_id: 0,
                text: "V".to_string(),
                segments: Vec::new(),
            });
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
    ed.view.panes[pid].wrap_mode = WrapMode::Soft { width: 0 };
    ed.view.panes[pid]
        .providers
        .add_virtual_line_source(Box::new(OneBeforeLine));
    ed
}

fn cell(buf: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
    buf.cell(ratatui::layout::Position { x, y })
        .unwrap()
        .symbol()
        .to_string()
}

#[test]
fn screen_pos_agrees_with_the_actual_render_for_a_top_line_before_block() {
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
    // ask `screen_pos` with that same settled state, exactly as production
    // code does after `prepare_frame`.
    let pid = ed.state.focused_pane_id;
    let vp = ed.view.panes[pid].viewport.clone();
    let len_lines = ed.doc().text().len_lines();
    let content_width = ed.view.panes[pid].content_width(len_lines);
    let wrap_mode = ed.view.panes[pid].wrap_mode.resolve(content_width);
    let tab_width = ed.doc().overrides.tab_width(&ed.state.settings);
    let whitespace = ed.doc().overrides.whitespace(&ed.state.settings);
    let rope = ed.doc().text().rope();
    let cursor_char = ed.current_selections().primary().head();
    let mut ctx = hume_engine::pipeline::RenderContext::new();

    let pos = crate::editor::cursor::screen_pos(
        &vp,
        rope,
        cursor_char,
        &wrap_mode,
        tab_width,
        &whitespace,
        &mut ctx,
        &ed.view.panes[pid].providers,
        content_width,
    );
    assert_eq!(
        pos.map(|(_, row)| row),
        Some(1),
        "screen_pos must report the row the renderer actually draws 'x' on"
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

    let scroll_down = || {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: termina::event::Modifiers::NONE,
        })
    };

    assert_eq!(ed.viewport().top_line, 0);
    assert_eq!(
        ed.viewport().top_row_offset,
        0,
        "sanity: starts at block row 0"
    );

    ed.handle_event(scroll_down());
    assert_eq!(ed.viewport().top_line, 0);
    assert_eq!(
        ed.viewport().top_row_offset,
        1,
        "one notch skips exactly the virtual row, not the whole 2-row block"
    );

    ed.handle_event(scroll_down());
    assert_eq!(
        ed.viewport().top_line,
        1,
        "second notch exhausts line 0's block, landing on line 1"
    );
    assert_eq!(ed.viewport().top_row_offset, 0);
}

/// Emits `self.1` distinct `After(self.0)` rows.
struct MultiAfterLine(usize, usize);

impl VirtualLineSource for MultiAfterLine {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<VirtualLine>,
    ) {
        if visible_lines.contains(&self.0) {
            for i in 0..self.1 {
                out.push(VirtualLine {
                    anchor: VirtualLineAnchor::After(self.0),
                    provider_id: 0,
                    text: (i + 1).to_string(),
                    segments: Vec::new(),
                });
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
    use crate::ops::MotionMode;

    let content: String = (0..6).map(|i| format!("{i}\n")).collect();
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 0 }] {
        let mut ed = Editor::for_testing(Buffer::new(buf.clone(), sels.clone()));
        let pid = ed.state.focused_pane_id;
        ed.view.panes[pid].wrap_mode = wrap;
        ed.view.panes[pid]
            .providers
            .add_virtual_line_source(Box::new(MultiAfterLine(1, 3)));

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
