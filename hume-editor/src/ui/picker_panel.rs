//! Centered fuzzy-picker panel — bordered box with a query input line on
//! top and a ranked, scrolling item list below.
//!
//! Deliberately a sibling of [`super::menu_box`], not built on it: this
//! panel is a *fixed-size* box (sized as a fraction of the panes region,
//! independent of item count) with a two-zone layout (input row + list) and
//! an edge-anchored scroll model owned by `PickerSession` — `menu_box`'s
//! selected-row window centers the selection instead, a different and
//! conflicting scroll model. The only thing shared is the border-drawing
//! routine itself, [`super::menu_box::draw_box_border`].
//!
//! Write side (`Editor::sync_picker_view`) resolves geometry once per
//! frame against the current panes region and writes a [`PickerViewState`]
//! snapshot; [`PickerOverlay`] only paints it — same split as
//! [`super::popup::PopupOverlay`].
//!
use hume_engine::types::ResolvedStyle;
use hume_grid::Rect;
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::OverlayProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;
use hume_scripting::host::TruncateEnd;

use super::width::{ELLIPSIS, ELLIPSIS_WIDTH, text_width, truncate_text, truncate_text_tail};

/// Maximum panel width/height in terminal cells, before the pane-fraction
/// clamp — mirrors `MAX_POPUP_WIDTH`'s role for the popup widget.
const MAX_PANEL_WIDTH: u16 = 100;
const MAX_PANEL_HEIGHT: u16 = 30;

/// Smallest panel `panel_geometry`/`draw_picker_panel` will paint into — a
/// single shared bound so the two can't drift on what counts as "too small
/// to render" (one place returned `None`, the other painted a truncated box
/// for the same input).
const MIN_PANEL_WIDTH: u16 = 3;
const MIN_PANEL_HEIGHT: u16 = 4;

/// Rows the border and input line always claim: the top border, the input
/// row, and the bottom border. Subtracted from the outer height exactly
/// once, in `panel_geometry`, to produce `PanelGeometry::list_rows` — the
/// row budget a keystroke pages against and the one a frame paints
/// (`PickerViewState::list_rows`, carried from the same call) are the same
/// number by construction rather than two derivations that have to agree.
const CHROME_ROWS: u16 = 3;

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
    pub(crate) rect: Rect,
    /// `PanelGeometry::list_rows` from the same `panel_geometry` call that
    /// produced `rect` — carried alongside it rather than re-subtracted
    /// from `rect.height` at paint time, so the row budget a keystroke
    /// paged against and the one a frame paints can never drift apart.
    pub(crate) list_rows: usize,
    /// Fed from the `popup-border` setting, same as popup/menu/drawer.
    pub(crate) border: bool,
    /// Which end of an over-long row this session clips — `#:truncate`,
    /// `PickerSession::truncate()`.
    pub(crate) truncate: TruncateEnd,
}

/// Resolved panel geometry — the single source of truth shared by the write
/// side (`sync_picker_view`, sizing the paint) and the key-interception side
/// (`handle_picker_key`, sizing `move_selection`'s `visible_rows`). Both call
/// this against the same `EditorState.view.last_pane_area`, so a keystroke
/// and the next paint always agree on how many rows are visible.
pub(crate) struct PanelGeometry {
    pub(crate) rect: Rect,
    /// Inner list capacity: outer height minus [`CHROME_ROWS`].
    pub(crate) list_rows: usize,
}

/// Size the panel as a fraction of `pane_area` — width `min(80%, 100 cols)`,
/// height `min(60%, 30 rows)` — then center it. Returns `None` when the
/// region can't host a viable panel (narrower than [`MIN_PANEL_WIDTH`] or
/// shorter than [`MIN_PANEL_HEIGHT`], i.e. not even one list row) — callers
/// then paint nothing rather than a degenerate box.
pub(crate) fn panel_geometry(pane_area: Rect) -> Option<PanelGeometry> {
    let width = ((pane_area.width as u32 * 80 / 100) as u16)
        .min(MAX_PANEL_WIDTH)
        .min(pane_area.width);
    let height = ((pane_area.height as u32 * 60 / 100) as u16)
        .min(MAX_PANEL_HEIGHT)
        .min(pane_area.height);
    if width < MIN_PANEL_WIDTH || height < MIN_PANEL_HEIGHT {
        return None;
    }
    Some(PanelGeometry {
        rect: pane_area.centered(width, height),
        list_rows: (height - CHROME_ROWS) as usize,
    })
}

/// Resolved styles for the picker body — Helix's own picker scopes, not a
/// HUME invention: `ui.background` (panel fill), `ui.text` (border/rows/
/// query), `ui.text.focus` (the selected row — Helix's docs call it "the
/// currently selected line in the picker"), `ui.cursor.primary` (query block
/// cursor). `Theme::resolve_raw`'s prefix-trim already degrades each of
/// these to `default` when a theme omits it, so no custom fallback layer is
/// needed here.
#[derive(Clone, Copy)]
pub(crate) struct PickerStyles {
    pub(crate) background: ResolvedStyle,
    pub(crate) text: ResolvedStyle,
    pub(crate) selected: ResolvedStyle,
    pub(crate) cursor: ResolvedStyle,
}

pub(crate) fn picker_styles(theme: &Theme) -> PickerStyles {
    let by = |name| theme.resolve_by_name(Scope(name));
    PickerStyles {
        background: by("ui.background"),
        text: by("ui.text"),
        selected: by("ui.text.focus"),
        cursor: by("ui.cursor.primary"),
    }
}

