//! Inlay-hint rendering — a per-pane INLINE-kind `DecorationSource` fed from
//! the `inlay_hints` decoration store, keyed by line so `decorations_for_line`
//! is a plain map lookup.

use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource, InlineInsert};

pub(crate) type InlayHintMap = Arc<RwLock<FxHashMap<usize, Vec<InlineInsert>>>>;

pub(crate) struct InlayHintProvider {
    pub(crate) data: InlayHintMap,
}

impl DecorationSource for InlayHintProvider {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if let Some(hints) = self.data.read_or_panic().get(&line_idx) {
            out.extend(hints.iter().cloned().map(Decoration::Inline));
        }
    }
}
