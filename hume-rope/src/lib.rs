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

pub mod cursor;
pub mod grapheme;
pub mod lines;
pub mod position_encoding;
pub mod width;

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
