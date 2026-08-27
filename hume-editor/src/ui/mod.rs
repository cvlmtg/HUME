pub(crate) mod completion_overlay;
pub(crate) mod confirm;
pub(crate) mod drawer;
pub(crate) mod highlight_providers;
pub(crate) mod inline_decorations;
pub(crate) mod line_backgrounds;
pub(crate) mod menu_box;
pub(crate) mod picker_panel;
pub(crate) mod popup;
pub(crate) mod signs;
pub mod statusline;
pub(crate) mod theme;
pub(crate) mod virtual_lines;
pub(crate) mod width;

use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use hume_engine::builtins::line_number::LineNumberColumn;
use hume_engine::builtins::sign_column::SignColumn;
use hume_engine::pane::Pane;
use hume_engine::pipeline::BufferId;
use hume_engine::providers::{DEFAULT_GUTTER_SCOPE, HighlightTier, ProviderSet};
use hume_engine::theme::ScopeRegistry;

use completion_overlay::MinibufCompletionOverlay;
use highlight_providers::{PaneHighlights, ScopedHighlighter};
use inline_decorations::{InlineDecorationMap, InlineDecorationProvider};
use line_backgrounds::{LineBgMap, PaneLineBackgrounds};
use picker_panel::PickerOverlay;
use popup::PopupOverlay;
use signs::{PaneSigns, SharedSignSource};
use virtual_lines::{PaneVirtualLines, VirtualLineMap};

/// A pane's six render-decoration handles, allocated together by
/// [`build_pane`] and stored as one `SecondaryMap` entry on
/// `EditorState.panes.render` — they are always seeded and dropped as a
/// unit (never independently), and every read site borrows the map
/// shared, so bundling them costs nothing and removes the "added a new
/// per-pane provider, forgot to drop it in `drop_pane_state`" bug class.
pub(crate) struct PaneRenderHandles {
    pub(crate) highlights: PaneHighlights,
    pub(crate) signs: PaneSigns,
    pub(crate) inlay_hints: InlineDecorationMap,
    pub(crate) virtual_lines: VirtualLineMap,
    /// EOL text (the diagnostics plugin's per-line summary is its first
    /// client) — a second `InlineDecorationProvider` instance (same
    /// INLINE-kind `DecorationSource` shape, distinct Arc/`ProviderId`) fed
    /// by `decorations.eol_text` instead of `inlay_hints`, so the two
    /// coexist on the same line without one clobbering the other.
    pub(crate) eol_text: InlineDecorationMap,
    pub(crate) line_backgrounds: LineBgMap,
}

