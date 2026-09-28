//! Rope-domain utilities shared across the HUME workspace.
//!
//! ## The trailing-newline invariant
//!
//! Every HUME buffer ends with a structural `\n`, so ropey reports one extra
//! empty "phantom" line past the real content. Line counts come in two
//! domains:
//!
//! - **Ropey domain** (`ropey_line_count`, `last_ropey_line`): phantom line
//!   included. Valid on any rope; for positions that must address every
//!   ropey line.
//! - **Content domain** (`content_line_count`, `last_content_line`): phantom
//!   line excluded. Assumes the invariant (debug-asserted); for user-facing
//!   counts and content bounds.
//!
//! ## LF is the only line break
//!
//! Ropey is compiled without `cr_lines` and `unicode_lines`, so lines split on
//! `\n` alone and `\r` (like VT, FF, NEL, LS, PS) is ordinary content. Line
//! terminator logic here is therefore single-char.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod column;
pub mod cursor;
pub mod grapheme;
pub mod line;
pub mod lines;
pub mod offset;
pub mod position_encoding;
pub mod width;

/// Test-only helpers shared by this crate's test submodules.
#[cfg(test)]
pub(crate) mod test_support {
    use ropey::Rope;

    /// Mirrors `hume_editing::text::BufferText::from`'s trailing-newline
    /// invariant, so the algorithms under test are exercised against the same
    /// buffer shape production code always hands them. Line-ending
    /// normalization is not mirrored: it changes no rope this crate's tests
    /// build, since `\n` is the only break here.
    pub(crate) fn rope(s: &str) -> Rope {
        if s.ends_with('\n') {
            Rope::from_str(s)
        } else {
            let mut r = Rope::from_str(s);
            r.insert_char(r.len_chars(), '\n');
            r
        }
    }
}
