//! Cursor-anchored popup widget (`show-popup!`) — a floating text panel used
//! by hover, signature help, and (as `MenuModel`) the selection
//! menu / completion menu.
//!
//! Geometry rules, shared by every caller built on this widget:
//! - Preferred placement: below-right of the anchor cell.
//! - Flip above the anchor when the space below is smaller than the content
//!   and the space above is larger.
//! - Clamp horizontally so the popup never crosses the pane's right edge.
//! - Max width: `min(60, pane_width - 4)`. Max height: ⅓ of the pane's
//!   height (the hover-surface default threshold) — content taller
//!   than that is the *caller's* problem (hover overflows to the drawer).
//! - No border/padding options in v1 — one look, theme-scoped via
//!   `ui.popup`.
//!
//! All geometry (wrapping + flip + clamp) is resolved once, per frame, by
//! the write side (`Editor::sync_popup_view`) — using the *specific* pane's
//! rect it has access to there. `PopupOverlay::render` only paints at the
//! already-resolved `(x, y)`, with a final defensive clip against whatever
//! `pane_rect` it's actually given (belt-and-braces: the write side's rect
//! and the render rect are the same frame's geometry, so this should never
//! trigger — see the "never draws outside pane_rect" test).

use std::sync::{Arc, RwLock};

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;

use hume_engine::providers::OverlayProvider;
use hume_engine::render::fill_rect_bg;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

/// Maximum popup width in terminal columns, before any pane-width clamp.
pub(crate) const MAX_POPUP_WIDTH: u16 = 60;

/// `(show-popup! text)`'s raw, unwrapped content — held on `EditorState`
/// until the next frame's `sync_popup_view` resolves it into a positioned
/// [`PopupState`].
pub(crate) struct PopupModel {
    pub(crate) text: String,
}

/// `(show-menu! items on-select)`'s raw content — held on `EditorState`
/// until the next frame's `sync_menu_view` resolves it into a positioned
/// [`PopupState`] with `selected` set. `callback` fires exactly once (one
/// per selection or dismissal), then the whole model is dropped.
pub(crate) struct MenuModel {
    pub(crate) items: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) callback: steel::rvals::SteelVal,
}

/// Fully-resolved popup/menu content and position — computed once per frame
/// by the write side; the overlay only paints.
pub(crate) struct PopupState {
    /// Pre-wrapped display lines (word-wrapped to the resolved max width for
    /// a plain popup; one line per item, unwrapped, for a menu).
    pub(crate) lines: Vec<String>,
    /// Top-left screen cell to paint at (already flipped/clamped).
    pub(crate) x: u16,
    pub(crate) y: u16,
    /// The highlighted row index, for menus. `None` for a plain popup.
    pub(crate) selected: Option<usize>,
}

/// Generic overlay that paints a `PopupState` snapshot. Used directly for
/// hover-style popups (`show-popup!`) and, via a second registration with
/// its own `Arc`, for the selection menu and completion menu.
pub(crate) struct PopupOverlay {
    pub(crate) data: Arc<RwLock<Option<PopupState>>>,
    /// Scope resolved for the background/text fill (`ui.popup` for hover
    /// popups, `ui.menu` for menus).
    pub(crate) scope: &'static str,
    /// Scope for the highlighted row, used when `state.selected.is_some()`
    /// (menus only — `None` for plain popups, which never highlight a row).
    pub(crate) selected_scope: Option<&'static str>,
}

impl OverlayProvider for PopupOverlay {
    fn is_active(&self) -> bool {
        self.data.read().expect("RwLock not poisoned").is_some()
    }

