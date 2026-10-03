//! Lays a buffer line's provider virtual lines out into display lines.

use hume_rope::column::ByteCol;
use hume_rope::line::{ContentLine, RopeyLine};
use unicode_segmentation::UnicodeSegmentation;

use crate::pane::WrapMode;
use crate::providers::{VirtualLine, VirtualLineAnchor};
use crate::style::highlight::IntervalCursor;

use super::virtual_cells::{VirtualCells, VirtualRun};
use super::{
    VirtualFormat, WrapOwner, WrapState, close_display_line_at, continuation_indent,
    is_whitespace_grapheme,
};

/// Lay `virtual_lines` (sorted `Before` ones first, all anchored to
/// `anchor_line`) out into `out`, wrapping each under `wrap_mode` the way
/// [`format_buffer_line`](super::format_buffer_line) wraps buffer text: one
/// virtual line becomes as many display lines as it needs, all of the same
/// `DisplayLineKind::Virtual`. `out` must be empty.
///
/// Returns how many of `out`'s display lines come from `Before`-anchored
/// virtual lines.
pub(crate) fn format_virtual_lines(
    virtual_lines: &[VirtualLine],
    anchor_line: ContentLine,
    tab_width: u8,
    wrap_mode: &WrapMode,
    out: &mut VirtualFormat,
) -> usize {
    debug_assert!(out.display_lines.is_empty(), "`out` must start empty");
    let wrap_width = wrap_mode.wrap_width().map(u32::from);
    let word_break = wrap_mode.breaks_at_word();
    let anchor_line = RopeyLine::from(anchor_line);

    let mut before = 0;
    for vl in virtual_lines {
        let owner = WrapOwner::Virtual {
            provider_id: vl.provider_id,
            anchor_line,
        };
        let indent_display_cols = continuation_indent(
            wrap_mode,
            hume_rope::width::indent_depth(&vl.text, tab_width),
            tab_width,
        );
        let mut wrap = WrapState::start(
            owner,
            &mut out.display_lines,
            out.graphemes.len(),
            word_break,
        );

        // `vl.segments` was sorted at intake, and `grapheme_indices` yields
        // byte offsets in ascending order, so a single monotonic cursor
        // resolves every grapheme's scope in O(graphemes + segments).
        let mut scope_cursor = IntervalCursor::new(&vl.segments);
        let run = VirtualRun {
            text: &vl.text,
            byte_offset: 0, // no buffer position
            pos: None,
            indent_depth: 0,
        };
        let mut cells = VirtualCells::new(&mut out.texts, &mut out.graphemes, &run, tab_width);
        for (byte_offset, cluster) in vl.text.grapheme_indices(true) {
            let measured_at = wrap.current_display_col;
            let mut classified = cells.classify_at(cluster, measured_at);
            wrap.maybe_wrap(
                classified.width() as u8,
                wrap_width,
                indent_display_cols,
                0,
                &mut out.display_lines,
                cells.graphemes_out,
            );

            let is_ws = is_whitespace_grapheme(cluster);
            let scope = scope_cursor
                .scope_at(ByteCol::new(byte_offset))
                .or(vl.base_scope);
            if wrap.current_display_col != measured_at {
                classified = cells.classify_at(cluster, wrap.current_display_col);
            }
            cells.push_classified(
                byte_offset,
                cluster,
                classified,
                &mut wrap.current_display_col,
                scope,
            );
            wrap.note_ws(is_ws, cells.graphemes_out.len());
        }

        close_display_line_at(
            &mut out.display_lines,
            wrap.line_g_start,
            out.graphemes.len(),
        );
        out.base_scopes
            .resize(out.display_lines.len(), vl.base_scope);
        if matches!(vl.anchor, VirtualLineAnchor::Before(_)) {
            before = out.display_lines.len();
        }
    }
    before
}
