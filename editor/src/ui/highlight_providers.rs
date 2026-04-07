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

use engine::providers::{HighlightSource, HighlightTier, SourceContext};
use engine::types::ScopeId;

/// Highlights a set of byte ranges, all sharing the same scope and tier.
///
/// Data is `(line_idx, byte_start, byte_end)` in line-relative byte offsets.
/// The editor writes this via the shared `Arc<RwLock<...>>` once per frame
/// in `update_highlight_providers()`.
pub(crate) struct SharedHighlighter {
    pub(crate) scope: ScopeId,
    pub(crate) tier: HighlightTier,
    /// Shared data: `(line_idx, byte_start, byte_end)` for each highlight.
    pub(crate) data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
}

impl HighlightSource for SharedHighlighter {
    fn tier(&self) -> HighlightTier { self.tier }

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
            if l != line_idx { break; }
            out.push((byte_start, byte_end, self.scope));
        }
    }
}
