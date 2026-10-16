//! Rope char offset ↔ LSP wire `(line, character)` position conversion, in
//! both negotiated encodings (negotiate `utf-8`, fall back to UTF-16).
//! [`PositionEncoding`] mirrors the wire `Position`/encoding concept as a
//! plain type so this crate doesn't need to depend on `lsp-types`;
//! `hume-lsp` converts to/from `lsp_types::Position`.
//!
//! Wire math is **not** motion math: `character` counts code units in the
//! negotiated encoding, never a grapheme or a raw byte. The grapheme
//! helpers in [`crate::grapheme`] are the wrong tool in this module; do not
//! reach for them here (hub: testing playbook).

use std::ops::Range;

use ropey::{Rope, RopeSlice};

use crate::column::CharCol;
use crate::line::{ContentLine, RopeyLine};
use crate::lines::line_terminator_start;
use crate::offset::{CharOffset, ExclusiveRange};

/// Wire-format position encoding negotiated with an LSP server. `Utf32` is
/// LSP 3.17's own third `PositionEncodingKind`: `character` counts Unicode
/// scalar values (chars), never negotiated with a real server here, but the
/// right, spec-defined choice for a `textEdit` from a non-`#:resolve`
/// completion source (`hume-editor`'s `session/accept.rs`), which has no
/// wire encoding to honor at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
    Utf32,
}

/// A raw LSP wire position: `line` is a 0-based line number, `character` a
/// code-unit column in the negotiated encoding, never a char or grapheme
/// index (see the module doc). Fields are `pub`, matching
/// [`crate::offset::ExclusiveRange`]'s own rationale: nothing here prevents
/// arithmetic misuse the way a private-field domain type does. This exists
/// purely so "which of the two wire numbers is which" is a name at the call
/// site, not a positional-tuple lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WirePos {
    pub line: usize,
    pub character: usize,
}

/// char offset → a [`WirePos`] in `enc` code units.
///
/// Total: a `char_idx` past `text.len_chars()` clamps to the document end
/// rather than panicking (ropey's own indexing functions panic past
/// `len_chars()`). Mirrors [`wire_range_to_char_range`]'s clamp-don't-error
/// convention.
pub fn char_to_wire(text: &Rope, char_idx: CharOffset, enc: PositionEncoding) -> WirePos {
    let char_idx = char_idx.index().min(text.len_chars());
    let line = crate::lines::char_to_ropey_line(text, CharOffset::new(char_idx)).index();
    let character = match enc {
        PositionEncoding::Utf8 => text.char_to_byte(char_idx) - text.line_to_byte(line),
        PositionEncoding::Utf16 => {
            let line_start = crate::lines::line_start_char(text, RopeyLine::new(line)).index();
            text.char_to_utf16_cu(char_idx) - text.char_to_utf16_cu(line_start)
        }
        // `character` already *is* a char index: no code-unit conversion,
        // unlike the two wire encodings above.
        PositionEncoding::Utf32 => {
            char_idx - crate::lines::line_start_char(text, RopeyLine::new(line)).index()
        }
    };
    WirePos { line, character }
}

/// `[start_char, end_char)` → a wire `[start, end)` [`WirePos`] range, via
/// [`char_to_wire`] on each end. The inverse of [`wire_range_to_char_range`].
pub fn char_range_to_wire_range(
    text: &Rope,
    range: ExclusiveRange<CharOffset>,
    enc: PositionEncoding,
) -> ExclusiveRange<WirePos> {
    ExclusiveRange::new(
        char_to_wire(text, range.start, enc),
        char_to_wire(text, range.end, enc),
    )
}

/// A flat wire code-unit offset into `text` → the char index it names within
/// `text` itself.
///
/// The shared step of every wire→char conversion here: `line_char_col`
/// applies it to one line's content (where the result *is* a line-relative
/// char column; it wraps this in [`CharCol`] itself, the one place that's
/// true), [`wire_offsets_to_byte_range`] to a whole server-authored `&str`
/// (where it isn't). Domain-free like `hume_rope::width`'s primitives, for
/// the same reason: this function has no way to know which domain `text`
/// is in, so it returns a bare index and leaves wrapping to whichever caller
/// actually knows.
///
/// Clamping lives here so both callers inherit one contract: an offset past
/// `text` lands at its end, and one that would split a multi-byte char or a
/// UTF-16 surrogate pair rounds *down* to that char's start rather than
/// landing mid-char (ropey's `byte_to_char` / `utf16_cu_to_char` guarantee
/// that for any in-bounds code-unit index, on-boundary or not).
fn wire_offset_to_char_index(text: RopeSlice<'_>, offset: usize, enc: PositionEncoding) -> usize {
    match enc {
        PositionEncoding::Utf8 => text.byte_to_char(offset.min(text.len_bytes())),
        PositionEncoding::Utf16 => text.utf16_cu_to_char(offset.min(text.len_utf16_cu())),
        // `offset` already *is* a char index into `text`: clamp, no
        // conversion.
        PositionEncoding::Utf32 => offset.min(text.len_chars()),
    }
}

/// `(line, character)` → `(clamped line, line-relative char column)`: the
/// wire-domain half of [`wire_to_char`]. `line` past EOF clamps to the last
/// line here; `character` clamps to the line's wire-domain content end
/// because that is the extent of the slice handed to
/// `wire_offset_to_char_index`, which owns the rest of the clamp contract.
fn wire_to_line_char_col(text: &Rope, pos: WirePos, enc: PositionEncoding) -> (RopeyLine, CharCol) {
    let line = RopeyLine::clamped(text, pos.line);
    (line, line_char_col(text, line, pos.character, enc))
}

