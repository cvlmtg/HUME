pub(crate) mod completion_overlay;
pub(crate) mod drawer;
pub(crate) mod highlight_providers;
pub(crate) mod inlay_hints;
pub(crate) mod menu_box;
pub(crate) mod popup;
pub(crate) mod signs;
pub mod statusline;
pub(crate) mod theme;
pub(crate) mod virtual_lines;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use hume_engine::builtins::line_number::LineNumberColumn;
use hume_engine::builtins::sign_column::SignColumn;
use hume_engine::pane::{Pane, WrapMode};
use hume_engine::pipeline::BufferId;
use hume_engine::providers::{HighlightTier, ProviderSet};
use hume_engine::theme::ScopeRegistry;

use completion_overlay::CompletionOverlay;
use highlight_providers::{PaneHighlights, ScopedHighlighter, SharedHighlighter};
use inlay_hints::{InlayHintMap, InlayHintProvider};
use popup::PopupOverlay;
use signs::{PaneSigns, SharedSignSource};
use virtual_lines::{PaneVirtualLines, VirtualLineMap};

/// A pane's four render-decoration handles, allocated together by
/// [`build_pane`] and stored as one `SecondaryMap` entry on
/// `EditorState.panes.render` — they are always seeded and dropped as a
/// unit (never independently), and every read site borrows the map
/// shared, so bundling them costs nothing and removes the "added a new
/// per-pane provider, forgot to drop it in `drop_pane_state`" bug class.
pub(crate) struct PaneRenderHandles {
    pub(crate) highlights: PaneHighlights,
    pub(crate) signs: PaneSigns,
    pub(crate) inlay_hints: InlayHintMap,
    pub(crate) virtual_lines: VirtualLineMap,
}

/// Build a new pane viewing `buffer_id`: sign column, line-number gutter,
/// bracket-match/search-match/diagnostic/extra-highlight sources, inlay-hint
/// decoration, virtual-line source, completion/hover/selection-menu/LSP
/// overlays, and `wrap_mode` seeded from the caller's current settings.
///
/// Returns the pane with its freshly-allocated [`PaneRenderHandles`] — every
/// pane gets its own buffers (never shared), so each pane's decorations come
/// from that pane's own buffer and viewport. The caller stores them in
/// `EditorState.panes.render` keyed by the new pane's id.
///
/// The gutter column is added with its default style; `prepare_frame` syncs
/// the buffer-resolved `line-number-style` into every pane's gutter before
/// each render (see `sync_line_number_style`), so the seeded style never
/// reaches a frame. Interning `bracket_scope`/`search_scope` here is safe
/// even for panes built after the last bake (e.g. splits) — `prepare_frame`
/// calls `Theme::bake_if_stale` every frame, picking up any scope interned
/// since.
///
/// Single source of truth for pane construction — every creation site
/// (`Editor::open`'s bootstrap pane, `commands::open_pane`) goes through
/// this, so panes render identically. `Pane::new` alone has an empty
/// `ProviderSet` (no gutter column).
pub(crate) fn build_pane(
    registry: &mut ScopeRegistry,
    completion_view: &Arc<RwLock<Option<completion_overlay::CompletionView>>>,
    popup_view: &Arc<RwLock<Option<popup::PopupState>>>,
    menu_view: &Arc<RwLock<Option<popup::PopupState>>>,
    lsp_completion_view: &Arc<RwLock<Option<popup::PopupState>>>,
    wrap_mode: WrapMode,
    buffer_id: BufferId,
) -> (Pane, PaneRenderHandles) {
    let bracket_scope = registry.intern("ui.cursor.match");
    let search_scope = registry.intern("ui.selection.search");

    let highlights = PaneHighlights::default();
    let signs = PaneSigns::default();
    let inlay_hint_map: InlayHintMap = Arc::new(RwLock::new(HashMap::new()));
    let virtual_line_map: VirtualLineMap = Arc::new(RwLock::new(HashMap::new()));

    let mut providers = ProviderSet::new();
    let mut sign_column = SignColumn::new();
    // Plugin signs registered after diagnostics so a plugin can override at
    // equal priority (`SignColumn`'s tie-break: later-registered wins).
    sign_column.add_source(Box::new(SharedSignSource::new(Arc::clone(
        &signs.diagnostics,
    ))));
    sign_column.add_source(Box::new(SharedSignSource::new(Arc::clone(&signs.plugin))));
    providers.add_gutter_column(Box::new(sign_column));
    providers.add_gutter_column(Box::new(LineNumberColumn::default()));
    providers.add_highlight_source(Box::new(SharedHighlighter {
        scope: bracket_scope,
        tier: HighlightTier::BracketMatch,
        data: Arc::clone(&highlights.bracket),
    }));
    providers.add_highlight_source(Box::new(SharedHighlighter {
        scope: search_scope,
        tier: HighlightTier::SearchMatch,
        data: Arc::clone(&highlights.search),
    }));
    providers.add_highlight_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::Diagnostic,
        data: Arc::clone(&highlights.diagnostics),
    }));
    providers.add_highlight_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::Extra,
        data: Arc::clone(&highlights.extra),
    }));
    providers.add_inline_decoration(Box::new(InlayHintProvider {
        data: Arc::clone(&inlay_hint_map),
    }));
    providers.add_virtual_line_source(Box::new(PaneVirtualLines {
        data: Arc::clone(&virtual_line_map),
    }));
    providers.add_overlay(Box::new(CompletionOverlay {
        data: Arc::clone(completion_view),
    }));
    // Registered after the completion overlay so a hover/signature-help
    // popup paints on top of it (last registration wins z-order).
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(popup_view),
        scope: "ui.popup",
        selected_scope: None,
    }));
    // Registered last so the selection menu paints on top of a hover popup
    // (both showing at once is unusual, but the architecture allows it).
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(menu_view),
        scope: "ui.menu",
        selected_scope: Some("ui.menu.selected"),
    }));
    // The LSP completion menu — same widget shape as the selection menu
    // (selected-row styling), registered last (highest z-order): an
    // in-progress completion is the most action-relevant overlay when both
    // could theoretically be visible.
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(lsp_completion_view),
        scope: "ui.menu",
        selected_scope: Some("ui.menu.selected"),
    }));

    let pane = Pane {
        providers,
        ..Pane::new(buffer_id, wrap_mode)
    };
    (
        pane,
        PaneRenderHandles {
            highlights,
            signs,
            inlay_hints: inlay_hint_map,
            virtual_lines: virtual_line_map,
        },
    )
}
