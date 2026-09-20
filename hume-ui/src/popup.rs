//! Cursor-anchored popup widget (`show-popup!`) — a floating text panel used
//! by hover, signature help, and (as a menu) the selection menu / completion
//! menu / minibuffer `:` completion.
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
//!   (frame, scroll window, rows) lives in [`super::menu_box`].
//!
//! [`resolve_popup`]/[`resolve_menu`]/[`resolve_band`] are the composition
//! entry points: each resolves geometry (wrapping + flip + clamp) fresh,
//! every frame, from a [`PopupContent`]/[`MenuRows`] plus a [`PopupPlacement`]
//! (or, for `resolve_band`, the raw band width — a docked popup has no
//! anchor or pane rect). `hume-editor`'s `Editor::sync_popup_view`/
//! `sync_menu_view`/`sync_completion_menu_view`/`sync_minibuf_completion_view`/
//! `sync_popup_band_view` call these against the focused pane's *current*
//! rect — never pre-computed at `show-popup!` time — so a resize or scroll
//! never leaves the result stale.
//! `PopupOverlay`/`PopupBandWidget::render` only paint the already-resolved
//! result, with a final defensive clip against whatever `pane_rect` they're
//! actually given (belt-and-braces: the write side's rect and the render
//! rect are the same frame's geometry, so this should never trigger — see
//! the "never draws outside pane_rect" test).

use hume_engine::types::ResolvedStyle;
use hume_grid::Rect;
use std::sync::Arc;

use hume_engine::lock::SharedSlot;

use hume_engine::providers::{BottomBandProvider, OverlayProvider};
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::theme::ui_scopes;

use super::menu_box::{MenuBoxStyles, draw_menu_box};
use super::width::cell_width;

/// Maximum popup width in terminal columns, before any pane-width clamp.
/// Popup-only — a menu doesn't wrap, so it has no analogous cap here (it
/// uses [`super::menu_box::MAX_MENU_ROWS`], a row cap, instead).
const MAX_POPUP_WIDTH: u16 = 60;

/// Where a popup renders — same widget, same model, two placements.
/// `show-popup!`'s `#:anchor` kwarg selects between them (`hume-editor`'s
/// `PopupLayer::layout`).
pub enum PopupLayout {
    /// Floating, anchored near the focused pane's cursor (`#:anchor
    /// 'cursor`, the default) — painted by `PopupOverlay`.
    Cursor,
    /// Docked as a full-width chrome band directly above the statusline,
    /// reserving pane space like the drawer (`#:anchor 'bottom`) — painted
    /// by `PopupBandWidget`. Used for hover content too tall for the
    /// cursor layout; keeps popup semantics (plain scroll, no selection,
    /// close-on-any-other-key) rather than becoming a pick-list.
    Docked,
}

/// Popup text, wrapped on demand and memoized by width — a pure function of
/// `(source, width)`. Source is fixed at construction (a `show-popup!`
/// builds a fresh `PopupContent`; text/highlights never change during a
/// popup's lifetime), so `width` is the only invalidation key the cache
/// needs; a `:theme` switch does not need to invalidate it either: a
/// `Scrollable` popup closes on any non-scroll key (`Editor::popup_input`)
/// and a `Sticky` one dies with its mode layer as soon as Insert ends
/// (`EditorState::push_mode_layer`'s `clear_popups()` call) before
/// Command-mode input like `:theme` can run — no popup survives to see a
/// stale highlight.
///
/// [`Self::plain`]/[`Self::styled`] both resolve through the same
/// `wrap_styled` call internally — there is one wrap algorithm, not two;
/// `plain` is exactly a single default-style run. `styled` takes
/// pre-resolved `(text, style)` runs rather than a grammar name: this crate
/// has no tree-sitter dependency of its own, so `hume-editor` (which does,
/// via `MarkupSyntax`) resolves spans against its own baked `Theme` before
/// handing them over — `Theme::resolve` debug-asserts on an unbaked
/// `ScopeId`, and this crate has no `ScopeRegistry` access to bake one.
pub struct PopupContent {
    source: Vec<(String, ResolvedStyle)>,
    /// `false` for plain text: [`resolve_popup`]/[`resolve_band`] then leave
    /// `PopupState::styled_rows`/`PopupBandState::styled_rows` `None`, so the
    /// box paints every row in one style instead of walking per-run.
    styled: bool,
    cache: Option<Wrapped>,
}

