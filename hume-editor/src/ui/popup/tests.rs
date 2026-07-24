use super::*;

// ── wrap_text ──────────────────────────────────────────────────────────

#[test]
fn wrap_text_short_line_is_unchanged() {
    assert_eq!(wrap_text("hello", 60), vec!["hello"]);
}

#[test]
fn wrap_text_breaks_on_word_boundary() {
    assert_eq!(wrap_text("hello world foo", 11), vec!["hello world", "foo"]);
}

#[test]
fn wrap_text_preserves_explicit_newlines() {
    assert_eq!(
        wrap_text("line one\nline two", 60),
        vec!["line one", "line two"]
    );
}

#[test]
fn wrap_text_hard_breaks_an_overlong_word() {
    assert_eq!(wrap_text("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
}

#[test]
fn wrap_text_empty_line_preserved() {
    assert_eq!(wrap_text("a\n\nb", 60), vec!["a", "", "b"]);
}

// ── wrap_styled ────────────────────────────────────────────────────────

fn red() -> Style {
    Style::default().fg(ratatui::style::Color::Red)
}

#[test]
fn wrap_styled_preserves_a_style_boundary_within_one_row() {
    let runs = [
        ("hello ".to_string(), Style::default()),
        ("world".to_string(), red()),
    ];
    let rows = wrap_styled(&runs, 60);
    assert_eq!(rows.len(), 1, "both words fit on one row");
    assert_eq!(
        rows[0],
        vec![
            ("hello".to_string(), Style::default()),
            (" world".to_string(), red()),
        ],
        "the style change must land at the word boundary, not bleed into \
         the plain run or get lost (the synthetic separator space carries \
         the *following* word's style, so it coalesces with \"world\")"
    );
}

#[test]
fn wrap_styled_splits_a_style_change_across_two_wrapped_rows() {
    // "aaaa bbbb" at width 4: "aaaa" and "bbbb" each land on their own row
    // (the too-narrow-for-both-words case `wrap_text_breaks_on_word_boundary`
    // already covers for plain text) — here "bbbb" carries a different style,
    // which must survive onto its own row's run list untouched.
    let runs = [
        ("aaaa ".to_string(), Style::default()),
        ("bbbb".to_string(), red()),
    ];
    let rows = wrap_styled(&runs, 4);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], vec![("aaaa".to_string(), Style::default())]);
    assert_eq!(rows[1], vec![("bbbb".to_string(), red())]);
}

// ── resolve_popup_geometry ────────────────────────────────────────────

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
    Rect::new(x, y, w, h)
}

#[test]
fn geometry_places_below_cursor_by_default() {
    let pane = rect(0, 0, 40, 20);
    let (x, y, w, h) = resolve_popup_geometry(2, 1, (5, 5), pane);
    assert_eq!((x, y, w, h), (5, 6, 2, 1), "one line below the cursor row");
}

/// Content fits below the anchor even though there happens to be *more*
/// room above than below — must still place it below. Distinguishes the
/// "fits below" condition from a "prefer whichever side has more room"
/// condition, which would flip it above unnecessarily here.
#[test]
fn geometry_stays_below_when_content_fits_even_with_more_room_above() {
    let pane = rect(0, 0, 40, 100);
    let (_, y, _, _) = resolve_popup_geometry(5, 10, (5, 50), pane);
    assert_eq!(
        y, 51,
        "content (height 10) fits in the 49 rows below — must not flip \
             just because 50 rows happen to be available above"
    );
}

#[test]
fn geometry_flips_above_near_bottom_edge() {
    let pane = rect(0, 0, 40, 20);
    // Cursor near the bottom: only 2 rows below, 17 above — flip.
    let (_, y, _, _) = resolve_popup_geometry(5, 5, (5, 18), pane);
    assert_eq!(y, 13, "flips to render entirely above the cursor row");
}

#[test]
fn geometry_clamps_horizontally_at_right_edge() {
    let width = unicode_display_width("a very long popup line here") as u16;
    // Pane wide enough to hold the content, but the anchor sits close
    // enough to the right edge that placing the popup there unclamped
    // would overflow.
    let pane = rect(0, 0, 32, 20);
    let (x, _, w, _) = resolve_popup_geometry(width, 1, (15, 5), pane);
    assert!(
        x + w <= pane.x + pane.width,
        "popup must not cross the pane's right edge, got x={x}, w={w}"
    );
}

#[test]
fn geometry_never_escapes_pane_bounds_even_at_corner() {
    let pane = rect(2, 2, 10, 10);
    let (x, y, w, _) = resolve_popup_geometry(5, 1, (2, 2), pane);
    assert!(x >= pane.x && x + w <= pane.x + pane.width);
    assert!(y >= pane.y && y < pane.y + pane.height);
}

/// A box wider than the pane must be clamped to the pane's width, not
/// merely repositioned — an unclamped over-wide box fails
/// `PopupOverlay`'s bounds check and the whole popup is silently
/// dropped instead of painting a clamped one.
#[test]
fn geometry_clamps_width_to_pane_when_content_is_wider() {
    let pane = rect(0, 0, 20, 20);
    let (x, _, w, _) = resolve_popup_geometry(50, 1, (5, 5), pane);
    assert_eq!(w, 20, "width must clamp to the full pane width");
    assert_eq!(
        x, 0,
        "clamped box has nowhere to go but the pane's left edge"
    );
}

/// Same as above for height: a box taller than a short/split pane must
/// shrink to fit rather than being positioned off-bounds.
#[test]
fn geometry_clamps_height_to_pane_when_content_is_taller() {
    let pane = rect(0, 0, 40, 8);
    let (_, y, _, h) = resolve_popup_geometry(5, 20, (5, 3), pane);
    assert_eq!(h, 8, "height must clamp to the full pane height");
    assert_eq!(
        y, 0,
        "clamped box has nowhere to go but the pane's top edge"
    );
}
