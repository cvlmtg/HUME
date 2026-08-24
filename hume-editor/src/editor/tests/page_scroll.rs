use super::*;
use pretty_assertions::assert_eq;

// ── Page scroll ───────────────────────────────────────────────────────────────
//
// page_scroll / half_page_scroll are EditorCmd dispatches.
// These tests verify they move by the right distance.
//
// Viewport height in for_testing = 24 → page = 24, half = 12.
// Text: 30 single-char lines "a\n" (60 chars total). No wrap needed.
// Line N starts at char 2*N.

fn page_test_editor() -> Editor {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    let content = "a\n".repeat(30);
    let buf = BufferText::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    // Pin the pane so it doesn't inherit whatever the default buffer/global
    // wrap-mode happens to be — these tests need no-wrap regardless.
    ed.view.panes[ed.state.focused_pane_id].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::None),
        saved: None,
    });
    ed
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
        24,
        "half-page-down from line 0: cursor at line 12"
    );
}

/// Ctrl+u (half-page-up) moves cursor up by half the viewport height.
#[test]
fn half_page_up_moves_half_viewport() {
    let mut ed = page_test_editor();
    // Place cursor at line 12 first.
    ed.handle_key(key_ctrl('d'));
    assert_eq!(ed.current_selections().primary().head(), 24);
    ed.handle_key(key_ctrl('u'));
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "half-page-up returns to line 0"
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
        48,
        "page-down from line 0: cursor at line 24"
    );
}

/// PageUp moves cursor up by a full viewport height.
#[test]
fn page_up_moves_full_viewport() {
    let mut ed = page_test_editor();
    // Place cursor at line 24 first.
    ed.handle_key(key_page_down());
    assert_eq!(ed.current_selections().primary().head(), 48);
    ed.handle_key(key_page_up());
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "page-up returns to line 0"
    );
}
