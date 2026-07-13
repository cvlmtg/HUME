//! Engine-compatible sign sources feeding the gutter's `SignColumn`.
//!
//! Each source wraps an `Arc<RwLock<HashMap<line_idx, Vec<Sign>>>>` that the
//! editor writes once per frame (after scroll is resolved, before
//! `term.draw` — same cluster as the highlight providers). `signs_for_line`
//! is then a cheap map lookup, matching `SignSource`'s per-row-per-frame
//! contract.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use hume_engine::builtins::sign_column::{Sign, SignSource};
use hume_engine::providers::GutterRowCtx;

/// Shared per-frame sign data: up to N priority-ordered `Sign`s per line
/// (where N = the buffer's configured `signcolumn` columns).
pub(crate) type SignMap = Arc<RwLock<HashMap<usize, Vec<Sign>>>>;

/// The pair of sign maps every pane owns: diagnostics (Rust-owned) and
/// plugin signs (`set-signs!`, all sources pre-merged at write time).
/// Never shared across panes — same rationale as `PaneHighlights`.
#[derive(Default)]
pub(crate) struct PaneSigns {
    pub(crate) diagnostics: SignMap,
    pub(crate) plugin: SignMap,
}

/// One `SignSource` reading a shared per-frame line->signs map. Used for both
/// the diagnostics map and the merged plugin-signs map — the two differ only
/// in which `Arc` they wrap and which priority the write side assigns, not in
/// how they're read.
pub(crate) struct SharedSignSource {
    data: SignMap,
}

impl SharedSignSource {
    pub(crate) fn new(data: SignMap) -> Self {
        Self { data }
    }
}

impl SignSource for SharedSignSource {
    fn signs_for_line(&self, line_idx: usize, _ctx: &GutterRowCtx) -> Vec<Sign> {
        self.data
            .read()
            .expect("RwLock not poisoned")
            .get(&line_idx)
            .cloned()
            .unwrap_or_default()
    }
}
