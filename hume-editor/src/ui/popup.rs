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

use crate::lock_ext::LockExt;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;

use hume_engine::providers::{BottomBandProvider, OverlayProvider, SyntaxSpans};
use hume_engine::theme::Theme;
use hume_engine::types::Scope;
use hume_scripting::host::PopupKind;

use super::menu_box::{MenuBoxStyles, draw_menu_box};
use super::width::cell_width;

/// Maximum popup width in terminal columns, before any pane-width clamp.
pub(crate) const MAX_POPUP_WIDTH: u16 = 60;

/// Synchronously-parsed highlight state for a popup's read-only text, keyed
/// by grammar name (`#:lang`) — built once at `show-popup!` time (there is
/// nothing to incrementally reparse: the content never changes after this).
/// `None` where `PopupModel::syntax` would go when no grammar by that name
/// is registered, or `#:lang` wasn't requested — the plain-text fallback.
///
/// Shared by the cursor and docked popup layouts so highlight resolution
/// (`styled_row`/`styled_runs`) has one implementation — only
/// wrapping/geometry differs between the two.
pub(crate) struct MarkupSyntax {
    pub(crate) syntax: hume_treesitter::syntax::Syntax,
    /// Same content the syntax was parsed from, wrapped as a rope-backed
    /// `Text` — `Syntax::spans_for_line` needs `&Rope`, and re-deriving one
    /// from the source string every frame would re-walk it on every render.
    pub(crate) text: hume_editing::text::Text,
}

impl MarkupSyntax {
    /// Highlight spans for one source line, resolved into contiguous
    /// same-style runs — a run per contiguous same-scope span, with gaps
    /// between spans (and a line with no spans at all) getting `base_style`.
    ///
    /// `line` is the caller's own text for `line_idx` (not re-sliced from
    /// `self.text`'s rope) — byte offsets from `spans_for_line` are relative
    /// to the line start either way, so this stays exact even when the
    /// caller's line boundaries differ slightly from the rope's own (see
    /// `styled_runs`'s doc on `self.text`'s padded trailing `'\n'`).
    pub(crate) fn styled_row(
        &self,
        line_idx: usize,
        line: &str,
        theme: &Theme,
        base_style: Style,
    ) -> StyledRow {
        let mut spans = Vec::new();
        self.syntax
            .spans_for_line(line_idx, self.text.rope(), &mut spans);

        let mut row: StyledRow = Vec::new();
        let mut cursor = 0usize;
        for &(start, end, scope) in &spans {
            if start > cursor {
                push_run(&mut row, &line[cursor..start], base_style);
            }
            push_run(&mut row, &line[start..end], theme.resolve(scope).into());
            cursor = end;
        }
        if cursor < line.len() {
            push_run(&mut row, &line[cursor..], base_style);
        }
        row
    }

    /// Resolve every line of `text` into one flat run sequence for
    /// [`wrap_styled`] — lines joined by a bare `"\n"` run so its paragraph
    /// splitting sees the exact same boundaries a plain popup would.
    ///
    /// Slices `text`'s own paragraphs (`text.split('\n')`), not `self.text`'s
    /// rope lines — `self.text` is `Text::from(text)`, which may have padded
    /// on a trailing `'\n'` `text` itself lacked (the buffer invariant), and
    /// iterating the padded rope would emit a spurious trailing empty row
    /// `wrap_text` on plain `text` never would.
    pub(crate) fn styled_runs(
        &self,
        text: &str,
        theme: &Theme,
        base_style: Style,
    ) -> Vec<(String, Style)> {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut runs: Vec<(String, Style)> = Vec::new();
        for (line_idx, line) in lines.iter().enumerate() {
            runs.extend(self.styled_row(line_idx, line, theme, base_style));
            if line_idx + 1 < lines.len() {
                push_run(&mut runs, "\n", base_style);
            }
        }
        runs
    }
}

