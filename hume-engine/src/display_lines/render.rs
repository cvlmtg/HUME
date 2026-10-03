//! Render-stage access (`render_display_line`).

use crate::format::FormatBound;

use super::pos::{BlockSlot, DisplayLinePos};
use super::{DisplayLineMap, RenderDisplayLine};

impl<'a> DisplayLineMap<'a> {
    /// Borrow what the render stage needs to style and compose `pos`.
    ///
    /// Content display lines come from the cached format, so a line is
    /// formatted once however many of its display lines get rendered.
    /// Virtual display lines were laid out when the line's block was built
    /// (`format_virtual_lines`), with the same grapheme/width/column/wrap
    /// bookkeeping `format_buffer_line` does for real lines, so a provider
    /// handing over plain text and scoped byte ranges cannot get that
    /// arithmetic wrong.
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
                    line_text: format.line_text.as_str(),
                    virtual_texts: &format.virtual_texts,
                    base_scope: None,
                }
            }
            BlockSlot::Before(i) => self.virtual_display_line(idx, i),
            // `resolve` already walked this line's block, so its `before`
            // count is on the entry it handed back, so no need to walk it again.
            BlockSlot::After(i) => {
                let before = self.store.entry(idx).before;
                self.virtual_display_line(idx, before + i)
            }
        }
    }

    /// Borrow the `vi`th laid-out virtual display line of entry `idx`.
    ///
    /// Read from the entry's `virtual_format`, not the line's own format: a
    /// `Before` display line renders ahead of its line's content display
    /// lines, which are very likely already formatted (`block` runs the
    /// formatter in wrapping mode to count wrap display lines), and the two
    /// must not share buffers.
    fn virtual_display_line(&self, idx: usize, vi: usize) -> RenderDisplayLine<'_> {
        let vf = &self.store.entry(idx).virtual_format;
        RenderDisplayLine {
            display_line: &vf.display_lines[vi],
            graphemes: &vf.graphemes,
            // A virtual display line has no buffer text: every cell resolves out of
            // `virtual_texts` instead: `Virtual` text itself, or the
            // `Placeholder` a control character becomes. A tab needs no
            // arena lookup at all; it's `TabFill`, drawn as blanks directly.
            line_text: "",
            virtual_texts: &vf.texts,
            base_scope: vf.base_scopes[vi],
        }
    }
}