    fn render(&self, pane_rect: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read().expect("RwLock not poisoned");
        let Some(state) = guard.as_ref() else { return };
        if state.lines.is_empty() {
            return;
        }

        let width = state
            .lines
            .iter()
            .map(|l| unicode_display_width(l))
            .max()
            .unwrap_or(0) as u16;
        let height = state.lines.len() as u16;

        // Defensive clip: the write side computed (x, y) against this same
        // pane's rect this same frame, so this should never trigger — but
        // painting outside the pane is worse than a dropped frame of content.
        if state.x < pane_rect.x
            || state.y < pane_rect.y
            || state.x + width > pane_rect.x + pane_rect.width
            || state.y + height > pane_rect.y + pane_rect.height
        {
            return;
        }

        let style = theme.resolve_by_name(Scope(self.scope)).into();
        let selected_style = self
            .selected_scope
            .map(|s| theme.resolve_by_name(Scope(s)).into());
        fill_rect_bg(buf, Rect::new(state.x, state.y, width, height), style);
        for (i, line) in state.lines.iter().enumerate() {
            let row_style = if state.selected == Some(i) {
                selected_style.unwrap_or(style)
            } else {
                style
            };
            if row_style != style {
                fill_rect_bg(
                    buf,
                    Rect::new(state.x, state.y + i as u16, width, 1),
                    row_style,
                );
            }
            buf.set_string(state.x, state.y + i as u16, line, row_style);
        }
    }
}

/// Resolve the fully-positioned `PopupState` for `lines` (already-wrapped
/// display text) anchored near `anchor` (cursor cell, absolute screen
/// coords) within `pane_rect`. Shared by every caller of this widget.
pub(crate) fn resolve_popup_geometry(
    lines: &[String],
    anchor: (u16, u16),
    pane_rect: Rect,
) -> (u16, u16) {
    let width = lines
        .iter()
        .map(|l| unicode_display_width(l))
        .max()
        .unwrap_or(0) as u16;
    let height = lines.len() as u16;

    let (anchor_x, anchor_y) = anchor;
    let space_below = (pane_rect.y + pane_rect.height).saturating_sub(anchor_y + 1);
    let space_above = anchor_y.saturating_sub(pane_rect.y);

    let y = if space_below >= height || space_below >= space_above {
        anchor_y + 1
    } else {
        anchor_y.saturating_sub(height)
    };
    let y = y
        .max(pane_rect.y)
        .min((pane_rect.y + pane_rect.height).saturating_sub(height));

    let x = anchor_x
        .max(pane_rect.x)
        .min((pane_rect.x + pane_rect.width).saturating_sub(width));

    (x, y)
}

/// Word-wrap `text` (newline-separated paragraphs preserved) to `max_width`
/// display columns, breaking on grapheme-cluster boundaries. Truncates to
/// `max_height` lines (a taller popup is the caller's problem — hover overflows
/// to the drawer).
pub(crate) fn wrap_text(text: &str, max_width: u16, max_height: u16) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;

    let max_width = max_width.max(1) as usize;
    let mut out = Vec::new();

    'paragraphs: for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            if out.len() as u16 >= max_height {
                break 'paragraphs;
            }
            continue;
        }
        let mut current = String::new();
        let mut current_w = 0usize;
        for word in paragraph.split(' ') {
            let word_w = unicode_display_width(word);
            // Would-be width if `word` were appended to the current line —
            // recomputed fresh each iteration (never carried across a break)
            // so a line-break never leaves a stale separator width behind.
            let would_be_w = if current.is_empty() {
                word_w
            } else {
                current_w + 1 + word_w
            };
            if would_be_w > max_width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_w = 0;
                if out.len() as u16 >= max_height {
                    break 'paragraphs;
                }
            }
            if word_w > max_width {
                // A single word wider than the line — hard-break it on
                // grapheme boundaries rather than overflow.
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    if out.len() as u16 >= max_height {
                        break 'paragraphs;
                    }
                }
                let mut piece = String::new();
                let mut piece_w = 0usize;
                for g in word.graphemes(true) {
                    let gw = unicode_display_width(g);
                    if piece_w + gw > max_width && !piece.is_empty() {
                        out.push(std::mem::take(&mut piece));
                        piece_w = 0;
                        if out.len() as u16 >= max_height {
                            break 'paragraphs;
                        }
                    }
                    piece.push_str(g);
                    piece_w += gw;
                }
                current = piece;
                current_w = piece_w;
                continue;
            }
            if !current.is_empty() {
                current.push(' ');
                current_w += 1;
            }
            current.push_str(word);
            current_w += word_w;
        }
        out.push(current);
        if out.len() as u16 >= max_height {
            break 'paragraphs;
        }
    }

    out.truncate(max_height as usize);
    out
}