struct Wrapped {
    width: u16,
    lines: Arc<Vec<String>>,
    /// Widest line's display width — measured once per wrap, not
    /// re-measured every frame an unchanged wrap is reused. See
    /// [`super::menu_box::menu_inner_width`], the same measurement
    /// [`MenuRows`] caches for the same reason.
    inner_width: u16,
    styled_rows: Option<Arc<Vec<StyledRow>>>,
}

impl PopupContent {
    pub fn plain(text: &str) -> Self {
        Self {
            source: vec![(text.to_string(), ResolvedStyle::default())],
            styled: false,
            cache: None,
        }
    }

    /// `runs`: pre-highlighted `(text, style)` spans in source order — see
    /// this type's own doc for why the caller resolves these rather than
    /// handing over a grammar name.
    pub fn styled(runs: Vec<(String, ResolvedStyle)>) -> Self {
        Self {
            source: runs,
            styled: true,
            cache: None,
        }
    }

    /// Wrapped lines + (for [`Self::styled`] content) per-run styles at
    /// `width`, recomputing only when `width` differs from the cached one —
    /// an unchanged width across frames is O(1), not a re-wrap. `lines`/
    /// `styled_rows` are `Arc`-wrapped so writing them into the per-frame
    /// `PopupState`/`PopupBandState` (behind a shared `RwLock`) is a pointer
    /// clone, not a copy of every row.
    fn wrapped(&mut self, width: u16) -> &Wrapped {
        let stale = self.cache.as_ref().is_none_or(|c| c.width != width);
        if stale {
            let rows = wrap_styled(&self.source, width);
            let lines: Vec<String> = rows
                .iter()
                .map(|row| row.iter().map(|(s, _)| s.as_str()).collect())
                .collect();
            let styled_rows = self.styled.then(|| Arc::new(rows));
            let inner_width = super::menu_box::menu_inner_width(&lines);
            self.cache = Some(Wrapped {
                width,
                lines: Arc::new(lines),
                inner_width,
                styled_rows,
            });
        }
        self.cache.as_ref().expect("populated above")
    }
}

/// Fully-resolved popup/menu content and position — computed once per frame
/// by the write side; the overlay only paints.
pub struct PopupState {
    /// Pre-wrapped display lines (word-wrapped to the resolved max width for
    /// a plain popup; one line per item, unwrapped, for a menu). `Arc`-shared
    /// with the source [`PopupContent`]'s cache for a wrapped popup — a
    /// menu's unwrapped labels come from its own [`MenuRows`] instead, never
    /// cached across frames.
    pub lines: Arc<Vec<String>>,
    /// Outer footprint (including the 1-cell frame), top-left corner already
    /// flipped/clamped by the write side. Each caller's row cap differs
    /// (hover: ⅓ pane height; menus/LSP completion: `MAX_MENU_ROWS` with a
    /// scroll window) — carrying the resolved size here, rather than
    /// re-deriving it from `lines.len()`
    /// at render time, keeps the painted box and the positioned box the
    /// same box.
    pub rect: Rect,
    /// The highlighted row index, for menus. `None` for a plain popup.
    pub selected: Option<usize>,
    /// First visible row, for a plain popup (`selected.is_none()`) — the
    /// resolved counterpart of `PopupLayer::scroll`. Ignored by menus, which
    /// window around `selected` instead.
    pub scroll: usize,
    /// Whether to draw box-drawing border glyphs around the popup (vs. a
    /// plain background-filled 1-cell margin). Fed from the `popup-border`
    /// setting.
    pub border: bool,
    /// Per-run styled counterpart of `lines`, same length and same text
    /// when flattened — `Some` only for a [`PopupContent::styled`] popup.
    /// `None` for every other popup and for menus, which paint `lines` in
    /// one style regardless.
    pub styled_rows: Option<Arc<Vec<StyledRow>>>,
}

