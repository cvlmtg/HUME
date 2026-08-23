//! Centered fuzzy-picker panel — bordered box with a query input line on
//! top and a ranked, scrolling item list below.
//!
//! Deliberately a sibling of [`super::menu_box`], not built on it: this
//! panel is a *fixed-size* box (sized as a fraction of the panes region,
//! independent of item count) with a two-zone layout (input row + list) and
//! an edge-anchored scroll model owned by `PickerSession` — `menu_box`'s
//! `visible_window` centers the selection instead, a different and
//! conflicting scroll model. The only thing shared is the border-drawing
//! routine itself, [`super::menu_box::draw_box_border`].
//!
//! Write side (`Editor::sync_picker_view`) resolves geometry once per
//! frame against the current panes region and writes a [`PickerViewState`]
//! snapshot; [`PickerOverlay`] only paints it — same split as
//! [`super::popup::PopupOverlay`].
//!
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;

use hume_engine::providers::OverlayProvider;
use hume_engine::render::{fill_rect_bg, write_text_run};
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

use super::width::{text_width, truncate_text, truncate_text_tail};

/// Maximum panel width/height in terminal cells, before the pane-fraction
/// clamp — mirrors `MAX_POPUP_WIDTH`'s role for the popup widget.
const MAX_PANEL_WIDTH: u16 = 100;
const MAX_PANEL_HEIGHT: u16 = 30;

