pub(crate) mod completion_overlay;
pub(crate) mod highlight_providers;
pub(crate) mod popup;
pub(crate) mod signs;
pub mod statusline;
pub(crate) mod theme;

use std::sync::{Arc, RwLock};

use hume_engine::builtins::line_number::LineNumberColumn;
use hume_engine::builtins::sign_column::SignColumn;
use hume_engine::pane::{Pane, WrapMode};
use hume_engine::pipeline::BufferId;
use hume_engine::providers::{HighlightTier, ProviderSet};
use hume_engine::theme::ScopeRegistry;

use completion_overlay::CompletionOverlay;
use highlight_providers::{PaneHighlights, ScopedHighlighter, SharedHighlighter};
use popup::PopupOverlay;
use signs::{PaneSigns, SharedSignSource};

/// Build a new pane viewing `buffer_id`: a sign column, a line-number
/// gutter, the bracket-match / search-match / diagnostic / extra-highlight
/// sources, the completion popup overlay, the hover-popup overlay, the
/// selection-menu overlay, and `wrap_mode` seeded from the caller's current
/// settings.
///
/// Returns the pane together with its freshly-allocated [`PaneHighlights`]
/// and [`PaneSigns`] — every pane gets its own buffers (never shared with any
/// other pane), so each pane's decorations are computed from that pane's own
/// buffer and viewport. The caller stores them in `EditorState.panes.highlights`
/// / `.signs` keyed by the new pane's id.
///
/// The gutter column is added with its default style — `prepare_frame` syncs
/// the buffer-resolved `line-number-style` into every pane's gutter before
/// each render (see `sync_line_number_style`), so the style seeded here never
/// reaches a frame. Interning `bracket_scope`/`search_scope` here is safe even
/// for panes built after the last bake (e.g. splits): `prepare_frame` calls
/// `Theme::bake_if_stale` every frame, so any scope interned since the last
/// bake is picked up before the next render.
///
/// Single source of truth for pane construction — every pane-creation site
/// (`Editor::open`'s bootstrap pane, `commands::open_pane`) goes through this
/// so all panes render identically and the provider list / `wrap_mode` seed
/// can't drift apart. A pane built with `Pane::new` alone has an empty
/// `ProviderSet` (no gutter column at all).
pub(crate) fn build_pane(
    registry: &mut ScopeRegistry,
    completion_view: &Arc<RwLock<Option<completion_overlay::CompletionView>>>,
    popup_view: &Arc<RwLock<Option<popup::PopupState>>>,
    menu_view: &Arc<RwLock<Option<popup::PopupState>>>,
    wrap_mode: WrapMode,
    buffer_id: BufferId,
) -> (Pane, PaneHighlights, PaneSigns) {
    let bracket_scope = registry.intern("ui.cursor.match");
    let search_scope = registry.intern("ui.selection.search");

    let highlights = PaneHighlights::default();
    let signs = PaneSigns::default();

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

    let pane = Pane {
        providers,
        ..Pane::new(buffer_id, wrap_mode)
    };
    (pane, highlights, signs)
}
