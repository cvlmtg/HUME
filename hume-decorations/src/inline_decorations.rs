//! Inline-decoration rendering — a per-pane INLINE-kind `DecorationSource`
//! fed from an `InlineInsert` map, keyed by line so `decorations_for_line`
//! is a plain map lookup. Two independent instances share this one type:
//! inlay hints (fed from the `inlay_hints` decoration store) and EOL text
//! (fed from `eol_text`) — same shape, distinct Arcs/`ProviderId`s, named by
//! their client on `PaneDecorationHandles` rather than on this type.

use rustc_hash::FxHashMap;

use hume_engine::lock::SharedSlot;
use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource, InlineInsert};
use hume_rope::line::ContentLine;

pub(crate) type InlineDecorationMap = SharedSlot<FxHashMap<ContentLine, Vec<InlineInsert>>>;

pub(crate) struct InlineDecorationProvider {
    pub(crate) data: InlineDecorationMap,
}

impl DecorationSource for InlineDecorationProvider {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if let Some(hints) = self.data.read().get(&line_idx) {
            out.extend(hints.iter().cloned().map(Decoration::Inline));
        }
    }
}
