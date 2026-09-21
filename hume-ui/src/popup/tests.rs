use super::*;
use hume_engine::types::ResolvedStyle;
use hume_grid::{Rect, Rgb};

// ── wrap_text ──────────────────────────────────────────────────────────
//
// Production code no longer has a separate `wrap_text` — plain popup text
// is exactly a single default-style run through `wrap_styled` (see
// `PopupContent::plain`). This local helper keeps that single-style edge-
// case coverage (word boundaries, hard breaks, explicit newlines) without
// duplicating a wrapper `wrap_styled` itself made redundant.
fn wrap_text(text: &str, max_width: u16) -> Vec<String> {
    let runs = [(text.to_string(), ResolvedStyle::default())];
    wrap_styled(&runs, max_width)
        .into_iter()
        .map(|row| row.into_iter().map(|(s, _)| s).collect())
        .collect()
}

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

fn red() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(Rgb(255, 0, 0)),
        ..Default::default()
    }
}

#[test]
fn wrap_styled_preserves_a_style_boundary_within_one_row() {
    let runs = [
        ("hello ".to_string(), ResolvedStyle::default()),
        ("world".to_string(), red()),
    ];
    let rows = wrap_styled(&runs, 60);
    assert_eq!(rows.len(), 1, "both words fit on one row");
    assert_eq!(
        rows[0],
        vec![
            ("hello".to_string(), ResolvedStyle::default()),
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
        ("aaaa ".to_string(), ResolvedStyle::default()),
        ("bbbb".to_string(), red()),
    ];
    let rows = wrap_styled(&runs, 4);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        vec![("aaaa".to_string(), ResolvedStyle::default())]
    );
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
    let width = hume_rope::width::str_width("a very long popup line here", 0, 1) as u16;
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

// ── resolve_menu ──────────────────────────────────────────────────────

fn placement(pane: Rect) -> PopupPlacement {
    PopupPlacement {
        anchor: (0, 0),
        pane_rect: pane,
        content_width: pane.width,
    }
}

/// A candidate scrolled out of the visible window must never widen the
/// box — the bug this fixes: the old `menu_inner_width` measured every
/// filtered candidate, so one very wide label anywhere in a long list
/// inflated the menu even while scrolled away from it.
#[test]
fn resolve_menu_width_reflects_only_the_visible_window_not_the_whole_list() {
    let mut rows: Vec<MenuRow> = (0..15).map(|i| MenuRow::plain(format!("r{i}"))).collect();
    rows[0] = MenuRow::plain("x".repeat(100));

    // selected = 14 (the last row) centers the window well past the wide
    // row 0 — window_range(15, 14 - 10/2 = 9, 10) clamps to [5, 15).
    let state = resolve_menu(
        MenuRows::two_column(rows),
        14,
        placement(rect(0, 0, 200, 50)),
        true,
    );
    assert_eq!(
        state.rect.width, 5,
        "widest *visible* row is \"r10\"..\"r14\" (3 chars) + the 2-cell frame, \
         not the 100-char row scrolled out of view"
    );
}

/// Two-column composition: `trailing` right-aligned flush against the
/// widest of it in the window, `main` left-aligned and padded to the
/// widest of it — an exact fit, no truncation.
#[test]
fn resolve_menu_right_aligns_trailing_when_it_fits() {
    let rows = vec![
        MenuRow {
            main: "kitty_support".to_string(),
            trailing: Some("bool".to_string()),
        },
        MenuRow {
            main: "kind".to_string(),
            trailing: Some("Kind".to_string()),
        },
    ];
    let state = resolve_menu(
        MenuRows::two_column(rows),
        0,
        placement(rect(0, 0, 200, 50)),
        true,
    );
    assert_eq!(
        state.lines.as_ref(),
        &vec![
            "kitty_support  bool".to_string(),
            "kind           Kind".to_string(),
        ],
        "main padded to the widest main (\"kitty_support\", 13), a 2-cell \
         gap, then trailing right-aligned to the widest trailing (4)"
    );
}

/// A pane too narrow for both columns truncates `trailing` with an
/// ellipsis rather than pushing the box past the pane clamp.
#[test]
fn resolve_menu_truncates_trailing_when_the_pane_is_too_narrow() {
    let rows = vec![MenuRow {
        main: "assert!".to_string(),
        trailing: Some("macro_rules! assert".to_string()),
    }];
    // max_inner = pane.width - 2 = 10; main_col = 7 ("assert!"); trail_col
    // = min(19, 10 - 7 - 2) = 1 — just enough for the ellipsis marker.
    let state = resolve_menu(
        MenuRows::two_column(rows),
        0,
        placement(rect(0, 0, 12, 50)),
        true,
    );
    assert_eq!(state.lines.as_ref(), &vec!["assert!  …".to_string()]);
}

/// When there's no room left for a trailing column at all (`trail_col`
/// clamps to 0), every row's trailing part is dropped uniformly — never a
/// half-truncated fragment with no gap before it.
#[test]
fn resolve_menu_drops_trailing_entirely_when_no_room_is_left() {
    let rows = vec![MenuRow {
        main: "ab".to_string(),
        trailing: Some("xy".to_string()),
    }];
    // max_inner = pane.width - 2 = 2, all consumed by `main` alone.
    let state = resolve_menu(
        MenuRows::two_column(rows),
        0,
        placement(rect(0, 0, 4, 50)),
        true,
    );
    assert_eq!(state.lines.as_ref(), &vec!["ab".to_string()]);
}

/// A session narrowed to zero matches must not panic computing `selected`
/// against an empty list — a real path (`sync_completion_menu_view` calls
/// `resolve_menu` unconditionally, whether or not the filter matched
/// anything).
#[test]
fn resolve_menu_on_an_empty_list_does_not_panic() {
    let state = resolve_menu(
        MenuRows::plain(Vec::new()),
        0,
        placement(rect(0, 0, 40, 20)),
        true,
    );
    assert!(state.lines.is_empty());
    assert_eq!(state.selected, None);
}

// ── band_visible_rows ─────────────────────────────────────────────────

/// The overflow-safety guard on the shared arithmetic itself lives in
/// `menu_box::tests` (`band_capacity_clamps_instead_of_overflowing_u16`) —
/// this pins the popup band's own 2-row frame against it instead of
/// re-testing the arithmetic.
#[test]
fn band_visible_rows_reserves_the_frame_row_pair_below_the_cap() {
    assert_eq!(
        band_visible_rows(3, 20),
        3,
        "well under the cap: all 3 lines fit"
    );
}
