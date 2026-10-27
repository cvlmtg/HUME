//! Character classes the text crates agree on.

/// Whether `ch` is inline blank space: a space, a tab, a no-break space or an
/// ideographic space. The one definition of the blank chars a line's leading
/// whitespace, a word boundary and a comment token's edges treat as space.
pub fn is_space_char(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\u{a0}' | '\u{3000}')
}