/// Generic overlay that paints a `PopupState` snapshot. Used directly for
/// hover-style popups (`show-popup!`) and, via a second registration with
/// its own `Arc`, for the selection menu and completion menu.
pub(crate) struct PopupOverlay {
    pub(crate) data: SharedSlot<Option<PopupState>>,
    /// Root scope for the background/text fill (`ui.popup` for hover popups,
    /// `ui.menu` for menus) — `MenuBoxStyles::resolve` derives the
    /// highlighted-row and scrollbar-thumb styles from it.
    pub(crate) scope: &'static str,
}

impl OverlayProvider for PopupOverlay {
    fn is_active(&self) -> bool {
        self.data.read().is_some()
    }

    fn render(&self, pane_rect: Rect, theme: &Theme, canvas: &mut Canvas) {
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };
        if state.lines.is_empty() {
            return;
        }

        // Defensive clip: the write side computed this rect against this
        // same pane's rect this same frame, so this should never trigger —
        // see `fits_inside`'s doc.
        if !super::menu_box::fits_inside(state.rect, pane_rect) {
            return;
        }
        draw_menu_box(
            canvas,
            state.rect,
            &state.lines,
            state.selected,
            state.scroll,
            state.border,
            MenuBoxStyles::resolve(theme, self.scope),
            state.styled_rows.as_ref().map(|rows| rows.as_slice()),
        );
    }
}

/// Fully-resolved content for a **docked** popup (`PopupLayout::Docked`) —
/// the `PopupBandWidget` counterpart of [`PopupState`]. No position/size
/// is stored here: unlike the floating popup, a bottom band's geometry is
/// resolved by the engine at render time from `height(max)` and the chrome
/// area (the same contract the drawer already follows), not pre-computed by
/// the write side.
pub struct PopupBandState {
    /// Word-wrapped to the band's width (the write side, `Editor::
    /// sync_popup_band_view`, wraps against `last_terminal_area` — the same
    /// raw area the engine will render the band into).
    pub lines: Arc<Vec<String>>,
    pub scroll: usize,
    pub border: bool,
    pub styled_rows: Option<Arc<Vec<StyledRow>>>,
}

/// Engine-facing bottom-band provider for a docked popup — mirrors
/// [`super::drawer::DrawerWidget`]'s shape (chrome, not per-pane), but paints
/// through `draw_menu_box` so a docked hover keeps the popup's framed,
/// `ui.popup`-scoped look rather than the drawer's plain list rows.
pub(crate) struct PopupBandWidget {
    pub(crate) data: SharedSlot<Option<PopupBandState>>,
}

/// The frame's top/bottom cells — always reserved, even with `popup-border`
/// off (a plain background margin still takes the row, see `draw_menu_box`'s
/// doc on `border`).
const POPUP_FRAME_ROWS: u16 = 2;

/// Rows a docked popup shows at once, given `lines` wrapped lines and the
/// band's row ceiling `max` (35% of the last-rendered *terminal* height,
/// mirroring `PopupBandWidget::height`'s own `max`) — the number
/// `Editor::scroll_popup` pages against, agreeing with what the engine will
/// next paint by construction (both derive from
/// `super::menu_box::band_capacity`).
pub fn band_visible_rows(lines: usize, max: u16) -> usize {
    super::menu_box::band_visible_rows(lines, POPUP_FRAME_ROWS, max)
}

impl BottomBandProvider for PopupBandWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read();
        guard.as_ref().map_or(0, |s| {
            super::menu_box::band_capacity(s.lines.len(), POPUP_FRAME_ROWS, max)
        })
    }

    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };
        draw_menu_box(
            canvas,
            area,
            &state.lines,
            None,
            state.scroll,
            state.border,
            MenuBoxStyles::resolve(theme, ui_scopes::POPUP),
            state.styled_rows.as_ref().map(|rows| rows.as_slice()),
        );
    }
}

/// Where a cursor-anchored box sits: the anchor cell (absolute screen
/// coords), the pane it must stay inside, and that pane's text-column
/// budget. Three facts about the cursor — no per-widget size cap here: a
/// popup wraps to ⅓ pane height and `MAX_POPUP_WIDTH`, a menu doesn't wrap
/// at all and uses `MAX_MENU_ROWS` instead, so folding either onto this
/// shared value would leave the other caller discarding it (as the old
/// combined 4-tuple this replaced did for its two menu-shaped callers).
#[derive(Clone, Copy)]
pub struct PopupPlacement {
    pub anchor: (u16, u16),
    pub pane_rect: Rect,
    pub content_width: u16,
}

