use std::any::Any;
use std::str::FromStr;

use crate::providers::{GutterCell, GutterCellContent, GutterColumn};
use crate::types::{RowKind, ScopeId};

// ---------------------------------------------------------------------------
// Line number style
// ---------------------------------------------------------------------------

/// How line numbers are displayed in the gutter.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LineNumberStyle {
    /// 1-based absolute line numbers.
    Absolute,
    /// Distance from the cursor line (0 at the cursor, counting outward).
    Relative,
    /// Absolute number on the cursor line, relative everywhere else.
    #[default]
    Hybrid,
}

impl LineNumberStyle {
    /// The wire-format strings `FromStr` accepts — the single source
    /// `:set buffer line-number-style=<Tab>` completion mirrors, so the two
    /// can never drift out of sync.
    pub const VALUES: &'static [&'static str] = &["absolute", "relative", "hybrid"];
}

impl FromStr for LineNumberStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "absolute" => Ok(LineNumberStyle::Absolute),
            "relative" => Ok(LineNumberStyle::Relative),
            "hybrid" => Ok(LineNumberStyle::Hybrid),
            _ => Err(format!(
                "invalid line-number-style '{s}': expected absolute, relative, or hybrid"
            )),
        }
    }
}

impl std::fmt::Display for LineNumberStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::Hybrid => "hybrid",
        })
    }
}

// ---------------------------------------------------------------------------
// LineNumberColumn
// ---------------------------------------------------------------------------

/// Built-in gutter column that renders line numbers.
///
/// Width is computed dynamically: `floor(log10(total_lines)) + 1` digits plus
/// one space of padding on the right.
pub struct LineNumberColumn {
    pub style: LineNumberStyle,
    /// Interned `"ui.linenr"` — every row but the primary head line.
    default_scope: ScopeId,
    /// Interned `"ui.linenr.selected"` — the primary selection's head line.
    selected_scope: ScopeId,
}

impl LineNumberColumn {
    /// `default_scope`/`selected_scope` are interned by the caller (once, at
    /// pane construction) — same intern-at-construction contract as every
    /// other provider (`DecorationSource`, `SignSource`).
    pub fn new(default_scope: ScopeId, selected_scope: ScopeId) -> Self {
        Self {
            style: LineNumberStyle::Hybrid,
            default_scope,
            selected_scope,
        }
    }

    pub fn with_style(
        style: LineNumberStyle,
        default_scope: ScopeId,
        selected_scope: ScopeId,
    ) -> Self {
        Self {
            style,
            default_scope,
            selected_scope,
        }
    }

    /// Number of digits needed to represent `total_lines`.
    fn digit_count(total_lines: usize) -> u8 {
        if total_lines == 0 {
            1
        } else {
            total_lines.ilog10() as u8 + 1
        }
    }
}

impl GutterColumn for LineNumberColumn {
    fn width(&self, last_line_idx: usize) -> u8 {
        // Digits needed to display the 1-based line number, plus 1 space of
        // right-padding. `last_line_idx` is the phantom-inclusive
        // `hume_rope::last_ropey_line` (see `layout.rs`/`Pane::content_width`
        // callers), so this sizes for `content_line_count() + 1` digits — one
        // wider than content strictly needs. The statusline's own row-digit
        // field (`ui/statusline/elements/position.rs`) instead sizes for
        // `content_line_count()` — an accidental, shipped divergence, not a
        // bug to fix here.
        Self::digit_count(last_line_idx + 1).saturating_add(1)
    }

    fn render_row_cells(
        &self,
        kind: RowKind,
        ctx: &crate::providers::GutterRowCtx,
    ) -> Vec<GutterCell> {
        let primary_head_line = ctx.primary_head_line;
        let cell = match kind {
            RowKind::Filler | RowKind::Virtual { .. } | RowKind::Wrap { .. } => {
                GutterCell::blank(self.default_scope)
            }
            RowKind::LineStart { line_idx } => {
                let scope = if line_idx == primary_head_line {
                    self.selected_scope
                } else {
                    self.default_scope
                };

                let display_num = match self.style {
                    LineNumberStyle::Absolute => line_idx + 1,
                    LineNumberStyle::Relative => {
                        (line_idx as isize - primary_head_line as isize).unsigned_abs()
                    }
                    LineNumberStyle::Hybrid => {
                        if line_idx == primary_head_line {
                            line_idx + 1 // absolute on the primary selection head line
                        } else {
                            (line_idx as isize - primary_head_line as isize).unsigned_abs()
                        }
                    }
                };

                GutterCell {
                    content: GutterCellContent::from_number(display_num),
                    scope,
                }
            }
        };
        vec![cell]
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
