//! Values the indent model hands the edits it drives: the indent a line
//! break computes, and the input an electric dedent answers.

use hume_rope::column::BufferLineCol;

/// A computed indent for a line break: the new line's width, and whether the
/// cluster before the break is a syntax token of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakIndent {
    pub width: BufferLineCol,
    pub after_token: bool,
}

/// The input an electric dedent answers, which decides when a line's first
/// token is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElectricTrigger {
    /// A character typed before the cursor: a token ending at the cursor
    /// may still grow.
    Typed,
    /// A line break about to go in at the cursor: it ends every token
    /// before it and cuts any token reaching past it.
    LineBreak,
}