/// Where a popup renders — same widget, same model, two placements.
/// `show-popup!`'s `#:anchor` kwarg selects between them.
pub(crate) enum PopupLayout {
    /// Floating, anchored near the focused pane's cursor (`#:anchor
    /// 'cursor`, the default) — painted by [`PopupOverlay`].
    Cursor,
    /// Docked as a full-width chrome band directly above the statusline,
    /// reserving pane space like the drawer (`#:anchor 'bottom`) — painted
    /// by [`PopupBandWidget`]. Used for hover content too tall for the
    /// cursor layout; keeps popup semantics (plain scroll, no selection,
    /// close-on-any-other-key) rather than becoming a pick-list.
    Docked,
}

/// `(show-popup! text)`'s raw, unwrapped content — held on `EditorState`
/// until the next frame's `sync_popup_view`/`sync_popup_band_view` (per
/// `layout`) resolves it into a positioned [`PopupState`] or a
/// [`PopupBandState`].
pub(crate) struct PopupModel {
    pub(crate) text: String,
    pub(crate) kind: PopupKind,
    /// First visible wrapped row, for a `Scrollable` popup. Clamped against
    /// `max_scroll` in `Editor::scroll_popup` before each delta is applied,
    /// since content height (and so `max_scroll`) can shrink between key
    /// presses (e.g. the terminal grows) without this field being touched.
    /// `Editor::sync_popup_view`/`sync_popup_band_view` additionally clamp a
    /// *copy* of this value for rendering each frame — that clamp is
    /// view-only and never writes back to the model.
    pub(crate) scroll: usize,
    /// `#:lang` — rebuilt fresh on every `show-popup!`, dropped with the
    /// popup on close. No separate invalidation path: the popup's lifetime
    /// IS the syntax's (SSOT).
    pub(crate) syntax: Option<MarkupSyntax>,
    pub(crate) layout: PopupLayout,
    /// Cached wrap+highlight, keyed by `max_width` — see [`ResolvedPopupText`].
    /// `None` until the first `sync_popup_view`/`sync_popup_band_view` call.
    pub(crate) resolved: Option<ResolvedPopupText>,
}

/// Wrap+highlight resolved for one `max_width` — a pure function of
/// `(text, syntax, width)`. `Editor::resolve_popup_text` recomputes only when
/// `width` differs from the cached one; `text`/`syntax` never change during a
/// model's lifetime (a new `show-popup!` builds a fresh `PopupModel`), so
/// `width` is the only invalidation key needed. Mirrors `MarkupSyntax`'s own
/// build-once discipline (see its doc).
///
/// A `:theme` switch does not need to invalidate this: a `Scrollable` popup
/// closes on any key (`Editor::handle_key`'s top-of-loop check) and a
/// `Sticky` one closes on `on-mode-change` (`register-hook! 'on-mode-change
/// close-popup!` in `core:lsp/lib.scm`) before Command-mode input like
/// `:theme` can run — no popup survives to see a stale highlight.
///
/// `lines`/`styled_rows` are `Arc`-wrapped so writing them into the
/// per-frame `PopupState`/`PopupBandState` (behind a shared `RwLock`) is a
/// pointer clone, not a copy of every row.
#[derive(Clone)]
pub(crate) struct ResolvedPopupText {
    pub(crate) width: u16,
    pub(crate) lines: Arc<Vec<String>>,
    pub(crate) styled_rows: Option<Arc<Vec<StyledRow>>>,
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
    /// a plain popup; one line per item, unwrapped, for a menu). `Arc`-shared
    /// with `PopupModel::resolved` for a wrapped popup (see
    /// [`ResolvedPopupText`]) — a menu's unwrapped labels are freshly
    /// `Arc::new`-wrapped each frame instead, since they're never cached.
    pub(crate) lines: Arc<Vec<String>>,
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
    /// First visible row, for a plain popup (`selected.is_none()`) — the
    /// resolved counterpart of `PopupModel::scroll`. Ignored by menus, which
    /// window around `selected` instead.
    pub(crate) scroll: usize,
    /// Whether to draw box-drawing border glyphs around the popup (vs. a
    /// plain background-filled 1-cell margin). Fed from the `popup-border`
    /// setting.
    pub(crate) border: bool,
    /// Per-run styled counterpart of `lines`, same length and same text
    /// when flattened — `Some` only for a popup with `#:lang` set to a
    /// registered grammar (`PopupModel::syntax`). `None` for every other
    /// popup and for menus, which paint `lines` in one style regardless.
    pub(crate) styled_rows: Option<Arc<Vec<StyledRow>>>,
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
    /// Scope for the scrollbar thumb (`ui.popup.scroll` / `ui.menu.scroll`).
    /// Falls back to `scope`'s own style via the theme's dot-notation chain
    /// when a theme doesn't define it — see `Theme::resolve_raw`. Unlike the
    /// statusline separator (`EditorColors::from_theme`, `ui/theme.rs`), this
    /// fallback target is never mode-tinted, so an absent scope only degrades
    /// the thumb to the border's own color rather than painting the wrong
    /// background — a plain dot-fallback is safe here.
    pub(crate) scroll_scope: &'static str,
}

