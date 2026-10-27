//! What a native command is run with besides its pane: its count and its
//! operand.

/// Ceiling on a command's numeric count prefix (the `3` in `3w`), whether
/// the keyboard builds it one digit at a time or a script passes it whole.
/// Without one, a count can make a command loop with no fixed-point exit
/// (macro replay grows its queue by `count × macro length`) long enough to
/// hang the editor, and a typed count would overflow `usize` past about 20
/// digits.
const MAX_COUNT: usize = 10_000;

/// A command's repeat count (the `3` in `3w`): one the user typed or a
/// script passed, clamped into `1..=MAX_COUNT`, or none, which a command
/// runs as one. Vertical moves read the difference: `9j` moves nine buffer
/// lines, a bare `j` one visual line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count(Option<usize>);

impl Count {
    /// No count was given.
    pub const UNTYPED: Self = Self(None);

    pub fn typed(n: usize) -> Self {
        Self(Some(n.clamp(1, MAX_COUNT)))
    }

    /// The count with `digit` typed after it. The held value is at most
    /// `MAX_COUNT`, so the product cannot overflow before the clamp.
    pub fn append_digit(self, digit: u32) -> Self {
        debug_assert!(digit < 10, "{digit} is not a decimal digit");
        Self::typed(self.0.unwrap_or(0) * 10 + digit as usize)
    }

    /// The repeat count, one when none was given.
    pub fn get(self) -> usize {
        self.0.unwrap_or(1)
    }

    pub fn is_typed(self) -> bool {
        self.0.is_some()
    }
}

/// The operand a native command acts with: the character `f`, `r`, … act
/// on, or the pattern a prompt command searches for. A command declares
/// which kind it takes through its body's signature; the editor checks the
/// two match before the body runs, and reads a missing one from the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandArg {
    Char(char),
    Text(String),
}

#[cfg(test)]
mod tests;
