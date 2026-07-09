//! Inlay-hint rendering — a per-pane `InlineDecoration` fed from the
//! `inlay_hints` decoration store, keyed by line so `decorations_for_line` is a
//! plain map lookup.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use hume_engine::providers::{InlineDecoration, InlineInsert};

pub(crate) type InlayHintMap = Arc<RwLock<HashMap<usize, Vec<InlineInsert>>>>;

pub(crate) struct InlayHintProvider {
    pub(crate) data: InlayHintMap,
}

impl InlineDecoration for InlayHintProvider {
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<InlineInsert>) {
        if let Some(hints) = self
            .data
            .read()
            .expect("RwLock not poisoned")
            .get(&line_idx)
        {
            out.extend(hints.iter().cloned());
        }
    }
}
