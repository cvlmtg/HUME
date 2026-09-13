//! Render-stage access (`render_display_line`/`segment_virtual_line`) —
//! moved out of `display_lines.rs`'s per-role split.

use hume_rope::column::DisplayLineCol;

use crate::format::FormatBound;
use crate::types::DisplayLine;

use super::pos::{BlockSlot, DisplayLinePos};
use super::{DisplayLineMap, RenderDisplayLine};

impl<'a> DisplayLineMap<'a> {
    /// Borrow what the render stage needs to style and compose `pos`.
    ///
    /// Content display lines come from the cached format, so a line is
    /// formatted once however many of its display lines get rendered.
    /// Virtual display lines are segmented here — the same
    /// grapheme/width/column bookkeeping `format_buffer_line` does for real
    /// lines, so a provider handing over plain text and scoped byte ranges
    /// cannot get that arithmetic wrong.
    pub fn render_display_line(&mut self, pos: DisplayLinePos) -> RenderDisplayLine<'_> {
        let (idx, slot) = self.resolve(pos);
        match slot {
            BlockSlot::Content(sub) => {
                // `Full`: the render stage emits whole display lines, and
                // its own clipping is the map's `h_window`, applied inside
                // the format.
                self.ensure_format_at(idx, FormatBound::Full);
                let format = self.format_at(idx);
                RenderDisplayLine {
                    display_line: &format.display_lines[sub],
                    graphemes: &format.graphemes,
                    line_text: &format.line_texts,
                    virtual_texts: &format.virtual_texts,
                    base_scope: None,
                }
            }
            BlockSlot::Before(i) => self.segment_virtual_line(idx, i),
            // `resolve` already walked this line's block, so its `before`
            // count is on the entry it handed back — no need to walk it again.
            BlockSlot::After(i) => {
                let before = self.store.entry(idx).before;
                self.segment_virtual_line(idx, before + i)
            }
        }
    }

    /// Lay one virtual display line out into its own scratch and borrow it
    /// back.
    ///
    /// Uses the store's `virtual_line`, not the line's own format: a
    /// `Before` display line renders ahead of its line's content display
    /// lines, which are very likely already formatted (`block` runs the
    /// formatter in wrapping mode to count wrap display lines) — laying the
    /// virtual display line out over them would destroy that and force a
    /// reformat of the content display lines that follow.
    fn segment_virtual_line(&mut self, idx: usize, vl_idx: usize) -> RenderDisplayLine<'_> {
        let tab_width = self.key.tab_width;
        // The entry's virtual display lines and the scratch they lay out
        // into are disjoint parts of the store, borrowed together so the
        // display line's text can be read while its cells are written.
        let (entry, vline) = self.store.entry_and_virtual_line(idx);
        let vl = &entry.virtual_lines[vl_idx];
        let anchor_line = entry.line.into();
        let provider_id = vl.provider_id;
        let base_scope = vl.base_scope;
        vline.clear();

        // `vl.segments` was sorted by `block()` at intake, and
        // `grapheme_indices` yields byte offsets in ascending order, so a
        // single monotonic cursor resolves every grapheme's scope in
        // O(graphemes + segments) instead of a per-grapheme linear scan.
        let mut scope_cursor = crate::style::highlight::IntervalCursor::new(&vl.segments);
        let mut display_col = DisplayLineCol::new(0);
        crate::format::push_virtual_cells(
            &mut vline.texts,
            &mut vline.graphemes,
            &crate::format::VirtualRun {
                text: &vl.text,
                byte_offset: 0, // no buffer position
                char_offset: usize::MAX,
                indent_depth: 0,
            },
            tab_width,
            &mut display_col,
            |byte_offset| scope_cursor.scope_at(byte_offset).or(base_scope),
        );

        let display_line = vline.display_line.insert(DisplayLine {
            kind: crate::types::DisplayLineKind::Virtual {
                provider_id,
                anchor_line,
            },
            graphemes: 0..vline.graphemes.len(),
        });

        RenderDisplayLine {
            display_line,
            graphemes: &vline.graphemes,
            // A virtual display line has no buffer text — every cell resolves out of
            // `virtual_texts` instead: `Virtual` text itself, or the
            // `Placeholder` a control character becomes. A tab needs no
            // arena lookup at all — it's `TabFill`, drawn as blanks directly.
            line_text: "",
            virtual_texts: &vline.texts,
            base_scope,
        }
    }
}