impl OverlayProvider for PopupOverlay {
    fn is_active(&self) -> bool {
        self.data.read_or_panic().is_some()
    }

    fn render(&self, pane_rect: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read_or_panic();
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
        let scroll_style = theme.resolve_by_name(Scope(self.scroll_scope)).into();
        draw_menu_box(
            buf,
            outer,
            &state.lines,
            state.selected,
            state.scroll,
            state.border,
            MenuBoxStyles {
                base: style,
                selected: selected_style,
                scroll: scroll_style,
            },
            state.styled_rows.as_ref().map(|rows| rows.as_slice()),
            theme.ui.invisible.into(),
        );
    }
}

/// Fully-resolved content for a **docked** popup (`PopupLayout::Docked`) —
/// the [`PopupBandWidget`] counterpart of [`PopupState`]. No position/size
/// is stored here: unlike the floating popup, a bottom band's geometry is
/// resolved by the engine at render time from `height(max)` and the chrome
/// area (the same contract the drawer already follows), not pre-computed by
/// the write side.
pub(crate) struct PopupBandState {
    /// Word-wrapped to the band's width (the write side, `Editor::
    /// sync_popup_band_view`, wraps against `last_terminal_area` — the same
    /// raw area the engine will render the band into). `Arc`-shared with
    /// `PopupModel::resolved` — see [`ResolvedPopupText`].
    pub(crate) lines: Arc<Vec<String>>,
    pub(crate) scroll: usize,
    pub(crate) border: bool,
    pub(crate) styled_rows: Option<Arc<Vec<StyledRow>>>,
}

/// Engine-facing bottom-band provider for a docked popup — mirrors
/// `ui::drawer::DrawerWidget`'s shape (chrome, not per-pane), but paints
/// through [`draw_menu_box`] so a docked hover keeps the popup's framed,
/// `ui.popup`-scoped look rather than the drawer's plain list rows.
pub(crate) struct PopupBandWidget {
    pub(crate) data: Arc<RwLock<Option<PopupBandState>>>,
}

/// Outer row count for a docked popup band holding `lines` content rows,
/// capped at `max` — the single source of truth for this arithmetic, shared
/// by [`PopupBandWidget::height`] (what the engine paints against),
/// `Editor::sync_popup_band_view`'s scroll clamp, and
/// `Editor::popup_band_visible_rows` (what `scroll_popup` pages against).
/// Kept in one place so the painted band and the scroll clamp can never
/// silently disagree.
///
/// `+2` reserves the frame's top/bottom cells — always reserved, even with
/// `popup-border` off (a plain background margin still takes the row, see
/// `draw_menu_box`'s doc on `border`).
pub(crate) fn band_capacity(lines: usize, max: u16) -> u16 {
    (lines as u16 + 2).min(max)
}

