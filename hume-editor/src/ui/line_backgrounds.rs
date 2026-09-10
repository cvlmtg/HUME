//! Line-background rendering — a per-pane LINE_BG-kind `DecorationSource` fed
//! from the `line_backgrounds` decoration store, keyed by line so
//! `decorations_for_line` is a plain map lookup.

use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource};
use hume_engine::types::ScopeId;
use hume_rope::line::ContentLine;

pub(in crate::ui) type LineBgMap = Arc<RwLock<FxHashMap<ContentLine, ScopeId>>>;

pub(in crate::ui) struct PaneLineBackgrounds {
    pub(crate) data: LineBgMap,
}

impl DecorationSource for PaneLineBackgrounds {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::LINE_BG
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if let Some(&scope) = self.data.read_or_panic().get(&line_idx) {
            out.push(Decoration::LineBg(scope));
        }
    }
}
