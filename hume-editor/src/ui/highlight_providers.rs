//! Engine-compatible highlight providers for bracket matching and search.
//!
//! Each provider wraps an `Arc<RwLock<Vec<(line_idx, byte_start, byte_end)>>>`
//! that the editor writes once per frame (after scroll is resolved, before
//! `term.draw`). The provider reads the shared data in `highlights_for_line()`
//! during the engine's per-line render loop.
//!
//! Using `Arc<RwLock<...>>` is correct: it satisfies `Send + Sync` (required
//! by `HighlightSource: Send + Sync`) and is uncontended in practice (~25ns
//! per lock/unlock). Do not replace with `UnsafeCell`.

use std::sync::{Arc, RwLock};

use hume_engine::builtins::line_number::{
    LineNumberColumn, LineNumberStyle as EngineLineNumberStyle,
};
use hume_engine::providers::{HighlightSource, HighlightTier, ProviderSet, SourceContext};
use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;

/// Shared per-frame highlight data: `(line_idx, byte_start, byte_end)` triples,
/// written once per frame and read during the engine's per-line render loop.
pub(crate) type HighlightRanges = Arc<RwLock<Vec<(usize, usize, usize)>>>;

/// Highlights a set of byte ranges, all sharing the same scope and tier.
///
/// Data is `(line_idx, byte_start, byte_end)` in line-relative byte offsets.
/// The editor writes this via the shared `Arc<RwLock<...>>` once per frame
/// in `update_highlight_providers()`.
pub(crate) struct SharedHighlighter {
    pub(crate) scope: ScopeId,
    pub(crate) tier: HighlightTier,
    /// Shared data: `(line_idx, byte_start, byte_end)` for each highlight.
    pub(crate) data: HighlightRanges,
}

impl HighlightSource for SharedHighlighter {
    fn tier(&self) -> HighlightTier {
        self.tier
    }

    fn highlights_for_line(
        &self,
        line_idx: usize,
        _ctx: &SourceContext,
        out: &mut Vec<(usize, usize, ScopeId)>,
    ) {
        let data = self.data.read().expect("RwLock not poisoned");
        // Data is sorted by line_idx (search matches) or tiny (bracket match),
        // so binary-search to the first entry for this line.
        let start = data.partition_point(|&(l, _, _)| l < line_idx);
        for &(l, byte_start, byte_end) in &data[start..] {
            if l != line_idx {
                break;
            }
            out.push((byte_start, byte_end, self.scope));
        }
    }
}

/// Build the provider set every pane gets: a hybrid line-number gutter, the
/// bracket-match / search-match highlight sources, and the completion popup
/// overlay.
///
/// Shared by the initial pane (`Editor::open`) and every split-created pane
/// (`commands::open_pane`) so all panes render identically — a pane built with
/// `Pane::new` alone has an empty `ProviderSet` (no gutter column at all).
pub(crate) fn build_pane_providers(
    registry: &mut ScopeRegistry,
    bracket_hl_data: &HighlightRanges,
    search_hl_data: &HighlightRanges,
    completion_view: &Arc<RwLock<Option<crate::ui::completion_overlay::CompletionView>>>,
) -> ProviderSet {
    let bracket_scope = registry.intern("ui.cursor.match");
    let search_scope = registry.intern("ui.selection.search");

    let mut providers = ProviderSet::new();
    providers.add_gutter_column(Box::new(LineNumberColumn::with_style(
        EngineLineNumberStyle::Hybrid,
    )));
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
    providers.add_overlay(Box::new(crate::ui::completion_overlay::CompletionOverlay {
        data: Arc::clone(completion_view),
    }));
    providers
}