/// Fully-resolved panel content and position — computed once per frame by
/// the write side (`Editor::sync_picker_view`); the overlay only paints.
pub(crate) struct PickerViewState {
    /// Label painted before the query on the input row, e.g. `"files: "` —
    /// empty by default, in which case the input row renders exactly as it
    /// did before prompts existed. Not yet width-clipped — paint-time
    /// concern, same as `query`.
    pub(crate) prompt: String,
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
    /// `PickerSession::is_pending` — appends a "still arriving" marker to
    /// the counter so a picker opened empty (`spawn-async!`-backed, or a
    /// live `picker-source-spawn!` source) doesn't read as "zero results"
    /// while its job is still running.
    pub(crate) pending: bool,
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

/// Resolved styles for the picker body — Helix's own picker scopes, not a
/// HUME invention: `ui.background` (panel fill), `ui.text` (border/rows/
/// query), `ui.text.focus` (the selected row — Helix's docs call it "the
/// currently selected line in the picker"), `ui.cursor.primary` (query block
/// cursor). `Theme::resolve_raw`'s prefix-trim already degrades each of
/// these to `default` when a theme omits it, so no custom fallback layer is
/// needed here.
pub(crate) struct PickerStyles {
    pub(crate) background: Style,
    pub(crate) text: Style,
    pub(crate) selected: Style,
    pub(crate) cursor: Style,
}

pub(crate) fn picker_styles(theme: &Theme) -> PickerStyles {
    let by = |name| theme.resolve_by_name(Scope(name)).into();
    PickerStyles {
        background: by("ui.background"),
        text: by("ui.text"),
        selected: by("ui.text.focus"),
        cursor: by("ui.cursor.primary"),
    }
}

/// Remove leading graphemes from `s` until its display width fits `budget`,
/// keeping the *tail* — so the cursor cell (always at the end of the query,
/// per the store's append/pop-at-end-only editing model) stays visible.
fn truncate_tail(s: &str, budget: usize) -> String {
    truncate_text_tail(s, budget).to_string()
}

/// Remove trailing graphemes from `s` until its display width fits `budget`,
/// keeping the *head* — the prompt is a fixed label, not something the user
/// is editing, so if it must be clipped at all (a pathologically narrow
/// panel), the readable prefix matters more than the tail.
fn truncate_head(s: &str, budget: usize) -> String {
    truncate_text(s, budget).to_string()
}

/// Clip `s` to `budget` display cells, keeping the *tail* and prefixing a
/// `…` marker when anything was dropped — for list rows (e.g. file paths)
/// the distinguishing part (the basename) sits at the end. Grapheme-cluster
/// aware via [`truncate_tail`]. Kept distinct from `truncate_tail` because
/// the query row must never gain a marker — the query is the user's
/// editable text, and its bare tail (no `…`) is intentional there.
fn truncate_tail_marked(s: &str, budget: usize) -> String {
    if text_width(s) <= budget {
        return s.to_string();
    }
    if budget == 0 {
        return String::new();
    }
    format!("…{}", truncate_tail(s, budget - 1))
}

/// Paint the panel into `state`'s resolved outer rect. Pure function of its
/// arguments (styles pre-resolved by the caller, mirroring
/// `draw_menu_box`'s shape) — safe to call once per pane per frame even
/// though the overlay loop hands every pane the same whole-panes-region
/// rect (`hume-engine/src/pipeline/mod.rs`'s "may span panes" overlay pass).
///
/// Layout: row 0 inside the frame is the input line (query tail, a block
/// cursor cell at the end styled `ui.cursor.primary`, and a right-aligned
/// `matched/total` counter when it fits); the remaining rows are `state.rows`, with
/// `state.selected_row` highlighted across the full inner width. `rows` is
/// never re-windowed here — the store already scrolled it.
pub(crate) fn draw_picker_panel(
    buf: &mut ScreenBuf,
    state: &PickerViewState,
    background: Style,
    text: Style,
    selected: Style,
    cursor: Style,
    invisible_style: Style,
) {
    let outer = Rect::new(state.x, state.y, state.width, state.height);
    if outer.width < 3 || outer.height < 4 {
        return;
    }

    fill_rect_bg(buf, outer, background);

    if state.border {
        super::menu_box::draw_box_border(buf, outer, text, invisible_style);
    }

    let inner_x = outer.x + 1;
    let inner_width = (outer.width - 2) as usize;
    // One bound for every text write below: the panel's inner right edge.
    // Each string is already truncated to fit, so this is a backstop that
    // keeps a mis-sized one inside the border instead of over it.
    let inner_right = inner_x + inner_width as u16;
    let input_y = outer.y + 1;

    let counts = if state.pending {
        format!("{}/{} …", state.matched, state.total)
    } else {
        format!("{}/{}", state.matched, state.total)
    };
    let counts_width = text_width(&counts);

    // Clip the prompt itself only in the pathological case where it alone
    // exceeds the inner width — the common case (empty or a short label)
    // leaves this a no-op, so an empty prompt renders identically to no
    // prompt at all.
    let prompt_shown = truncate_head(&state.prompt, inner_width);
    let prompt_width = text_width(&prompt_shown);
    if prompt_width > 0 {
        write_text_run(
            buf,
            inner_x,
            input_y,
            &prompt_shown,
            text,
            invisible_style,
            inner_right,
        );
    }

    let after_prompt_width = inner_width.saturating_sub(prompt_width);
    // Reserve 1 col for the cursor cell + 1 col of gap before the counter.
    let show_counts = after_prompt_width >= counts_width + 2;
    let query_budget = if show_counts {
        after_prompt_width - counts_width - 2
    } else {
        after_prompt_width.saturating_sub(1)
    };
    let query_tail = truncate_tail(&state.query, query_budget);
    let query_width = text_width(&query_tail);

    let query_x = inner_x + prompt_width as u16;
    write_text_run(
        buf,
        query_x,
        input_y,
        &query_tail,
        text,
        invisible_style,
        inner_right,
    );
    let cursor_x = query_x + query_width as u16;
    if cursor_x < inner_x + inner_width as u16 {
        write_text_run(
            buf,
            cursor_x,
            input_y,
            " ",
            cursor,
            invisible_style,
            inner_right,
        );
    }
    if show_counts {
        let counts_x = outer.x + outer.width - 1 - counts_width as u16;
        write_text_run(
            buf,
            counts_x,
            input_y,
            &counts,
            text,
            invisible_style,
            inner_right,
        );
    }

    let list_capacity = (outer.height - 3) as usize;
    for (i, row_text) in state.rows.iter().take(list_capacity).enumerate() {
        let y = outer.y + 2 + i as u16;
        let shown = truncate_tail_marked(row_text, inner_width);
        if state.selected_row == Some(i) {
            let row_rect = Rect::new(inner_x, y, outer.width - 2, 1);
            fill_rect_bg(buf, row_rect, selected);
            write_text_run(
                buf,
                inner_x,
                y,
                &shown,
                selected,
                invisible_style,
                inner_right,
            );
        } else {
            write_text_run(buf, inner_x, y, &shown, text, invisible_style, inner_right);
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
        self.data.read_or_panic().is_some()
    }

    fn render(&self, pane_rect: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read_or_panic();
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
        draw_picker_panel(
            buf,
            state,
            styles.background,
            styles.text,
            styles.selected,
            styles.cursor,
            theme.ui.invisible.into(),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
