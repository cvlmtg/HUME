//! Rope-domain utilities shared across the HUME workspace.
//!
//! ## The trailing-newline invariant
//!
//! Every HUME buffer (`hume_editing::Text`) always ends with a structural
//! `\n`. Ropey does not know this — it happily reports one extra empty line
//! past the buffer's real content (the "phantom" line). Two families of
//! functions in this crate answer "how many lines" / "which line is last":
//!
//! - **Ropey domain** (`ropey_line_count`, `last_ropey_line`,
//!   `ropey_lines_range`): the raw ropey count, phantom line included. Valid
//!   on any rope, invariant or not — this is what gutter sizing and LSP
//!   wire-position clamps want, since they must stay addressable up to
//!   ropey's own line indexing, not just the buffer's real content.
//! - **Content domain** (`content_line_count`, `last_content_line`,
//!   `content_lines_range`): the phantom line subtracted out. **Assumes the
//!   trailing-newline invariant** (debug-asserted) — this is what
//!   user-facing line counts and content-bounds checks want.
//!
//! All line ranges produced by this crate are end-exclusive `Range<usize>`.

mod cursor;
pub mod grapheme;
mod lines;
pub mod position_encoding;

pub use cursor::{CharCursor, chars_at};
pub use lines::{
    char_to_line_byte, content_line_count, content_lines_range, ends_with_newline, is_empty_line,
    last_content_line, last_ropey_line, leading_whitespace_end, line_break_char, line_content_end,
    line_end_exclusive, line_end_exclusive_byte, line_segments, place_column, ropey_line_count,
    ropey_lines_range, snap_to_grapheme_boundary, strip_line_break,
};

/// Test-only helpers shared by this crate's test submodules.
#[cfg(test)]
pub(crate) mod test_support {
    use ropey::Rope;

    /// Mirrors `hume_editing::text::Text::from`'s trailing-newline invariant
    /// (minus CRLF normalization, unneeded by these tests) so the algorithms
    /// under test are exercised against the same buffer shape production
    /// code always hands them.
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