impl BottomBandProvider for PopupBandWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read_or_panic();
        guard
            .as_ref()
            .map_or(0, |s| band_capacity(s.lines.len(), max))
    }

    fn render(&self, area: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read_or_panic();
        let Some(state) = guard.as_ref() else { return };
        let style = theme.resolve_by_name(Scope("ui.popup")).into();
        let scroll_style = theme.resolve_by_name(Scope("ui.popup.scroll")).into();
        draw_menu_box(
            buf,
            area,
            &state.lines,
            None,
            state.scroll,
            state.border,
            MenuBoxStyles {
                base: style,
                selected: style,
                scroll: scroll_style,
            },
            state.styled_rows.as_ref().map(|rows| rows.as_slice()),
            theme.ui.invisible.into(),
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

/// One wrapped display row's content, as contiguous same-style runs — the
/// styled counterpart of a `wrap_text` row (a `Vec<StyledRun>` instead of a
/// bare `String`).
pub(crate) type StyledRun = (String, Style);
pub(crate) type StyledRow = Vec<StyledRun>;

/// Merge adjacent `(text, style)` pairs sharing the same `Style` — shared by
/// [`MarkupSyntax::styled_row`] (building the *input* runs `wrap_styled`
/// wraps) and [`coalesce_atoms`] (merging wrapped *output* graphemes back
/// down); same "adjacent equal style" rule, different element granularity
/// (whole strings here, single graphemes there).
fn push_run(runs: &mut Vec<(String, Style)>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    match runs.last_mut() {
        Some((last_text, last_style)) if *last_style == style => last_text.push_str(text),
        _ => runs.push((text.to_string(), style)),
    }
}

/// Word-wrap `text` (newline-separated paragraphs preserved) to `max_width`
/// display columns, breaking on grapheme-cluster boundaries. Unbounded height
/// — the caller windows the result (`scroll`) rather than truncating it here.
///
/// A single-style call to [`wrap_styled`] — there is one wrap algorithm, not
/// two; this function's own test suite exercises it transitively, and a
/// styled popup (markdown-highlighted hover) reuses the exact same
/// word/hard-break decisions a plain popup would have made.
pub(crate) fn wrap_text(text: &str, max_width: u16) -> Vec<String> {
    let runs = [(text.to_string(), Style::default())];
    wrap_styled(&runs, max_width)
        .into_iter()
        .map(|row| row.into_iter().map(|(s, _)| s).collect())
        .collect()
}

/// Merge adjacent atoms sharing the same `Style` into `StyledRun`s.
fn coalesce_atoms(atoms: Vec<(&str, Style)>) -> StyledRow {
    let mut out: StyledRow = Vec::new();
    for (g, style) in atoms {
        push_run(&mut out, g, style);
    }
    out
}

/// Word-wrap `runs` (contiguous same-style chunks of source text — `\n` acts
/// as a paragraph delimiter exactly as in `wrap_text`, and may appear
/// anywhere inside a run) to `max_width` display columns, breaking on
/// grapheme-cluster boundaries. Unbounded height — every caller windows the
/// result (`scroll`) rather than truncating it here; a `Scrollable` popup
/// needs every row reachable, not just the first screenful.
///
/// Operates on a flat per-grapheme stream, never on `runs`' original chunk
/// boundaries — a style change (e.g. a `**bold**` span) can land anywhere,
/// including mid-word, so wrapping must not coarsen past grapheme
/// granularity. Word/paragraph splitting and the width math are otherwise
/// identical to the original single-style algorithm.
pub(crate) fn wrap_styled(runs: &[(String, Style)], max_width: u16) -> Vec<StyledRow> {
    use unicode_segmentation::UnicodeSegmentation;

    let atoms: Vec<(&str, Style)> = runs
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
            let mut current: Vec<(&str, Style)> = Vec::new();
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
                    let mut piece: Vec<(&str, Style)> = Vec::new();
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
                        let sep_style = word.first().map_or_else(Style::default, |&(_, s)| s);
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
