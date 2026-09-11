//! Line-background rendering — a per-pane LINE_BG-kind `DecorationSource` fed
//! from the `line_backgrounds` decoration store, keyed by line so
//! `decorations_for_line` is a plain map lookup.

use rustc_hash::FxHashMap;

use hume_engine::lock::SharedSlot;
use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource};
use hume_engine::types::ScopeId;
use hume_rope::line::ContentLine;

pub(crate) type LineBgMap = SharedSlot<FxHashMap<ContentLine, ScopeId>>;

pub(crate) struct PaneLineBackgrounds {
    pub(crate) data: LineBgMap,
}

impl DecorationSource for PaneLineBackgrounds {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::LINE_BG
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if let Some(&scope) = self.data.read().get(&line_idx) {
            out.push(Decoration::LineBg(scope));
        }
    }
}
