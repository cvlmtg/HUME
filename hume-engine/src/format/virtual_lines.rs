//! Lays a buffer line's provider virtual lines out into display lines.

use hume_rope::column::ByteCol;
use hume_rope::line::{ContentLine, RopeyLine};
use unicode_segmentation::UnicodeSegmentation;

use crate::pane::WrapMode;
use crate::providers::{VirtualLine, VirtualLineAnchor};
use crate::style::highlight::IntervalCursor;

use super::virtual_cells::{VirtualCells, VirtualRun};
use super::{VirtualFormat, WrapOwner, WrapState, close_display_line_at};

/// Lay `virtual_lines` (sorted `Before` ones first, all anchored to
/// `anchor_line`) out into `out`, wrapping each under `wrap_mode` the way
/// [`format_buffer_line`](super::format_buffer_line) wraps buffer text: one
/// virtual line becomes as many display lines as it needs, all of the same
/// `DisplayLineKind::Virtual`. `out` must be empty.
///
/// Sets `out.before` to how many of `out`'s display lines come from
/// `Before`-anchored virtual lines.
pub(crate) fn format_virtual_lines(
    virtual_lines: &[VirtualLine],
    anchor_line: ContentLine,
    tab_width: u8,
    wrap_mode: &WrapMode,
    out: &mut VirtualFormat,
) {
    debug_assert!(out.display_lines.is_empty(), "`out` must start empty");
    let anchor_line = RopeyLine::from(anchor_line);

    for vl in virtual_lines {
        let owner = WrapOwner::Virtual {
            provider_id: vl.provider_id,
            anchor_line,
        };
        let mut wrap = WrapState::start(
            owner,
            wrap_mode,
            hume_rope::width::indent_depth(&vl.text, tab_width),
            tab_width,
            &mut out.display_lines,
            out.graphemes.len(),
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
            let scope = scope_cursor
                .scope_at(ByteCol::new(byte_offset))
                .or(vl.base_scope);
            cells.push_wrapping(
                &mut wrap,
                &mut out.display_lines,
                byte_offset,
                cluster,
                scope,
            );
        }

        close_display_line_at(
            &mut out.display_lines,
            wrap.line_g_start,
            out.graphemes.len(),
        );
        out.base_scopes
            .resize(out.display_lines.len(), vl.base_scope);
        if matches!(vl.anchor, VirtualLineAnchor::Before(_)) {
            out.before = out.display_lines.len();
        }
    }
}
