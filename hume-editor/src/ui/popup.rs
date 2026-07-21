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
//! - Framed with a 1-cell border on all sides, theme-scoped via `ui.popup`
//!   (or `ui.menu` for menus) — box-drawing glyphs when the `popup-border`
//!   setting is on, a plain background margin when it's off. Rendering
//!   (frame, scroll window, rows) is shared with `MinibufCompletionOverlay` via
//!   [`super::menu_box`].
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
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

use super::menu_box::draw_menu_box;

/// Maximum popup width in terminal columns, before any pane-width clamp.
pub(crate) const MAX_POPUP_WIDTH: u16 = 60;

/// `(show-popup! text)`'s raw, unwrapped content — held on `EditorState`
/// until the next frame's `sync_popup_view` resolves it into a positioned
/// [`PopupState`].
pub(crate) struct PopupModel {
    pub(crate) text: String,
    /// `#:dismiss-on-key` — when true, `Editor::handle_key` clears this
    /// model at the start of the *next* key event, regardless of what key
    /// it is. Hover/signature-help popups leave this `false` and keep their
    /// existing `on-mode-change` dismissal.
    pub(crate) dismiss_on_key: bool,
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
    /// Top-left screen cell to paint at (already flipped/clamped) — the
    /// outer frame corner, not the first content cell.
    pub(crate) x: u16,
    pub(crate) y: u16,
    /// Outer footprint (including the 1-cell frame) the write side sized
    /// `(x, y)` against. Each caller's row cap differs (hover: `max_height`
    /// from `popup_anchor_and_bounds`; menus/LSP completion: `MAX_MENU_ROWS`
    /// with a scroll window) — carrying the resolved size here, rather than
    /// re-deriving it from `lines.len()` at render time, keeps the painted
    /// box and the positioned box the same box.
    pub(crate) outer_w: u16,
    pub(crate) outer_h: u16,
    /// The highlighted row index, for menus. `None` for a plain popup.
    pub(crate) selected: Option<usize>,
    /// Whether to draw box-drawing border glyphs around the popup (vs. a
    /// plain background-filled 1-cell margin). Fed from the `popup-border`
    /// setting.
    pub(crate) border: bool,
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

        let outer = Rect::new(state.x, state.y, state.outer_w, state.outer_h);

        // Defensive clip: the write side computed (x, y) against this same
        // pane's rect this same frame, so this should never trigger — but
        // painting outside the pane is worse than a dropped frame of content.
        if outer.x < pane_rect.x
            || outer.y < pane_rect.y
            || outer.x + outer.width > pane_rect.x + pane_rect.width
            || outer.y + outer.height > pane_rect.y + pane_rect.height
        {
            return;
        }

        let style = theme.resolve_by_name(Scope(self.scope)).into();
        let selected_style = self
            .selected_scope
            .map(|s| theme.resolve_by_name(Scope(s)).into())
            .unwrap_or(style);
        draw_menu_box(
            buf,
            outer,
            &state.lines,
            state.selected,
            state.border,
            style,
            selected_style,
        );
    }
}

/// Resolve the top-left corner and clamped size for a `width` × `height` box
/// (the outer footprint, including any frame) anchored near `anchor`
/// (cursor cell, absolute screen coords) within `pane_rect`. Shared by every
/// caller of this widget — callers pass their content size plus the 2-cell
/// frame reserved for the border.
///
/// `width`/`height` are clamped to `pane_rect`'s size before the position is
/// resolved, so the returned box always fits inside the pane — callers must
/// use the returned size, not their original request, when painting.
/// `PopupOverlay`'s bounds check is a defensive backstop, not a substitute
/// for this: without the clamp, a box wider or taller than the pane can
/// never satisfy that check, and the whole popup silently fails to render.
pub(crate) fn resolve_popup_geometry(
    width: u16,
    height: u16,
    anchor: (u16, u16),
    pane_rect: Rect,
) -> (u16, u16, u16, u16) {
    let width = width.min(pane_rect.width);
    let height = height.min(pane_rect.height);
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

    (x, y, width, height)
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
mod tests;
