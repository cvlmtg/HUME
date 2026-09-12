//! Tab bar (`tabline` setting) — Vim's sense: a tab is a saved window
//! layout, not a per-buffer strip. See `editor::tab`'s module doc for the
//! model.
//!
//! Same three-type split as `hume_ui::drawer`: [`TablineViewState`] is the
//! read-side snapshot [`TablineWidget`] paints from, written every frame by
//! `Editor::sync_tabline_view` (self-healing, same rationale as the
//! drawer's own unconditional per-frame sync — see its module doc). Lives
//! here, not in `hume-ui`, for the same reason the statusline does (see
//! that module's own doc): `TabEntry`/`TablineViewState` hold a `TabId`,
//! an `editor`-crate type `hume-ui` cannot depend on.

use hume_grid::Rect;

use crate::editor::tab::TabId;
use crate::statusline::colors::TablineColors;
use hume_engine::types::TruncateEnd;

use hume_engine::lock::SharedSlot;
use hume_engine::providers::TabBarProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;

/// One tab's rendered label, already carrying its dirty marker (`"main.rs"`,
/// `"lib.rs+"`) — the widget never re-derives it from a live `Buffer`, same
/// "push formatted text, never a live handle" contract `DrawerViewState`
/// follows for its rows.
#[derive(Clone)]
pub(crate) struct TabEntry {
    pub(crate) id: TabId,
    pub(crate) label: String,
    /// `text_width(&label)`, measured once here rather than by every
    /// `pack_into`/`fit_label` call that reads this entry — a candidate
    /// scroll value or a re-render costs a re-walk of `ranges`/`tabs`
    /// otherwise, not a re-walk of each label's own graphemes.
    pub(crate) label_width: u16,
}

impl TabEntry {
    pub(crate) fn new(id: TabId, label: String) -> Self {
        let label_width = hume_ui::width::text_width(&label) as u16;
        Self {
            id,
            label,
            label_width,
        }
    }
}

/// Read-side snapshot for [`TablineWidget`].
#[derive(Default, Clone)]
pub(crate) struct TablineViewState {
    /// Every open tab, in display order (mirrors `TabStore::order`).
    pub(crate) tabs: Vec<TabEntry>,
    /// Index into `tabs` of the active one.
    pub(crate) active_index: usize,
    /// First visible tab index — clamped to keep `active_index` in view by
    /// `Editor::sync_tabline_view`, mirroring the drawer's own scroll.
    pub(crate) scroll: usize,
    /// Resolved from the `tabline` setting + tab count each sync, so
    /// `pane_area`'s chrome arithmetic (which reads `height()`) and this
    /// row's own painted content can never disagree about whether the tab
    /// bar is showing this frame.
    pub(crate) visible: bool,
}

/// Engine-facing tab-bar provider — one instance, owned by `EngineView`
/// (chrome, not per-pane), constructed once at `Editor::open` time.
pub(crate) struct TablineWidget {
    pub(crate) data: SharedSlot<TablineViewState>,
}

impl TabBarProvider for TablineWidget {
    fn height(&self) -> u16 {
        if self.data.read().visible { 1 } else { 0 }
    }

    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read();
        if !guard.visible {
            return;
        }

        let colors = TablineColors::from_theme(theme);
        canvas.fill_rect_bg(area, colors.inactive);

        let extents = tab_extents(&guard.tabs, guard.scroll, area.x, area.width);
        if extents.indicator_left {
            canvas.write_text_run(area.x, area.y, OVERFLOW_LEFT, colors.inactive, area.right());
        }
        for (i, &(start_x, end_x)) in extents.ranges.iter().enumerate() {
            let idx = guard.scroll + i;
            let entry = &guard.tabs[idx];
            let style = if idx == guard.active_index {
                colors.active
            } else {
                colors.inactive
            };
            let label = fit_label(&entry.label, end_x - start_x);
            // `end_x`, not `area.right()`: keeps a label inside its own
            // extent by construction rather than by `fit_label`'s arithmetic
            // alone — the forced single-tab fallback in `tab_extents` can
            // clamp an extent to as little as 1 cell, narrower than
            // `fit_label`'s own 2-cell padding minimum.
            canvas.write_text_run(start_x, area.y, &label, style, end_x);
        }
        if extents.clipped_tail {
            canvas.write_text_run(
                area.right().saturating_sub(1),
                area.y,
                OVERFLOW_RIGHT,
                colors.inactive,
                area.right(),
            );
        }
    }
}

const OVERFLOW_LEFT: &str = "‹";
const OVERFLOW_RIGHT: &str = "›";

/// Pad `label` the way every tab is (`" {label} "`), truncating with `…`
/// when the padded form doesn't fit `extent_w` cells — a label `tab_extents`
/// packed into an extent narrower than its own width (the forced single-tab
/// fallback for a row too narrow to fit even one full label) or a
/// genuinely long name at any width.
fn fit_label(label: &str, extent_w: u16) -> String {
    let budget = extent_w.saturating_sub(2);
    format!(
        " {} ",
        hume_ui::width::truncate_marked(label, budget as usize, TruncateEnd::Tail)
    )
}

