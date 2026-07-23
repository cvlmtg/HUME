//! Centered fuzzy-picker panel (`docs/FUZZY-FINDERS.md` B3) — bordered box
//! with a query input line on top and a ranked, scrolling item list below.
//!
//! Deliberately a sibling of [`super::menu_box`], not built on it: this
//! panel is a *fixed-size* box (sized as a fraction of the panes region,
//! independent of item count) with a two-zone layout (input row + list) and
//! an edge-anchored scroll model owned by `PickerSession` — `menu_box`'s
//! `visible_window` centers the selection instead, a different and
//! conflicting scroll model. The only thing shared is the border
//! glyph set.
//!
//! Write side ([`Editor::sync_picker_view`]) resolves geometry once per
//! frame against the current panes region and writes a [`PickerViewState`]
//! snapshot; [`PickerOverlay`] only paints it — same split as
//! [`super::popup::PopupOverlay`].
//!
use std::sync::{Arc, RwLock};

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;

use hume_engine::providers::OverlayProvider;
use hume_engine::render::fill_rect_bg;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

/// Maximum panel width/height in terminal cells, before the pane-fraction
/// clamp — mirrors `MAX_POPUP_WIDTH`'s role for the popup widget.
const MAX_PANEL_WIDTH: u16 = 100;
const MAX_PANEL_HEIGHT: u16 = 30;

/// Fully-resolved panel content and position — computed once per frame by
/// the write side (`Editor::sync_picker_view`); the overlay only paints.
pub(crate) struct PickerViewState {
    /// Raw query text (not yet tail-truncated — that's a paint-time
    /// concern, so this snapshot stays a dumb value, not pre-formatted UI).
    pub(crate) query: String,
    /// Display strings of the current scroll window, in ranked order
    /// (`PickerSession::window`'s output) — already scrolled; the widget
    /// never re-windows.
    pub(crate) rows: Vec<String>,
    /// On-screen index into `rows` of the selected row (`selected - scroll`
    /// from the store); `None` when `rows` is empty.
    pub(crate) selected_row: Option<usize>,
    /// `PickerSession::matched_len` / `total_len` — rendered as a
    /// right-aligned `"matched/total"` counter on the input row.
    pub(crate) matched: usize,
    pub(crate) total: usize,
    /// Outer footprint (including the 1-cell frame), centered in the panes
    /// region this same frame.
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// Fed from the `popup-border` setting, same as popup/menu/drawer.
    pub(crate) border: bool,
}

/// Resolved panel geometry — the single source of truth shared by the write
/// side (`sync_picker_view`, sizing the paint) and the key-interception side
/// (`handle_picker_key`, sizing `move_selection`'s `visible_rows`). Both call
/// this against the same `EditorState.view.last_pane_area`, so a keystroke
/// and the next paint always agree on how many rows are visible.
pub(crate) struct PanelGeometry {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// Inner list capacity: outer height minus the 1-cell top border, the
    /// input row, and the 1-cell bottom border.
    pub(crate) list_rows: usize,
}

/// Size the panel as a fraction of `pane_area` — width `min(80%, 100 cols)`,
/// height `min(60%, 30 rows)` — then center it. Returns `None` when the
/// region can't host a viable panel (narrower than 3 cols or shorter than
/// 4 rows, i.e. not even one list row) — callers then paint nothing rather
/// than a degenerate box.
pub(crate) fn panel_geometry(pane_area: Rect) -> Option<PanelGeometry> {
    let width = ((pane_area.width as u32 * 80 / 100) as u16)
        .min(MAX_PANEL_WIDTH)
        .min(pane_area.width);
    let height = ((pane_area.height as u32 * 60 / 100) as u16)
        .min(MAX_PANEL_HEIGHT)
        .min(pane_area.height);
    if width < 3 || height < 4 {
        return None;
    }
    let x = pane_area.x + (pane_area.width - width) / 2;
    let y = pane_area.y + (pane_area.height - height) / 2;
    Some(PanelGeometry {
        x,
        y,
        width,
        height,
        list_rows: (height - 3) as usize,
    })
}

/// Resolved styles for the three picker scopes, with the fallback aliasing
/// `Theme::resolve_raw`'s prefix-trim can't provide on its own (it never
/// crosses from `ui.picker*` to the sibling `ui.menu*` family):
/// `ui.picker` → else `ui.menu`; `ui.picker.selected` → else
/// `ui.menu.selected`; `ui.picker.input` → else `ui.picker` → else
/// `ui.menu`. Lets every existing Helix-derived theme (which defines
/// `ui.menu*` but not `ui.picker*`) render a usable picker unmodified.
pub(crate) struct PickerStyles {
    pub(crate) base: Style,
    pub(crate) selected: Style,
    pub(crate) input: Style,
}

pub(crate) fn picker_styles(theme: &Theme) -> PickerStyles {
    let base = resolve_or(theme, "ui.picker", "ui.menu");
    let selected = resolve_or(theme, "ui.picker.selected", "ui.menu.selected");
    let input = if theme.raw_contains("ui.picker.input") {
        theme.resolve_by_name(Scope("ui.picker.input")).into()
    } else if theme.raw_contains("ui.picker") {
        theme.resolve_by_name(Scope("ui.picker")).into()
    } else {
        theme.resolve_by_name(Scope("ui.menu")).into()
    };
    PickerStyles {
        base,
        selected,
        input,
    }
}

fn resolve_or(theme: &Theme, scope: &'static str, fallback: &'static str) -> Style {
    let name = if theme.raw_contains(scope) {
        scope
    } else {
        fallback
    };
    theme.resolve_by_name(Scope(name)).into()
}

