//! Engine-compatible decoration sources for bracket matching, search, and
//! diagnostic/extra highlights — all four share one `ScopedHighlighter`
//! shape.
//!
//! Each provider wraps an `Arc<RwLock<Vec<(line_idx, byte_start, byte_end,
//! scope)>>>` that the editor writes once per frame (after scroll is
//! resolved, before `term.draw`). The provider reads the shared data in
//! `decorations_for_line()` during the engine's per-line render loop.
//!
//! Using `Arc<RwLock<...>>` is correct: it satisfies `Send + Sync`, needed
//! since providers are boxed into the pane's `ProviderSet` and must outlive
//! the frame that writes them, and is uncontended in practice (~25ns per
//! lock/unlock). Do not replace with `UnsafeCell`.

use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource, HighlightTier};
use hume_engine::types::ScopeId;

/// Shared per-frame highlight data carrying a per-range scope:
/// `(line_idx, byte_start, byte_end, scope)`, written once per frame and read
/// during the engine's per-line render loop.
pub(crate) type ScopedHighlightRanges = Arc<RwLock<Vec<(usize, usize, usize, ScopeId)>>>;

/// The four highlight buffers every pane owns.
///
/// Each pane gets its own Arcs (never shared across panes — see
/// `build_pane`), so `update_highlight_providers` can compute one pane's
/// matches from that pane's own buffer and viewport without bleeding into any
/// other pane's rendering.
#[derive(Default)]
pub(crate) struct PaneHighlights {
    pub(crate) bracket: ScopedHighlightRanges,
    pub(crate) search: ScopedHighlightRanges,
    pub(crate) diagnostics: ScopedHighlightRanges,
    pub(crate) extra: ScopedHighlightRanges,
}

/// Highlights a set of byte ranges at a fixed tier, each carrying its own
/// scope. Bracket/search match each carry one editor-wide constant scope
/// (`ui.cursor.match`/`ui.selection.search`, interned once per frame in
/// `Editor::update_highlight_providers`) written into every span at push
/// time; diagnostics carry one scope per severity; extra
/// highlights carry one scope per plugin-supplied span — all four write the
/// scope onto the span rather than fixing it on the provider, so one shape
/// serves every caller without forcing a one-provider-per-scope split for
/// diagnostics/extra.
pub(crate) struct ScopedHighlighter {
    pub(crate) tier: HighlightTier,
    /// Shared data: `(line_idx, byte_start, byte_end, scope)` for each highlight.
    pub(crate) data: ScopedHighlightRanges,
}

impl DecorationSource for ScopedHighlighter {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::HIGHLIGHT
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        let data = self.data.read_or_panic();
        let start = data.partition_point(|&(l, _, _, _)| l < line_idx);
        for &(l, byte_start, byte_end, scope) in &data[start..] {
            if l != line_idx {
                break;
            }
            out.push(Decoration::Highlight {
                byte_start,
                byte_end,
                scope,
                tier: self.tier,
            });
        }
    }
}
