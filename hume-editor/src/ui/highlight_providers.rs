//! Engine-compatible decoration sources for bracket matching and search.
//!
//! Each provider wraps an `Arc<RwLock<Vec<(line_idx, byte_start, byte_end)>>>`
//! that the editor writes once per frame (after scroll is resolved, before
//! `term.draw`). The provider reads the shared data in
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

/// Shared per-frame highlight data: `(line_idx, byte_start, byte_end)` triples,
/// written once per frame and read during the engine's per-line render loop.
pub(crate) type HighlightRanges = Arc<RwLock<Vec<(usize, usize, usize)>>>;

/// The pair of highlight buffers every pane owns.
///
/// Each pane gets its own `bracket`/`search` Arcs (never shared across panes —
/// see `build_pane`), so `update_highlight_providers` can compute one pane's
/// matches from that pane's own buffer and viewport without bleeding into any
/// other pane's rendering.
#[derive(Default)]
pub(crate) struct PaneHighlights {
    pub(crate) bracket: HighlightRanges,
    pub(crate) search: HighlightRanges,
    pub(crate) diagnostics: ScopedHighlightRanges,
    pub(crate) extra: ScopedHighlightRanges,
}

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

impl DecorationSource for SharedHighlighter {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::HIGHLIGHT
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        let data = self.data.read_or_panic();
        // Data is sorted by line_idx (search matches) or tiny (bracket match),
        // so binary-search to the first entry for this line.
        let start = data.partition_point(|&(l, _, _)| l < line_idx);
        for &(l, byte_start, byte_end) in &data[start..] {
            if l != line_idx {
                break;
            }
            out.push(Decoration::Highlight {
                byte_start,
                byte_end,
                scope: self.scope,
                tier: self.tier,
            });
        }
    }
}

/// Shared per-frame highlight data carrying a per-range scope:
/// `(line_idx, byte_start, byte_end, scope)`, written once per frame and read
/// during the engine's per-line render loop.
pub(crate) type ScopedHighlightRanges = Arc<RwLock<Vec<(usize, usize, usize, ScopeId)>>>;

/// Highlights a set of byte ranges at a fixed tier, each carrying its own
/// scope (diagnostics: one scope per severity; extra highlights: one scope
/// per plugin-supplied span). Distinct from `SharedHighlighter`, which shares
/// a single scope across all its ranges — forcing diagnostics/extra through
/// that shape would mean one provider per scope, which doesn't fit either
/// caller.
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