/// Remove leading graphemes from `s` until its display width fits `budget`,
/// keeping the *tail* — so the cursor cell (always at the end of the query,
/// per the store's append/pop-at-end-only editing model) stays visible.
/// Grapheme-cluster aware, matching the project's text-boundary discipline.
fn truncate_tail(s: &str, budget: usize) -> String {
    if unicode_width::UnicodeWidthStr::width(s) <= budget {
        return s.to_string();
    }
    let mut acc = 0usize;
    let mut pieces: Vec<&str> = Vec::new();
    for g in s.graphemes(true).rev() {
        let w = unicode_width::UnicodeWidthStr::width(g);
        if acc + w > budget {
            break;
        }
        acc += w;
        pieces.push(g);
    }
    pieces.into_iter().rev().collect()
}

/// Paint the panel into `state`'s resolved outer rect. Pure function of its
/// arguments (styles pre-resolved by the caller, mirroring
/// `draw_menu_box`'s shape) — safe to call once per pane per frame even
/// though the overlay loop hands every pane the same whole-panes-region
/// rect (`hume-engine/src/pipeline/mod.rs`'s "may span panes" overlay pass).
///
/// Layout: row 0 inside the frame is the input line (query tail, a
/// reversed-cell cursor at the end, and a right-aligned `matched/total`
/// counter when it fits); the remaining rows are `state.rows`, with
/// `state.selected_row` highlighted across the full inner width. `rows` is
/// never re-windowed here — the store already scrolled it.
pub(crate) fn draw_picker_panel(
    buf: &mut ScreenBuf,
    state: &PickerViewState,
    base: Style,
    selected: Style,
    input: Style,
) {
    let outer = Rect::new(state.x, state.y, state.width, state.height);
    if outer.width < 3 || outer.height < 4 {
        return;
    }

    fill_rect_bg(buf, outer, base);

    if state.border {
        let right = outer.x + outer.width - 1;
        let bottom = outer.y + outer.height - 1;
        let fill_w = (outer.width - 2) as usize;
        let horiz: String = "─".repeat(fill_w);

        buf.set_string(outer.x, outer.y, "┌", base);
        buf.set_string(outer.x + 1, outer.y, &horiz, base);
        buf.set_string(right, outer.y, "┐", base);
        buf.set_string(outer.x, bottom, "└", base);
        buf.set_string(outer.x + 1, bottom, &horiz, base);
        buf.set_string(right, bottom, "┘", base);

        for row in 1..outer.height - 1 {
            buf.set_string(outer.x, outer.y + row, "│", base);
            buf.set_string(right, outer.y + row, "│", base);
        }
    }

    let inner_x = outer.x + 1;
    let inner_width = (outer.width - 2) as usize;
    let input_y = outer.y + 1;

    fill_rect_bg(buf, Rect::new(inner_x, input_y, outer.width - 2, 1), input);

    let counts = format!("{}/{}", state.matched, state.total);
    let counts_width = unicode_width::UnicodeWidthStr::width(counts.as_str());
    // Reserve 1 col for the cursor cell + 1 col of gap before the counter.
    let show_counts = inner_width >= counts_width + 2;
    let query_budget = if show_counts {
        inner_width - counts_width - 2
    } else {
        inner_width.saturating_sub(1)
    };
    let query_tail = truncate_tail(&state.query, query_budget);
    let query_width = unicode_width::UnicodeWidthStr::width(query_tail.as_str());

    buf.set_string(inner_x, input_y, &query_tail, input);
    let cursor_x = inner_x + query_width as u16;
    if cursor_x < inner_x + inner_width as u16 {
        buf.set_string(
            cursor_x,
            input_y,
            " ",
            input.add_modifier(Modifier::REVERSED),
        );
    }
    if show_counts {
        let counts_x = outer.x + outer.width - 1 - counts_width as u16;
        buf.set_string(counts_x, input_y, &counts, base);
    }

    let list_capacity = (outer.height - 3) as usize;
    for (i, row_text) in state.rows.iter().take(list_capacity).enumerate() {
        let y = outer.y + 2 + i as u16;
        if state.selected_row == Some(i) {
            let row_rect = Rect::new(inner_x, y, outer.width - 2, 1);
            fill_rect_bg(buf, row_rect, selected);
            buf.set_string(inner_x, y, row_text, selected);
        } else {
            buf.set_string(inner_x, y, row_text, base);
        }
    }
}

/// Overlay provider painting the picker panel — registered per-pane (last,
/// for top z-order, since the picker is full-modal and must sit above every
/// other overlay). See [`super::build_pane`].
pub(crate) struct PickerOverlay {
    pub(crate) data: Arc<RwLock<Option<PickerViewState>>>,
}

impl OverlayProvider for PickerOverlay {
    fn is_active(&self) -> bool {
        self.data.read().expect("RwLock not poisoned").is_some()
    }

    fn render(&self, pane_rect: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read().expect("RwLock not poisoned");
        let Some(state) = guard.as_ref() else {
            return;
        };

        let outer = Rect::new(state.x, state.y, state.width, state.height);
        // Defensive clip: the write side computed this rect against this
        // same pane's region this same frame (`PopupOverlay`'s precedent) —
        // should never trigger, but painting outside the pane is worse than
        // a dropped frame of content.
        if outer.x < pane_rect.x
            || outer.y < pane_rect.y
            || outer.x + outer.width > pane_rect.x + pane_rect.width
            || outer.y + outer.height > pane_rect.y + pane_rect.height
        {
            return;
        }

        let styles = picker_styles(theme);
        draw_picker_panel(buf, state, styles.base, styles.selected, styles.input);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