/// `(line, character)` → `(content line, line-relative char column)` for a
/// point that must name a character: `None` when `pos` is on the empty line
/// after the final line break or beyond it, where
/// [`checked_wire_range_to_char_range`] lets a range end sit on that empty
/// line. A column past its line's end clamps to the line's content end.
/// A caller that still has to *place* the result (snap it to a grapheme
/// boundary, land it on the motion-domain line end) feeds the column to
/// `ColumnView::place_char` rather than taking a raw offset.
pub fn checked_wire_to_line_char_col(
    text: &Rope,
    pos: WirePos,
    enc: PositionEncoding,
) -> Option<(ContentLine, CharCol)> {
    let line = ContentLine::checked(text, pos.line)?;
    Some((line, line_char_col(text, line.into(), pos.character, enc)))
}

/// `character`, in `enc` code units, as a char column of `line`'s content.
fn line_char_col(text: &Rope, line: RopeyLine, character: usize, enc: PositionEncoding) -> CharCol {
    let line_start = crate::lines::line_start_char(text, line).index();
    let content = text.slice(line_start..line_terminator_start(text, line).index());
    CharCol::new(wire_offset_to_char_index(content, character, enc))
}

/// `(line, character)` → char offset.
///
/// Out-of-range input clamps rather than errors: servers send past-end
/// positions routinely. See [`wire_to_line_char_col`] for the clamp
/// contract; this just folds its `(line, column)` pair into one absolute
/// offset.
fn wire_to_char(text: &Rope, pos: WirePos, enc: PositionEncoding) -> CharOffset {
    let (line, char_col) = wire_to_line_char_col(text, pos, enc);
    line_col_to_char(text, line, char_col)
}

/// [`checked_wire_to_line_char_col`]'s pair as one char offset: `None` where
/// that function is.
pub fn checked_wire_to_char(
    text: &Rope,
    pos: WirePos,
    enc: PositionEncoding,
) -> Option<CharOffset> {
    let (line, char_col) = checked_wire_to_line_char_col(text, pos, enc)?;
    Some(line_col_to_char(text, line.into(), char_col))
}

/// `line`'s start advanced by `char_col`, a column within the line's content,
/// so the step stays inside the line.
fn line_col_to_char(text: &Rope, line: RopeyLine, char_col: CharCol) -> CharOffset {
    crate::lines::line_start_char(text, line).shift(char_col.index() as isize)
}

/// A wire `[start, end)` range's two ends → `[start_char, end_char)`, each
/// end converted on its own. Out-of-range input clamps rather than errors,
/// since servers send past-end positions routinely: a line past EOF clamps
/// to the last line and a column to its line's content end. A reversed range
/// (`end` before `start`) is passed through unreordered; callers that must
/// reject one check `end < start` themselves. The inverse of
/// [`char_range_to_wire_range`].
pub fn wire_range_to_char_range(
    text: &Rope,
    range: ExclusiveRange<WirePos>,
    enc: PositionEncoding,
) -> ExclusiveRange<CharOffset> {
    ExclusiveRange::new(
        wire_to_char(text, range.start, enc),
        wire_to_char(text, range.end, enc),
    )
}

/// [`wire_range_to_char_range`] for a range that must name text: `None` when
/// either end is on a line past the document end. The document end is the
/// position `(line count, 0)` after the final line break, so each end may sit
/// on the empty line there (its column clamps like any other); a line beyond
/// it names nothing a clamp could honestly stand in for.
pub fn checked_wire_range_to_char_range(
    text: &Rope,
    range: ExclusiveRange<WirePos>,
    enc: PositionEncoding,
) -> Option<ExclusiveRange<CharOffset>> {
    let last_line = crate::lines::last_ropey_line(text).index();
    if range.start.line > last_line || range.end.line > last_line {
        return None;
    }
    Some(wire_range_to_char_range(text, range, enc))
}

/// The byte range of `text` named by a `[start, end)` pair of flat wire
/// code-unit offsets, for offsets that index a server-authored string
/// rather than a document. `ParameterInformation.label`'s pair into its
/// `SignatureInformation.label` is the only such shape in the protocol.
///
/// Unlike [`wire_range_to_char_range`], which passes a reversed range
/// through for the caller to reject, this one orders its ends: the result
/// indexes a `&str` directly, and `&text[3..1]` panics rather than yielding
/// something a caller could inspect. A reversed pair gives an empty range.
///
/// `RopeSlice::from` borrows rather than copies: it counts `text` once to
/// fill in the slice's char and surrogate tallies, then points at the same
/// bytes, so reaching the shared kernel costs no rope allocation.
pub fn wire_offsets_to_byte_range(
    text: &str,
    start: usize,
    end: usize,
    enc: PositionEncoding,
) -> Range<usize> {
    let slice = RopeSlice::from(text);
    let start_byte = slice.char_to_byte(wire_offset_to_char_index(slice, start, enc));
    let end_byte = slice.char_to_byte(wire_offset_to_char_index(slice, end, enc));
    start_byte..end_byte.max(start_byte)
}

#[cfg(test)]
mod tests;
