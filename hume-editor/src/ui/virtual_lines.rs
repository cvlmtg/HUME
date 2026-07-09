//! Virtual-line rendering — a per-pane `VirtualLineSource` fed from
//! the `virtual_lines` decoration store, keyed by anchor line so `virtual_lines`
//! (the trait method) is a plain map lookup per queried line.
//!
//! Unlike the popup/menu widgets, this provider is consulted by *scroll and
//! cursor math* (`display_rows_for_line`), not just rendering — so the
//! per-line lookup must stay cheap (a `HashMap` get + `Vec` clone-out, no
//! per-frame allocation-heavy work).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, RwLock};

use hume_engine::providers::{VirtualLine, VirtualLineSource};

pub(crate) type VirtualLineMap = Arc<RwLock<HashMap<usize, Vec<VirtualLine>>>>;

pub(crate) struct PaneVirtualLines {
    pub(crate) data: VirtualLineMap,
}

impl VirtualLineSource for PaneVirtualLines {
    fn virtual_lines(
        &self,
        visible_lines: Range<usize>,
        _content_width: u16,
        out: &mut Vec<VirtualLine>,
    ) {
        let guard = self.data.read().expect("RwLock not poisoned");
        for line in visible_lines {
            if let Some(lines) = guard.get(&line) {
                out.extend(lines.iter().cloned());
            }
        }
    }
}