/// Build a new pane viewing `buffer_id`: sign column, line-number gutter,
/// bracket-match/search-match/diagnostic/extra-highlight sources, inlay-hint
/// decoration, virtual-line source, line-background tint, and
/// completion/hover/selection-menu/LSP overlays. Wrap mode is not seeded
/// here — the new pane starts with no override for any buffer (`Pane::new`'s
/// empty `wraps` map) and resolves it lazily on every read
/// (`commands::effective_wrap_mode`).
///
/// Returns the pane with its freshly-allocated [`PaneRenderHandles`] — every
/// pane gets its own buffers (never shared), so each pane's decorations come
/// from that pane's own buffer and viewport. The caller stores them in
/// `EditorState.panes.render` keyed by the new pane's id.
///
/// The gutter column is added with its default style; `prepare_frame` syncs
/// the buffer-resolved `line-number-style` into every pane's gutter before
/// each render (see `sync_line_number_style`), so the seeded style never
/// reaches a frame. Bracket-match/search-match scopes are not interned
/// here — unlike `linenr_scope` (shared with the gutter columns built right
/// below), they have no other constructor needing them this frame, so they
/// resolve lazily on first render the same way `Editor::diagnostic_scopes`/
/// `inlay_hint_scope` already do (`decoration_providers.rs`).
///
/// Single source of truth for pane construction — every creation site
/// (`Editor::open`'s bootstrap pane, `commands::open_pane`) goes through
/// this, so panes render identically. `Pane::new` alone has an empty
/// `ProviderSet` (no gutter column).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pane(
    registry: &mut ScopeRegistry,
    minibuf_completion_view: &Arc<RwLock<Option<completion_overlay::MinibufCompletionView>>>,
    popup_view: &Arc<RwLock<Option<popup::PopupState>>>,
    menu_view: &Arc<RwLock<Option<popup::PopupState>>>,
    completion_menu_view: &Arc<RwLock<Option<popup::PopupState>>>,
    picker_view: &Arc<RwLock<Option<picker_panel::PickerViewState>>>,
    buffer_id: BufferId,
) -> (Pane, PaneRenderHandles) {
    // Interns the engine's own `DEFAULT_GUTTER_SCOPE` constant rather than
    // repeating the "ui.linenr" literal here — the two must resolve to the
    // same scope: `compose_gutter`'s own fallback
    // (`EngineView::default_gutter_scope`) interns that same constant, and a
    // blank sign slot / line-number cell rendering under a different
    // `ScopeId` than the row-fill fallback would silently disagree on
    // styling.
    let linenr_scope = registry.intern(DEFAULT_GUTTER_SCOPE.0);
    let linenr_selected_scope = registry.intern("ui.linenr.selected");

    let highlights = PaneHighlights::default();
    let signs = PaneSigns::default();
    let inlay_hint_map: InlineDecorationMap = Arc::new(RwLock::new(FxHashMap::default()));
    let eol_text_map: InlineDecorationMap = Arc::new(RwLock::new(FxHashMap::default()));
    let virtual_line_map: VirtualLineMap = Arc::new(RwLock::new(FxHashMap::default()));
    let line_bg_map: LineBgMap = Arc::new(RwLock::new(FxHashMap::default()));

    let mut providers = ProviderSet::new();
    let mut sign_column = SignColumn::new(linenr_scope);
    // Plugin signs registered after diagnostics so a plugin can override at
    // equal priority (`SignColumn`'s tie-break: later-registered wins).
    sign_column.add_source(Box::new(SharedSignSource::new(Arc::clone(
        &signs.diagnostics,
    ))));
    sign_column.add_source(Box::new(SharedSignSource::new(Arc::clone(&signs.plugin))));
    providers.add_gutter_column(Box::new(sign_column));
    providers.add_gutter_column(Box::new(LineNumberColumn::new(
        linenr_scope,
        linenr_selected_scope,
    )));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::BracketMatch,
        data: Arc::clone(&highlights.bracket),
    }));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::SearchMatch,
        data: Arc::clone(&highlights.search),
    }));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::Diagnostic,
        data: Arc::clone(&highlights.diagnostics),
    }));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::Extra,
        data: Arc::clone(&highlights.extra),
    }));
    providers.add_decoration_source(Box::new(InlineDecorationProvider {
        data: Arc::clone(&inlay_hint_map),
    }));
    // Registered after inlay hints so a diagnostic's end-of-line summary
    // sorts to the right of an inlay hint that lands at the same byte
    // offset (both anchor at end-of-line-content in the common case).
    providers.add_decoration_source(Box::new(InlineDecorationProvider {
        data: Arc::clone(&eol_text_map),
    }));
    providers.add_decoration_source(Box::new(PaneVirtualLines {
        data: Arc::clone(&virtual_line_map),
    }));
    providers.add_decoration_source(Box::new(PaneLineBackgrounds {
        data: Arc::clone(&line_bg_map),
    }));
    providers.add_overlay(Box::new(MinibufCompletionOverlay {
        data: Arc::clone(minibuf_completion_view),
    }));
    // Registered after the completion overlay so a hover/signature-help
    // popup paints on top of it (last registration wins z-order).
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(popup_view),
        scope: "ui.popup",
    }));
    // Registered last so the selection menu paints on top of a hover popup
    // (both showing at once is unusual, but the architecture allows it).
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(menu_view),
        scope: "ui.menu",
    }));
    // The LSP completion menu — same widget shape as the selection menu
    // (selected-row styling), registered last (highest z-order): an
    // in-progress completion is the most action-relevant overlay when both
    // could theoretically be visible.
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(completion_menu_view),
        scope: "ui.menu",
    }));
    // Registered last (highest z-order): the picker is full-modal and its
    // key routing sits above every other intercept (`handle_key`), so its
    // paint must sit above every other overlay too.
    providers.add_overlay(Box::new(PickerOverlay {
        data: Arc::clone(picker_view),
    }));

    let pane = Pane {
        providers,
        ..Pane::new(buffer_id)
    };
    (
        pane,
        PaneRenderHandles {
            highlights,
            signs,
            inlay_hints: inlay_hint_map,
            virtual_lines: virtual_line_map,
            eol_text: eol_text_map,
            line_backgrounds: line_bg_map,
        },
    )
}

/// Rows of `area` as plain symbols, trailing spaces trimmed per row.
///
/// Shared by `menu_box`'s and `picker_panel`'s own test modules — both dump
/// a rendered `Grid` region to a string for `insta`/plain assertions, and
/// the dump itself is identical between the two overlay kinds.
#[cfg(test)]
pub(crate) fn symbols_in(buf: &hume_grid::Grid, area: hume_grid::Rect) -> String {
    (area.y..area.y + area.height)
        .map(|y| {
            let row: String = (area.x..area.x + area.width)
                .map(|x| buf[(x, y)].text())
                .collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