/// Resolve a cursor-anchored popup's content + position for this frame.
/// `scroll` is the model's raw scroll value; the result clamps it to the
/// resolved content height.
pub fn resolve_popup(
    content: &mut PopupContent,
    placement: PopupPlacement,
    scroll: usize,
    border: bool,
) -> PopupState {
    // Reserve 2 cells on each axis for the popup's 1-cell frame, so
    // content + border together fit the same envelope this budget used to
    // give to content alone.
    let max_width = MAX_POPUP_WIDTH
        .min(placement.content_width.saturating_sub(4))
        .saturating_sub(2);
    let max_height = (placement.pane_rect.height / 3)
        .max(1)
        .saturating_sub(2)
        .max(1);

    let wrapped = content.wrapped(max_width);
    let (outer_w, outer_h) = super::menu_box::outer_dims_from_width(
        wrapped.inner_width,
        wrapped.lines.len(),
        max_height,
    );
    let (x, y, outer_w, outer_h) =
        resolve_popup_geometry(outer_w, outer_h, placement.anchor, placement.pane_rect);
    let inner_h = outer_h.saturating_sub(2) as usize;
    let scroll = scroll.min(wrapped.lines.len().saturating_sub(inner_h));
    PopupState {
        lines: Arc::clone(&wrapped.lines),
        rect: Rect::new(x, y, outer_w, outer_h),
        selected: None,
        scroll,
        styled_rows: wrapped.styled_rows.clone(),
        border,
    }
}

/// Resolve a docked popup's content for this frame — the [`PopupLayout::
/// Docked`] counterpart of [`resolve_popup`]. No anchor or pane rect: unlike
/// the floating popup, a bottom band's position/height is resolved by the
/// engine at render time from `height(max)` and the chrome area (mirroring
/// the drawer's own contract), so this only resolves content. `area_width`
/// is the raw, un-subtracted terminal width (the band spans the full
/// terminal, not just the panes region); `max_rows` bounds how many rows the
/// band may claim (mirrors `PopupBandWidget::height`'s own `max`).
pub fn resolve_band(
    content: &mut PopupContent,
    area_width: u16,
    max_rows: u16,
    scroll: usize,
    border: bool,
) -> PopupBandState {
    let max_width = area_width.saturating_sub(2);
    let wrapped = content.wrapped(max_width);
    let inner_h = band_visible_rows(wrapped.lines.len(), max_rows);
    let scroll = scroll.min(wrapped.lines.len().saturating_sub(inner_h));
    PopupBandState {
        lines: Arc::clone(&wrapped.lines),
        scroll,
        styled_rows: wrapped.styled_rows.clone(),
        border,
    }
}

/// Menu rows plus the width they measure to — carries its own measurement
/// so a caller with a cache (`CompletionSession::menu_cache`) reuses it
/// instead of re-measuring every candidate every frame, and so
/// [`resolve_menu`] can never be handed a width its rows don't actually
/// have.
#[derive(Clone)]
pub struct MenuRows {
    labels: Arc<Vec<String>>,
    inner_width: u16,
}

