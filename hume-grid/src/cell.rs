use crate::style::ResolvedStyle;
use compact_str::CompactString;

/// One cell of the frame: the text drawn in it, its style, and how many
/// columns it advances the cursor.
///
/// `advance` is stored, never recomputed — see the crate doc. It is the
/// writer's own measurement, carried forward so the diff and the emitter
/// consume the same number the text was laid out with.
///
/// A double-width glyph is stored as a *head* (the text, `advance == 2`)
/// followed by a *continuation*: no text, `advance == 0`, and **the head's
/// style**. Carrying the style rather than leaving it default is what makes
/// cell equality sufficient for the diff — a continuation then differs from
/// the previous frame exactly when its head does, so a changed glyph can
/// never be detected at its second column alone, where a repaint would start
/// mid-glyph. [`Grid`](crate::Grid) is what guarantees the pairing.
///
/// `text` is a [`CompactString`]: a grapheme cluster is almost always a few
/// bytes and fits inline, so the common case costs no allocation, while a
/// pathological cluster (a long ZWJ emoji sequence) still stores correctly
/// rather than being truncated to fit a fixed array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    text: CompactString,
    style: ResolvedStyle,
    advance: u8,
}

impl Cell {
    /// A cell holding `text`, advancing `advance` columns.
    ///
    /// `advance` is clamped to at least 1: a cluster measuring zero columns
    /// has already been substituted for a visible placeholder by the caller's
    /// width model before it reaches a grid, so a zero here is a clamp
    /// against a caller bug, not a policy about zero-width text.
    pub fn glyph(text: &str, advance: u8, style: ResolvedStyle) -> Cell {
        Cell {
            text: CompactString::new(text),
            style: style.normalized(),
            advance: advance.max(1),
        }
    }

    /// A single blank column.
    pub fn blank(style: ResolvedStyle) -> Cell {
        Cell {
            text: CompactString::const_new(" "),
            style: style.normalized(),
            advance: 1,
        }
    }

    /// The second column of a double-width glyph. Carries `style` — the
    /// head's — for the reason in this type's doc.
    pub fn continuation(style: ResolvedStyle) -> Cell {
        Cell {
            text: CompactString::const_new(""),
            style: style.normalized(),
            advance: 0,
        }
    }

    /// The text to emit for this cell. Empty for a continuation, which the
    /// emitter must not write: the terminal already moved past that column
    /// when it drew the head.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> ResolvedStyle {
        self.style
    }

    /// Columns the cursor moves past this cell — 0 for a continuation.
    pub fn advance(&self) -> u16 {
        self.advance as u16
    }

    pub fn is_continuation(&self) -> bool {
        self.advance == 0
    }
}

impl Default for Cell {
    /// A blank cell in the terminal's own colours — what a grid holds before
    /// anything is drawn into it, and what it is reset to between frames.
    fn default() -> Cell {
        Cell::blank(ResolvedStyle::default())
    }
}

#[cfg(test)]
mod tests;
