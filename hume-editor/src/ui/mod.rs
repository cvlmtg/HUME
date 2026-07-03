pub(crate) mod completion_overlay;
pub(crate) mod highlight_providers;
pub mod statusline;
pub(crate) mod theme;

use std::sync::{Arc, RwLock};

use hume_engine::builtins::line_number::LineNumberColumn;
use hume_engine::pane::{Pane, WrapMode};
use hume_engine::pipeline::BufferId;
use hume_engine::providers::{HighlightTier, ProviderSet};
use hume_engine::theme::ScopeRegistry;

use completion_overlay::CompletionOverlay;
use highlight_providers::{HighlightRanges, SharedHighlighter};

/// Build a new pane viewing `buffer_id`: a line-number gutter, the
/// bracket-match / search-match highlight sources, the completion popup
/// overlay, and `wrap_mode` seeded from the caller's current settings.
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
    bracket_hl_data: &HighlightRanges,
    search_hl_data: &HighlightRanges,
    completion_view: &Arc<RwLock<Option<completion_overlay::CompletionView>>>,
    wrap_mode: WrapMode,
    buffer_id: BufferId,
) -> Pane {
    let bracket_scope = registry.intern("ui.cursor.match");
    let search_scope = registry.intern("ui.selection.search");

    let mut providers = ProviderSet::new();
    providers.add_gutter_column(Box::new(LineNumberColumn::default()));
    providers.add_highlight_source(Box::new(SharedHighlighter {
        scope: bracket_scope,
        tier: HighlightTier::BracketMatch,
        data: Arc::clone(bracket_hl_data),
    }));
    providers.add_highlight_source(Box::new(SharedHighlighter {
        scope: search_scope,
        tier: HighlightTier::SearchMatch,
        data: Arc::clone(search_hl_data),
    }));
    providers.add_overlay(Box::new(CompletionOverlay {
        data: Arc::clone(completion_view),
    }));

    Pane {
        providers,
        ..Pane::new(buffer_id, wrap_mode)
    }
}