impl MenuRows {
    pub fn measure(labels: Arc<Vec<String>>) -> Self {
        let inner_width = super::menu_box::menu_inner_width(&labels);
        Self {
            labels,
            inner_width,
        }
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn labels(&self) -> &Arc<Vec<String>> {
        &self.labels
    }

    pub fn inner_width(&self) -> u16 {
        self.inner_width
    }
}

/// Resolve a menu's (selection menu or LSP completion menu) content +
/// position for this frame. No wrapping — menu entries are short labels,
/// not prose — so, unlike [`resolve_popup`], there is no width budget to
/// compute: `MAX_MENU_ROWS` alone bounds the box.
pub fn resolve_menu(
    rows: MenuRows,
    selected: usize,
    placement: PopupPlacement,
    border: bool,
) -> PopupState {
    let (outer_w, outer_h) = super::menu_box::outer_dims_from_width(
        rows.inner_width(),
        rows.len(),
        super::menu_box::MAX_MENU_ROWS,
    );
    let (x, y, outer_w, outer_h) =
        resolve_popup_geometry(outer_w, outer_h, placement.anchor, placement.pane_rect);
    let selected = if rows.is_empty() {
        None
    } else {
        Some(selected.min(rows.len() - 1))
    };
    PopupState {
        lines: rows.labels,
        rect: Rect::new(x, y, outer_w, outer_h),
        selected,
        scroll: 0,         // ignored: a menu windows around `selected`, not `scroll`
        styled_rows: None, // menus never highlight per-span, only per-row
        border,
    }
}

/// Resolve the top-left corner and clamped size for a `width` × `height` box
/// (the outer footprint, including any frame) anchored near `anchor`
/// (cursor cell, absolute screen coords) within `pane_rect`. Shared by
/// [`resolve_popup`]/[`resolve_menu`] — callers pass their content size plus
/// the 2-cell frame reserved for the border.
///
/// `width`/`height` are clamped to `pane_rect`'s size before the position is
/// resolved, so the returned box always fits inside the pane — callers must
/// use the returned size, not their original request, when painting.
/// `PopupOverlay`'s bounds check is a defensive backstop, not a substitute
/// for this: without the clamp, a box wider or taller than the pane can
/// never satisfy that check, and the whole popup silently fails to render.
fn resolve_popup_geometry(
    width: u16,
    height: u16,
    anchor: (u16, u16),
    pane_rect: Rect,
) -> (u16, u16, u16, u16) {
    let (width, height) = clamp_size_to_pane(width, height, pane_rect);
    let (anchor_x, anchor_y) = anchor;
    let space_below = pane_rect.bottom().saturating_sub(anchor_y + 1);
    let space_above = anchor_y.saturating_sub(pane_rect.y);

    let y = if space_below >= height || space_below >= space_above {
        anchor_y + 1
    } else {
        anchor_y.saturating_sub(height)
    };
    let y = y
        .max(pane_rect.y)
        .min(pane_rect.bottom().saturating_sub(height));

    let x = clamp_x_to_pane(anchor_x, width, pane_rect);

    (x, y, width, height)
}

/// Clamp `width`×`height` to fit inside `pane_rect` — the size half of a
/// box-in-pane placement, used by [`resolve_popup_geometry`] before it
/// resolves a position.
fn clamp_size_to_pane(width: u16, height: u16, pane_rect: Rect) -> (u16, u16) {
    (width.min(pane_rect.width), height.min(pane_rect.height))
}

/// Clamp `x` so a `width`-wide box starting there never crosses `pane_rect`'s
/// left or right edge — the horizontal-position half of a box-in-pane
/// placement, split out from [`resolve_popup_geometry`] so its vertical
/// counterpart isn't tangled up with this axis.
fn clamp_x_to_pane(x: u16, width: u16, pane_rect: Rect) -> u16 {
    x.max(pane_rect.x)
        .min(pane_rect.right().saturating_sub(width))
}

/// One wrapped popup row's content, as contiguous same-style runs — the
/// styled counterpart of a plain-wrapped row (a `Vec<StyledRun>` instead of
/// a bare `String`).
pub(in crate::popup) type StyledRun = (String, ResolvedStyle);
pub type StyledRow = Vec<StyledRun>;

/// Merge adjacent `(text, style)` pairs sharing the same `ResolvedStyle` —
/// [`coalesce_atoms`]'s own primitive, merging wrapped *output* graphemes
/// back down to runs.
fn push_run(runs: &mut Vec<(String, ResolvedStyle)>, text: &str, style: ResolvedStyle) {
    if text.is_empty() {
        return;
    }
    match runs.last_mut() {
        Some((last_text, last_style)) if *last_style == style => last_text.push_str(text),
        _ => runs.push((text.to_string(), style)),
    }
}

/// Merge adjacent atoms sharing the same `ResolvedStyle` into `StyledRun`s.
fn coalesce_atoms(atoms: Vec<(&str, ResolvedStyle)>) -> StyledRow {
    let mut out: StyledRow = Vec::new();
    for (g, style) in atoms {
        push_run(&mut out, g, style);
    }
    out
}

/// Word-wrap `runs` (contiguous same-style chunks of source text — `\n` acts
/// as a paragraph delimiter, and may appear anywhere inside a run) to
/// `max_width` display columns, breaking on grapheme-cluster boundaries.
/// Unbounded height — [`PopupContent::wrapped`]'s caller windows the result
/// (`scroll`) rather than truncating it here; a `Scrollable` popup needs
/// every row reachable, not just the first screenful.
///
/// Operates on a flat per-grapheme stream, never on `runs`' original chunk
/// boundaries — a style change (e.g. a `**bold**` span) can land anywhere,
/// including mid-word, so wrapping must not coarsen past grapheme
/// granularity. Plain popup text is exactly a single default-style run —
/// there is one wrap algorithm here, not a separate one for plain text.
fn wrap_styled(runs: &[(String, ResolvedStyle)], max_width: u16) -> Vec<StyledRow> {
    use unicode_segmentation::UnicodeSegmentation;

    let atoms: Vec<(&str, ResolvedStyle)> = runs
        .iter()
        .flat_map(|(text, style)| text.graphemes(true).map(move |g| (g, *style)))
        .collect();

    let max_width = max_width.max(1) as usize;
    let mut out: Vec<StyledRow> = Vec::new();
    let mut pos = 0;

    loop {
        let para_start = pos;
        while pos < atoms.len() && atoms[pos].0 != "\n" {
            pos += 1;
        }
        let paragraph = &atoms[para_start..pos];
        let had_newline = pos < atoms.len();
        if had_newline {
            pos += 1; // skip the "\n" atom itself
        }

        if paragraph.is_empty() {
            out.push(Vec::new());
        } else {
            let mut current: Vec<(&str, ResolvedStyle)> = Vec::new();
            let mut current_w = 0usize;
            let mut word_start = 0;
            loop {
                let mut word_end = word_start;
                while word_end < paragraph.len() && paragraph[word_end].0 != " " {
                    word_end += 1;
                }
                let word = &paragraph[word_start..word_end];
                let word_w: usize = word.iter().map(|(g, _)| cell_width(g)).sum();
                // Would-be width if `word` were appended to the current
                // line — recomputed fresh each iteration (never carried
                // across a break) so a line-break never leaves a stale
                // separator width behind.
                let would_be_w = if current.is_empty() {
                    word_w
                } else {
                    current_w + 1 + word_w
                };

                if would_be_w > max_width && !current.is_empty() {
                    out.push(coalesce_atoms(std::mem::take(&mut current)));
                    current_w = 0;
                }

                if word_w > max_width {
                    // A single word wider than the line — hard-break it on
                    // grapheme boundaries rather than overflow.
                    if !current.is_empty() {
                        out.push(coalesce_atoms(std::mem::take(&mut current)));
                    }
                    let mut piece: Vec<(&str, ResolvedStyle)> = Vec::new();
                    let mut piece_w = 0usize;
                    for &(g, style) in word {
                        let gw = cell_width(g);
                        if piece_w + gw > max_width && !piece.is_empty() {
                            out.push(coalesce_atoms(std::mem::take(&mut piece)));
                            piece_w = 0;
                        }
                        piece.push((g, style));
                        piece_w += gw;
                    }
                    current = piece;
                    current_w = piece_w;
                } else {
                    if !current.is_empty() {
                        // The synthetic separator carries the *next* word's
                        // style — it's a single blank cell either way, this
                        // just keeps it from spuriously splitting an
                        // otherwise-uniform run in two.
                        let sep_style = word
                            .first()
                            .map_or_else(ResolvedStyle::default, |&(_, s)| s);
                        current.push((" ", sep_style));
                        current_w += 1;
                    }
                    current.extend_from_slice(word);
                    current_w += word_w;
                }

                if word_end >= paragraph.len() {
                    break;
                }
                word_start = word_end + 1; // skip the delimiting space atom
            }
            out.push(coalesce_atoms(current));
        }

        if !had_newline {
            break;
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
