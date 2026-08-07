//! Inline-decoration rendering — a per-pane INLINE-kind `DecorationSource`
//! fed from an `InlineInsert` map, keyed by line so `decorations_for_line`
//! is a plain map lookup. Two independent instances share this one type:
//! inlay hints (fed from the `inlay_hints` decoration store) and EOL text
//! (fed from `eol_text`) — same shape, distinct Arcs/`ProviderId`s, named by
//! their client on `PaneRenderHandles` rather than on this type.

use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource, InlineInsert};

pub(crate) type InlineDecorationMap = Arc<RwLock<FxHashMap<usize, Vec<InlineInsert>>>>;

pub(crate) struct InlineDecorationProvider {
    pub(crate) data: InlineDecorationMap,
}

impl DecorationSource for InlineDecorationProvider {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if let Some(hints) = self.data.read_or_panic().get(&line_idx) {
            out.extend(hints.iter().cloned().map(Decoration::Inline));
        }
    }
}
