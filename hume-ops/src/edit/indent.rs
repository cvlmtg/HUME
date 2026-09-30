//! `>` / `<`: shift every line touched by a selection by whole indent levels.

use hume_editing::changeset::Assoc;
use hume_editing::edit::Edited;
use hume_editing::edit::{Landing, Landings, edit};
use hume_editing::lines::leading_indent;
use hume_editing::state::EditState;
use hume_editing::tab_style::TabStyle;
use hume_rope::offset::ExclusiveRange;
use hume_rope::width::indent_stop;

/// Indent every line touched by a selection by `levels` indent levels (`>`).
pub fn indent_lines(state: EditState, style: TabStyle, tab_width: u8, levels: usize) -> Edited {
    let delta_display_col = indent_stop(clamp_levels(levels, tab_width), tab_width) as isize;
    shift_indent(state, style, tab_width, delta_display_col)
}

/// Unindent every line touched by a selection by `levels` indent levels (`<`).
pub fn unindent_lines(state: EditState, style: TabStyle, tab_width: u8, levels: usize) -> Edited {
    let delta_display_col = -(indent_stop(clamp_levels(levels, tab_width), tab_width) as isize);
    shift_indent(state, style, tab_width, delta_display_col)
}

/// The widest indent an indent command writes: a terminal is at most
/// `u16::MAX` columns wide, so a wider indent could never be seen.
const MAX_INDENT_WIDTH: u32 = u16::MAX as u32;

/// Clamp `levels` so one shift moves an indent by at most
/// [`MAX_INDENT_WIDTH`] columns: an unbounded count would build an indent
/// string as large as the count.
fn clamp_levels(levels: usize, tab_width: u8) -> u32 {
    let tw = u32::from(tab_width).max(1);
    u32::try_from(levels).map_or(MAX_INDENT_WIDTH / tw, |levels| {
        levels.min(MAX_INDENT_WIDTH / tw)
    })
}

/// Render a leading-whitespace run of exactly `width` display columns in
/// `style`. Lives here rather than beside `hume_rope::width`'s `indent_depth`/
/// `indent_stop` because it needs `TabStyle`, which sits in `hume-editing`,
/// above `hume-rope` in the dependency graph.
fn render_indent(width: usize, style: TabStyle, tab_width: u8) -> String {
    match style {
        TabStyle::Soft => " ".repeat(width),
        TabStyle::Hard => {
            let tw = (tab_width as usize).max(1);
            std::iter::repeat_n('\t', width / tw)
                .chain(std::iter::repeat_n(' ', width % tw))
                .collect()
        }
    }
}

/// One signed display-column delta (positive indents, negative unindents),
/// since the two are otherwise identical. Callers pass columns (via
/// [`indent_stop`]) rather than levels, so this function never re-derives
/// "how many columns is a level" itself.
///
/// **Width-preserving, not level-snapping**: each touched line's indent
/// display-width shifts by `delta_display_col`, then that exact width is
/// re-rendered in `style`. An indent that isn't already a whole number of
/// levels (e.g. a continuation line hand-aligned to an open paren) shifts by
/// the requested amount without being rounded onto a tab stop first. This
/// also makes a `<` immediately after a `>` restore the exact prior width
/// (Vim's default, no `shiftround`); snapping to levels first would make
/// that round trip lossy. Not a full inverse in general, though: `new_width`
/// saturates at 0 (see below), so `<` on an indent narrower than one level
/// flattens it rather than going negative, and re-rendering in `style` means
/// a mixed tabs-and-spaces indent normalizes rather than surviving
/// byte-for-byte.
///
/// Iterates lines directly rather than going through [`super::apply_edit`]
/// (built for one edit per *selection*, not per *line*), the same reason
/// `sort_lines` drives its builder by hand.
fn shift_indent(
    state: EditState,
    style: TabStyle,
    tab_width: u8,
    delta_display_col: isize,
) -> Edited {
    let view = state.view();
    // Every distinct line touched by any selection, ascending: selections
    // are in document order and non-overlapping, so a consecutive dedup is
    // enough (mirrors `sort::collect_entries`).
    let mut lines: Vec<usize> = view
        .iter()
        .flat_map(|sel| {
            let lines = sel.lines();
            lines.start.index()..=lines.end.index()
        })
        .collect();
    lines.dedup();

    let text = state.text();
    // (line start, end of its indent, the indent it gets), for every line
    // whose indent changes.
    let mut rewrites = Vec::new();

    // `lines` stays bare `usize` rather than `Vec<ContentLine>`. CLAUDE.md's
    // "Line counts and ranges" sanctions this exact shape (a bare-usize loop
    // bounded by typed endpoints, re-minted immediately below): every value
    // here came from `.index()` on an already-valid `ContentLine` one line
    // up, so the mint can't fail.
    for line_idx in lines {
        let line = hume_rope::line::ContentLine::new(line_idx);
        let line_start = text.line_to_char(line.into());
        let (ws_end, old_width) = leading_indent(text, line, tab_width);
        // Blank line (empty, or whitespace-only): skipped untouched, matching
        // Vim's `>>`, so a blank separator line never collects trailing
        // whitespace.
        if text.char_at(ws_end.offset()) == Some('\n') {
            continue;
        }
        let new_width = old_width.shift_saturating(delta_display_col);
        if new_width == old_width {
            // Reachable at `delta_display_col == 0` (a `levels == 0` call, never
            // issued by the editor's own count dispatch, but this crate's ops
            // are a public API), or via the saturating clamp when unindenting
            // an already-flush line past width 0.
            continue;
        }
        let new_indent = render_indent(new_width.get() as usize, style, tab_width);
        rewrites.push((line_start, ws_end.offset(), new_indent));
    }

    if rewrites.is_empty() {
        // Every touched line kept its width (all blank, or a `<` saturating
        // at an already-flush indent).
        return Edited::unchanged(state);
    }

    let primary = view.primary().index();
    edit(&state, |b| {
        for (line_start, ws_end, new_indent) in &rewrites {
            // Insert before delete: the new indent's insertion then sits at
            // the line's start, so a selection end there resolves by its
            // `Assoc` instead of collapsing into the deletion that follows.
            b.insert(*line_start, new_indent);
            b.delete(ExclusiveRange::new(*line_start, *ws_end));
        }
        // A linewise selection's start is a rewritten line's start and stays
        // there: `Assoc::Before` sticks to the left of the new indent. Every
        // other end (a cursor that happens to sit at column 0, or one inside
        // the replaced indent) moves past it: `Assoc::After` passes the
        // insertion, and a position inside the replaced indent collapses onto
        // the same point.
        let landings = view
            .iter()
            .map(|sel| {
                let assoc = if sel.is_linewise() {
                    Assoc::Before
                } else {
                    Assoc::After
                };
                Landing::kept_with(sel.selection(), assoc)
            })
            .collect();
        Landings::new(landings, primary)
    })
}
