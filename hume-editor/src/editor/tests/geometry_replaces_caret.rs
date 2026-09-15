//! A resize or a wrap-mode change must still replace the caret even though
//! the cursor itself is unmoved between the two frames each test drives.
//! Neither fixture triggers a `ViewScroll`, so `PaneBufferState::scroll_pin`
//! stays `None` throughout and `frame.rs`'s vertical `ensure_cursor_visible`
//! correction runs unconditionally — these tests pin that a geometry change
//! is never mistaken, by anything, for the one case meant to skip that
//! correction (a `ViewScroll`'s deliberate park; see `scroll_into_view`'s own
//! doc).

use super::*;

/// A resize with the cursor unmoved must still replace the caret: no
/// `ViewScroll` ran between the two frames below, so `scroll_pin` is `None`
/// and `ensure_cursor_visible` runs unconditionally — it must scroll the
/// (unchanged) viewport top far enough for the shrunk height to still cover
/// the cursor, rather than trusting a stale top left over from the taller
/// frame.
#[test]
fn a_height_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = numbered_lines(60);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 40);

    assert!(
        frame(&mut ed, 80, 50).cursor_content_pos.is_some(),
        "sanity: the cursor is visible in the tall viewport"
    );

    assert!(
        frame(&mut ed, 80, 10).cursor_content_pos.is_some(),
        "the caret must still be placed after a resize, even though the \
         cursor itself did not move"
    );
}

/// A wrap-mode change with the cursor unmoved must still replace the caret:
/// again, no `ViewScroll` ran between the two frames, so `scroll_pin` is
/// `None` and `ensure_cursor_visible` runs unconditionally — it must
/// re-place the caret against wrapping's now-different display-line layout
/// rather than trusting the unwrapped frame's screen position.
#[test]
fn a_wrap_mode_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = (0..60)
        .map(|i| format!("{i} {}\n", "x".repeat(120)))
        .collect();
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 20);

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "sanity: the cursor is visible unwrapped"
    );

    let pid = ed.state.focus.id();
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 0 }),
        saved: None,
    });

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "the caret must still be placed after a wrap-mode change, even \
         though the cursor's DisplayLinePos is unchanged"
    );
}
