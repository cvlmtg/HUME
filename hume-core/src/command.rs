//! What a native command is run with besides its pane: its count and its
//! operand.

/// Ceiling on a command's numeric count prefix (the `3` in `3w`), whether
/// the keyboard builds it one digit at a time or a script passes it whole.
/// Without one, a count can make a command loop with no fixed-point exit
/// (macro replay grows its queue by `count × macro length`) long enough to
/// hang the editor, and a typed count would overflow `usize` past about 20
/// digits.
pub const MAX_COUNT: usize = 10_000;

/// The operand a native command acts with: the character `f`, `r`, … act
/// on, or the pattern a prompt command searches for. A command declares
/// which kind it takes through its body's signature; the editor checks the
/// two match before the body runs, and reads a missing one from the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandArg {
    Char(char),
    Text(String),
}
