//! Per-grapheme display width/content classification — moved out of
//! `format.rs`'s per-role split.

use hume_rope::column::DisplayLineCol;

use crate::pane::{WhitespaceConfig, WhitespaceRender};
use crate::types::CellContent;

use super::virtual_cells::push_arena_text;

/// Compute the display `width` and `CellContent` for one grapheme cluster.
///
/// Width and rendering kind both come from one `hume_rope::width::classify`
/// call — tab, space, NBSP/ideographic space, and regular graphemes alike —
/// so this and every other column computation in the workspace (editing
/// ops, Steel decorations, UI chrome) agree on where a given cluster lands,
/// and the tab-before-placeholder ordering (a tab is also a control
/// character) is decided once instead of re-tested here.
pub(super) fn grapheme_display(
    grapheme_str: &str,
    current_display_col: DisplayLineCol,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    is_trailing: bool,
    virtual_texts: &mut String,
) -> (u8, CellContent) {
    match hume_rope::width::classify(grapheme_str, current_display_col.get() as usize, tab_width) {
        hume_rope::width::Cluster::Tab { width } => {
            let content = if should_render_whitespace(whitespace.tab, is_trailing) {
                let (start, len) = push_arena_text(virtual_texts, whitespace.tab_char);
                CellContent::Whitespace { start, len }
            } else {
                // Tabs render as spaces when the indicator is off.
                CellContent::TabFill
            };
            (width as u8, content)
        }

        // A cluster the terminal must not be shown as itself: a control
        // character it would act on, or an invisible one it would
        // collapse. Renders as its codepoint, `<200b>`, the way Vim and
        // Emacs show them — never as a blank, which would leave a bidi
        // override looking exactly like a space. Not gated by any
        // `whitespace-*` setting: these are unrenderable rather than
        // merely invisible, and a reader who cannot see them cannot
        // review what they do. Same substitution `push_virtual_cells` and
        // `render::write_text_run` make, so the whole frame answers this
        // the same way.
        hume_rope::width::Cluster::Placeholder(p) => {
            let (start, len) = push_arena_text(virtual_texts, p.as_str());
            (len as u8, CellContent::Placeholder { start, len })
        }

        hume_rope::width::Cluster::Plain { width, .. } => {
            // Space and the invisible Unicode spaces (NBSP, ideographic
            // space) are gated by the same `space` render mode — NBSP/
            // ideographic space get a distinct glyph so a stray
            // non-breaking space stands out from an ordinary one.
            let content = if matches!(grapheme_str, " " | "\u{A0}" | "\u{3000}")
                && should_render_whitespace(whitespace.space, is_trailing)
            {
                let glyph = if grapheme_str == " " {
                    whitespace.space_char
                } else {
                    whitespace.nbsp_char
                };
                let (start, len) = push_arena_text(virtual_texts, glyph);
                CellContent::Whitespace { start, len }
            } else {
                // Regular grapheme (or a space-family one with the
                // indicator off).
                CellContent::Grapheme
            };
            (width as u8, content)
        }
    }
}

/// Returns `true` if a whitespace indicator should be rendered for this cell.
///
/// `is_trailing`: this grapheme's byte offset is at/after the start of the
/// line's trailing whitespace run (see `trailing_ws_start` in `format_buffer_line`).
fn should_render_whitespace(render: WhitespaceRender, is_trailing: bool) -> bool {
    match render {
        WhitespaceRender::None => false,
        WhitespaceRender::All => true,
        WhitespaceRender::Trailing => is_trailing,
    }
}