/// Clip `s` to `budget` display cells per `cut`, marking the dropped end
/// with `…` — list rows (file paths, grep matches, …) whose distinguishing
/// part can sit at either end depending on what a picker's source shows.
/// Grapheme-cluster aware via [`truncate_text`]/[`truncate_text_tail`].
/// Kept distinct from the query row's own tail-truncation
/// (`draw_picker_panel`'s direct `truncate_text_tail` call) because the
/// query row must never gain a marker — the query is the user's editable
/// text, and its bare tail (no `…`) is intentional there.
///
/// Borrows `s` unchanged on the (common) no-truncation path instead of
/// allocating a copy of every visible row every frame.
///
/// `width.rs`'s helpers are *keep*-oriented (`truncate_text_tail` keeps the
/// tail) while [`TruncateEnd`] is *cut*-oriented, so the arms below read
/// inverted: cutting the head keeps — and thus calls — `truncate_text_tail`.
fn truncate_marked(s: &str, budget: usize, cut: TruncateEnd) -> std::borrow::Cow<'_, str> {
    if text_width(s) <= budget {
        return std::borrow::Cow::Borrowed(s);
    }
    if budget == 0 {
        return std::borrow::Cow::Borrowed("");
    }
    let kept = budget.saturating_sub(ELLIPSIS_WIDTH);
    match cut {
        TruncateEnd::Head => {
            let (tail, _) = truncate_text_tail(s, kept);
            std::borrow::Cow::Owned(format!("{ELLIPSIS}{tail}"))
        }
        TruncateEnd::Tail => {
            let (head, _) = truncate_text(s, kept);
            std::borrow::Cow::Owned(format!("{head}{ELLIPSIS}"))
        }
    }
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
    canvas: &mut Canvas,
    state: &PickerViewState,
    styles: PickerStyles,
) {
    let outer = state.rect;
    if outer.width < MIN_PANEL_WIDTH || outer.height < MIN_PANEL_HEIGHT {
        return;
    }

    canvas.fill_rect_bg(outer, styles.background);

    if state.border {
        super::menu_box::draw_box_border(canvas, outer, styles.text);
    }

    let inner = outer.inset(1, 1);
    let inner_x = inner.x;
    let inner_width = inner.width as usize;
    // One bound for every text write below: the panel's inner right edge.
    // Each string is already truncated to fit, so this is a backstop that
    // keeps a mis-sized one inside the border instead of over it.
    let inner_right = inner.right();
    let input_y = inner.y;

    let counts = if state.pending {
        format!("{}/{} …", state.matched, state.total)
    } else {
        format!("{}/{}", state.matched, state.total)
    };
    let counts_width = text_width(&counts);

    // Clip the prompt itself only in the pathological case where it alone
    // exceeds the inner width — the common case (empty or a short label)
    // leaves this a no-op, so an empty prompt renders identically to no
    // prompt at all. The readable prefix matters more than the tail for a
    // fixed label, so this keeps the *head* (unlike the query's own
    // tail-truncation below).
    let (prompt_shown, prompt_width) = truncate_text(&state.prompt, inner_width);
    if prompt_width > 0 {
        canvas.write_text_run(inner_x, input_y, prompt_shown, styles.text, inner_right);
    }

    let after_prompt_width = inner_width.saturating_sub(prompt_width);
    // Reserve 1 col for the cursor cell + 1 col of gap before the counter.
    let show_counts = after_prompt_width >= counts_width + 2;
    let query_budget = if show_counts {
        after_prompt_width - counts_width - 2
    } else {
        after_prompt_width.saturating_sub(1)
    };
    // Remove leading graphemes until the query fits `query_budget`, keeping
    // the *tail* — so the cursor cell (always at the end of the query, per
    // the store's append/pop-at-end-only editing model) stays visible.
    let (query_tail, query_width) = truncate_text_tail(&state.query, query_budget);

    let query_x = inner_x + prompt_width as u16;
    canvas.write_text_run(query_x, input_y, query_tail, styles.text, inner_right);
    let cursor_x = query_x + query_width as u16;
    if cursor_x < inner_right {
        canvas.write_text_run(cursor_x, input_y, " ", styles.cursor, inner_right);
    }
    if show_counts {
        let counts_x = inner.right() - counts_width as u16;
        canvas.write_text_run(counts_x, input_y, &counts, styles.text, inner_right);
    }

    for (i, row_text) in state.rows.iter().take(state.list_rows).enumerate() {
        // Row 0 of `inner` is the input line (`input_y`); the list starts on
        // the row below it.
        let y = inner.y + 1 + i as u16;
        let shown = truncate_marked(row_text, inner_width, state.truncate);
        super::menu_box::draw_list_row(
            canvas,
            inner_x,
            y,
            inner.width,
            inner_right,
            &shown,
            state.selected_row == Some(i),
            styles.selected,
            styles.text,
        );
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

    fn render(&self, pane_rect: Rect, theme: &Theme, canvas: &mut Canvas) {
        let guard = self.data.read_or_panic();
        let Some(state) = guard.as_ref() else {
            return;
        };

        // Defensive clip: the write side computed this rect against this
        // same pane's region this same frame — see `fits_inside`'s doc.
        if !super::menu_box::fits_inside(state.rect, pane_rect) {
            return;
        }

        let styles = picker_styles(theme);
        draw_picker_panel(canvas, state, styles);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
