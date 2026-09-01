//! Rope-domain utilities shared across the HUME workspace.
//!
//! ## The trailing-newline invariant
//!
//! Every HUME buffer (`hume_editing::BufferText`) always ends with a structural
//! `\n`. Ropey does not know this — it happily reports one extra empty line
//! past the buffer's real content (the "phantom" line). Two families of
//! functions in this crate answer "how many lines" / "which line is last":
//!
//! - **Ropey domain** (`ropey_line_count`, `last_ropey_line`,
//!   `ropey_lines_range`): the raw ropey count, phantom line included. Valid
//!   on any rope, invariant or not — this is what gutter sizing and LSP
//!   wire-position clamps want, since they must stay addressable up to
//!   ropey's own line indexing, not just the buffer's real content. Those
//!   callers want a bound or a single index; the range is for a whole-buffer
//!   walk, which only whole-document code does (see its own doc).
//! - **Content domain** (`content_line_count`, `last_content_line`,
//!   `content_lines_range`): the phantom line subtracted out. **Assumes the
//!   trailing-newline invariant** (debug-asserted) — this is what
//!   user-facing line counts and content-bounds checks want.
//!
//! All line ranges produced by this crate are end-exclusive `Range<usize>`.
//!
//! ## LF is the only line break
//!
//! This workspace compiles ropey with neither `cr_lines` nor `unicode_lines`,
//! so `Rope::lines()` splits on `\n` alone. A `\r` — like VT, FF, NEL, LS and
//! PS — is ordinary content that never terminates a line, whatever rope it
//! reaches this crate in. Line-terminator logic here is correspondingly
//! single-char: there is no two-char terminator to look behind for, and no
//! break set to test membership in.

pub mod cursor;
pub mod grapheme;
pub mod lines;
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
