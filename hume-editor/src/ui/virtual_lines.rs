//! Virtual-line rendering — a per-pane VIRTUAL_LINE-kind `DecorationSource`
//! fed from the `virtual_lines` decoration store, keyed by anchor line so
//! `decorations_for_line` is a plain map lookup per queried line.
//!
//! Unlike the popup/menu widgets, this provider is consulted by *scroll and
//! cursor math* through `rows::RowMap`, not just rendering — so the per-line
//! lookup must stay cheap (a `FxHashMap` get + `Vec` clone-out, no per-frame
//! allocation-heavy work).

use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::{Decoration, DecorationKinds, DecorationSource, VirtualLine};

pub(crate) type VirtualLineMap = Arc<RwLock<FxHashMap<usize, Vec<VirtualLine>>>>;

pub(crate) struct PaneVirtualLines {
    pub(crate) data: VirtualLineMap,
}

impl DecorationSource for PaneVirtualLines {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if let Some(lines) = self.data.read_or_panic().get(&line_idx) {
            out.extend(lines.iter().cloned().map(Decoration::VirtualLine));
        }
    }
}