/// One visible tab's on-row extent, in absolute screen columns — `[start,
/// end)`, `start`-ordered, one entry per tab from `scroll` onward that fits
/// in `width` starting at `x_origin`. Shared by three callers, all of which
/// must pass the same `(x_origin, width)` for a given frame or the geometry
/// each computes will disagree: [`TablineWidget::render`] (draws each
/// extent) and `Editor::tabline_click` (`mouse.rs`, maps a click column back
/// to a tab index) both read them from `EngineView::tabbar_area`, and
/// `Editor::sync_tabline_view` (`frame.rs`, resolves `scroll` itself before
/// either of the other two runs) reads from the same place — so painting,
/// hit-testing, and scroll resolution can never disagree about where a tab
/// starts and ends.
#[derive(Default)]
pub(crate) struct TabExtents {
    /// `ranges[i]` is the extent of `tabs[scroll + i]`.
    pub(crate) ranges: Vec<(u16, u16)>,
    /// Whether `scroll > 0` — tabs exist before the visible window, so the
    /// leading `‹` indicator (and its reserved column) applies.
    pub(crate) indicator_left: bool,
    /// Whether at least one tab past the visible window didn't fit — the
    /// trailing `›` indicator's own condition.
    pub(crate) clipped_tail: bool,
}

pub(crate) fn tab_extents(
    tabs: &[TabEntry],
    scroll: usize,
    x_origin: u16,
    width: u16,
) -> TabExtents {
    let mut ranges = Vec::new();
    let (indicator_left, clipped_tail) =
        tab_extents_into(tabs, scroll, x_origin, width, &mut ranges);
    TabExtents {
        ranges,
        indicator_left,
        clipped_tail,
    }
}

/// [`tab_extents`]'s packing core, writing into a caller-supplied `ranges`
/// buffer (cleared first) instead of allocating a fresh one — returns
/// `(indicator_left, clipped_tail)`, the two flags `tab_extents` bundles
/// alongside its own owned `Vec`. `tab_extents` is a thin wrapper around
/// this for its two single-shot callers (`render`, `tabline_click`);
/// `Editor::sync_tabline_view`'s scroll probe calls this directly instead,
/// reusing one `Vec` across every `scroll` candidate it tries rather than
/// allocating fresh per candidate.
pub(crate) fn tab_extents_into(
    tabs: &[TabEntry],
    scroll: usize,
    x_origin: u16,
    width: u16,
    ranges: &mut Vec<(u16, u16)>,
) -> (bool, bool) {
    let indicator_left = scroll > 0;
    let start_x = x_origin.saturating_add(u16::from(indicator_left));
    let full_limit = x_origin.saturating_add(width);

    // First pass: try every tab from `scroll` onward against the full row
    // width, reserving no column for the trailing `›` — most frames need
    // none, and reserving one unconditionally would cost a visible tab for
    // nothing on every row that already fits.
    if pack_into(tabs, scroll, start_x, full_limit, ranges) {
        return (indicator_left, false);
    }

    // Something didn't fit: reserve one column for `›` and re-pack against
    // the narrower limit, so the arrow can never land inside a tab's own
    // extent and steal its styling (a strictly smaller limit can only pack
    // the same tabs or fewer, never more, so this second pass always leaves
    // `clipped_tail` true).
    let narrow_limit = full_limit.saturating_sub(1).max(start_x);
    pack_into(tabs, scroll, start_x, narrow_limit, ranges);
    if ranges.is_empty() && narrow_limit > start_x {
        // Not even the first visible tab's padded label fits — pack it
        // anyway, clamped to the available width, so the bar is never
        // left empty. `render` truncates the label with `…` to whatever
        // extent it's handed. `scroll < tabs.len()` is implied here: an
        // empty `ranges` from a non-empty `skip(scroll)` is exactly what
        // "didn't fit" means, and an empty `skip(scroll)` would have taken
        // the early `pack_into == true` return above instead.
        ranges.push((start_x, narrow_limit));
    }
    (indicator_left, true)
}

/// Pack as many tabs from `scroll` onward as fit between `start_x` and
/// `limit` into `ranges` (cleared first), returning whether every remaining
/// tab (from `scroll` to the end) fit. The moment one doesn't, packing stops
/// and this returns `false`, but whatever fit before that point is still in
/// `ranges` (the narrow-limit repack in [`tab_extents`] needs that prefix,
/// not just the yes/no).
fn pack_into(
    tabs: &[TabEntry],
    scroll: usize,
    start_x: u16,
    limit: u16,
    ranges: &mut Vec<(u16, u16)>,
) -> bool {
    ranges.clear();
    let mut x = start_x;
    for (i, entry) in tabs.iter().enumerate().skip(scroll) {
        // `" {label} "` — one leading/trailing padding cell, mirroring the
        // statusline's own edge-padding convention (`pad_left`/`pad_right`).
        let label_w = entry.label_width + 2;
        let separator_w = u16::from(i + 1 < tabs.len());
        if x.saturating_add(label_w) > limit {
            return false;
        }
        ranges.push((x, x + label_w));
        x += label_w + separator_w;
    }
    true
}

#[cfg(test)]
mod tests;
