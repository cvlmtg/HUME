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

use engine::providers::{HighlightSource, HighlightTier, ProviderId, SourceContext};
use engine::types::ScopeId;

// ── BracketMatchHighlighter ───────────────────────────────────────────────────

/// Highlights the bracket that pairs with the one under the cursor.
///
/// Data is `(line_idx, byte_start, byte_end)` in line-relative byte offsets.
/// The editor writes this via the shared `Arc<RwLock<...>>` once per frame
/// in `update_highlight_providers()`.
pub(crate) struct BracketMatchHighlighter {
    pub(crate) id: ProviderId,
    pub(crate) scope: ScopeId,
    /// Shared data: `(line_idx, byte_start, byte_end)` for each bracket match.
    pub(crate) data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
}

impl HighlightSource for BracketMatchHighlighter {
    fn id(&self) -> ProviderId { self.id }
    fn tier(&self) -> HighlightTier { HighlightTier::BracketMatch }

    fn highlights_for_line(
        &self,
        line_idx: usize,
        _ctx: &SourceContext,
        out: &mut Vec<(usize, usize, ScopeId)>,
    ) {
        // Unwrap: we never poison the lock (no panics while holding it).
        let data = self.data.read().unwrap();
        for &(l, byte_start, byte_end) in data.iter() {
            if l == line_idx {
                out.push((byte_start, byte_end, self.scope));
            }
        }
    }
}

// ── SearchMatchHighlighter ────────────────────────────────────────────────────

/// Highlights all search matches currently visible.
///
/// Data is `(line_idx, byte_start, byte_end)` in line-relative byte offsets.
/// The editor converts char-offset match pairs from `SearchState::matches()`
/// to line-relative byte offsets once per frame in `update_highlight_providers()`.
pub(crate) struct SearchMatchHighlighter {
    pub(crate) id: ProviderId,
    pub(crate) scope: ScopeId,
    /// Shared data: `(line_idx, byte_start, byte_end)` for each search match.
    pub(crate) data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
}

impl HighlightSource for SearchMatchHighlighter {
    fn id(&self) -> ProviderId { self.id }
    fn tier(&self) -> HighlightTier { HighlightTier::SearchMatch }

    fn highlights_for_line(
        &self,
        line_idx: usize,
        _ctx: &SourceContext,
        out: &mut Vec<(usize, usize, ScopeId)>,
    ) {
        let data = self.data.read().unwrap();
        for &(l, byte_start, byte_end) in data.iter() {
            if l == line_idx {
                out.push((byte_start, byte_end, self.scope));
            }
        }
    }
}