fn unicode_display_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── wrap_text ──────────────────────────────────────────────────────────

    #[test]
    fn wrap_text_short_line_is_unchanged() {
        assert_eq!(wrap_text("hello", 60, 10), vec!["hello"]);
    }

    #[test]
    fn wrap_text_breaks_on_word_boundary() {
        assert_eq!(
            wrap_text("hello world foo", 11, 10),
            vec!["hello world", "foo"]
        );
    }

    #[test]
    fn wrap_text_preserves_explicit_newlines() {
        assert_eq!(
            wrap_text("line one\nline two", 60, 10),
            vec!["line one", "line two"]
        );
    }

    #[test]
    fn wrap_text_hard_breaks_an_overlong_word() {
        assert_eq!(wrap_text("abcdefghij", 4, 10), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_text_truncates_to_max_height() {
        let out = wrap_text("a\nb\nc\nd\ne", 60, 3);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn wrap_text_empty_line_preserved() {
        assert_eq!(wrap_text("a\n\nb", 60, 10), vec!["a", "", "b"]);
    }

    // ── resolve_popup_geometry ────────────────────────────────────────────

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn geometry_places_below_cursor_by_default() {
        let lines = vec!["hi".to_string()];
        let pane = rect(0, 0, 40, 20);
        let (x, y) = resolve_popup_geometry(&lines, (5, 5), pane);
        assert_eq!((x, y), (5, 6), "one line below the cursor row");
    }

    /// Content fits below the anchor even though there happens to be *more*
    /// room above than below — must still place it below. Distinguishes the
    /// "fits below" condition from a "prefer whichever side has more room"
    /// condition, which would flip it above unnecessarily here.
    #[test]
    fn geometry_stays_below_when_content_fits_even_with_more_room_above() {
        let lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        let pane = rect(0, 0, 40, 100);
        let (_, y) = resolve_popup_geometry(&lines, (5, 50), pane);
        assert_eq!(
            y, 51,
            "content (height 10) fits in the 49 rows below — must not flip \
             just because 50 rows happen to be available above"
        );
    }

    #[test]
    fn geometry_flips_above_near_bottom_edge() {
        let lines: Vec<String> = (0..5).map(|i| format!("line{i}")).collect();
        let pane = rect(0, 0, 40, 20);
        // Cursor near the bottom: only 2 rows below, 17 above — flip.
        let (_, y) = resolve_popup_geometry(&lines, (5, 18), pane);
        assert_eq!(y, 13, "flips to render entirely above the cursor row");
    }

    #[test]
    fn geometry_clamps_horizontally_at_right_edge() {
        let line = "a very long popup line here";
        let width = unicode_display_width(line) as u16;
        let lines = vec![line.to_string()];
        // Pane wide enough to hold the content, but the anchor sits close
        // enough to the right edge that placing the popup there unclamped
        // would overflow.
        let pane = rect(0, 0, 32, 20);
        let (x, _) = resolve_popup_geometry(&lines, (15, 5), pane);
        assert!(
            x + width <= pane.x + pane.width,
            "popup must not cross the pane's right edge, got x={x}, width={width}"
        );
    }

    #[test]
    fn geometry_never_escapes_pane_bounds_even_at_corner() {
        let lines = vec!["hello".to_string()];
        let pane = rect(2, 2, 10, 10);
        let (x, y) = resolve_popup_geometry(&lines, (2, 2), pane);
        assert!(x >= pane.x && x + 5 <= pane.x + pane.width);
        assert!(y >= pane.y && y + 1 <= pane.y + pane.height);
    }
}
