use ropey::Rope;

use crate::pane::ViewportState;
use crate::providers::GutterColumn;

// ---------------------------------------------------------------------------
// Pane geometry — output of Stage 1
// ---------------------------------------------------------------------------

/// The output of the Layout stage: the pane geometry the Format/Render stages
/// should use.
///
/// Which *rows* are visible is not decided here — `rows::RowMap` walks them
/// from the viewport's top address, so the walk is the layout and no estimate
/// of "how many lines fill the screen" is needed.
#[derive(Debug, Clone)]
pub struct PaneGeometry {
    /// Available display rows in the content area.
    pub content_height: u16,
    /// Available columns in the content area (viewport width − gutter width).
    pub content_width: u16,
    /// Total gutter width in columns.
    pub gutter_width: u16,
    /// 0-based index of the last buffer line (`rope.len_lines() - 1`).
    /// This is the correct value to pass to `GutterColumn::width()`.
    pub last_line_idx: usize,
}

// ---------------------------------------------------------------------------
// Stage 1: compute_viewport
// ---------------------------------------------------------------------------

/// Sum of all gutter column widths for the given `max_line` (0-based last line index).
///
/// Pass the last line index of the entire file so gutter width is stable across scrolling.
///
/// Takes an iterator (rather than a slice) so callers can feed it
/// `ProviderSet::gutter_columns()` directly — `ProviderSet` stores
/// `(ProviderId, Box<dyn GutterColumn>)` pairs internally (to support
/// `ProviderSet::remove`), which isn't a shape this purely-arithmetic
/// function needs to know about.
pub fn gutter_width_for_line<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    max_line: usize,
) -> u16 {
    gutter_columns.map(|c| c.width(max_line) as u16).sum()
}

/// Compute the `PaneGeometry` for a pane given its current state.
///
/// This is purely arithmetic — no heap allocations.
pub fn compute_viewport<'a>(
    rope: &Rope,
    viewport: &ViewportState,
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
) -> PaneGeometry {
    // 0-based index of the last line — the single source of truth for
    // GutterColumn::width(). Using the whole-file last line (not just what is
    // on screen) keeps gutter width stable as the user scrolls.
    let last_line_idx = rope.len_lines().saturating_sub(1);
    let gutter_width = gutter_width_for_line(gutter_columns, last_line_idx);

    PaneGeometry {
        content_height: viewport.height,
        content_width: viewport.width.saturating_sub(gutter_width).max(1),
        gutter_width,
        last_line_idx,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
