//! Rope char offset ↔ LSP wire `(line, character)` position conversion, in
//! both negotiated encodings (negotiate `utf-8`, fall back to UTF-16).
//! [`PositionEncoding`] mirrors the wire `Position`/encoding concept as a
//! plain type so this crate doesn't need to depend on `lsp-types`;
//! `hume-lsp` converts to/from `lsp_types::Position`.
//!
//! Wire math is **not** motion math: `character` counts code units in the
//! negotiated encoding, never a grapheme or a raw byte. The grapheme
//! helpers in [`crate::grapheme`] are the wrong tool in this module — do not
//! reach for them here (hub: testing playbook).

use std::ops::Range;

use ropey::{Rope, RopeSlice};

use crate::column::CharCol;
use crate::line::RopeyLine;
use crate::lines::line_terminator_start;
use crate::offset::{CharOffset, ExclusiveRange};

/// Wire-format position encoding negotiated with an LSP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// A raw LSP wire position: `line` is a 0-based line number, `character` a
/// code-unit column in the negotiated encoding — never a char or grapheme
/// index (see the module doc). Fields are `pub`, matching
/// [`crate::offset::ExclusiveRange`]'s own rationale: nothing here prevents
/// arithmetic misuse the way a private-field domain type does — this exists
/// purely so "which of the two wire numbers is which" is a name at the call
/// site, not a positional-tuple lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WirePos {
    pub line: usize,
    pub character: usize,
}

/// char offset → `(line, character)` in `enc` code units.
///
/// Total: a `char_idx` past `text.len_chars()` clamps to the document end
/// rather than panicking (ropey's own indexing functions panic past
/// `len_chars()`) — mirrors [`wire_to_char`]'s clamp-don't-error convention.
pub fn char_to_wire(text: &Rope, char_idx: CharOffset, enc: PositionEncoding) -> (usize, usize) {
    let char_idx = char_idx.index().min(text.len_chars());
    let line = crate::lines::char_to_ropey_line(text, CharOffset::new(char_idx)).index();
    let character = match enc {
        PositionEncoding::Utf8 => text.char_to_byte(char_idx) - text.line_to_byte(line),
        PositionEncoding::Utf16 => {
            let line_start = crate::lines::line_start_char(text, RopeyLine::new(line)).index();
            text.char_to_utf16_cu(char_idx) - text.char_to_utf16_cu(line_start)
        }
    };
    (line, character)
}

/// `[start_char, end_char)` → a wire `((line, character), (line, character))`
/// pair, via [`char_to_wire`] on each end. The inverse of
/// [`wire_range_to_char_range`].
pub fn char_range_to_wire_range(
    text: &Rope,
    range: ExclusiveRange<CharOffset>,
    enc: PositionEncoding,
) -> ((usize, usize), (usize, usize)) {
    (
        char_to_wire(text, range.start, enc),
        char_to_wire(text, range.end, enc),
    )
}

/// A flat wire code-unit offset into `text` → the char column it names.
///
/// The shared step of every wire→char conversion here: [`wire_to_char`]
/// applies it to one line's content, [`wire_offsets_to_byte_range`] to a
/// whole `&str`. Clamping lives here so both inherit one contract — an
/// offset past `text` lands at its end, and one that would split a
/// multi-byte char or a UTF-16 surrogate pair rounds *down* to that char's
/// start rather than landing mid-char (ropey's `byte_to_char` /
/// `utf16_cu_to_char` guarantee that for any in-bounds code-unit index,
/// on-boundary or not).
///
/// Returns [`CharCol`]: the result is always a line-relative char column —
/// `wire_to_line_char_col`'s second element directly, never a bare index to
/// re-wrap at the call site.
pub fn wire_offset_to_char(text: RopeSlice<'_>, offset: usize, enc: PositionEncoding) -> CharCol {
    CharCol::new(match enc {
        PositionEncoding::Utf8 => text.byte_to_char(offset.min(text.len_bytes())),
        PositionEncoding::Utf16 => text.utf16_cu_to_char(offset.min(text.len_utf16_cu())),
    })
}

/// `(line, character)` → `(clamped line, line-relative char column)`.
///
/// The wire-domain half of [`wire_to_char`], split out so a caller that
/// still needs to *place* the result — snap it to a grapheme boundary, land
/// it on the motion-domain line end rather than the wire-domain one — can
/// feed the column to `hume_editing::lines::place_char_column` instead of
/// treating the raw code-unit offset as a final cursor position. `line`
/// past EOF clamps to the last line here; `character` clamps to the line's
/// wire-domain content end because that is the extent of the slice handed
/// to [`wire_offset_to_char`], which owns the rest of the clamp contract.
pub fn wire_to_line_char_col(
    text: &Rope,
    pos: WirePos,
    enc: PositionEncoding,
) -> (RopeyLine, CharCol) {
    let line = RopeyLine::clamped(text, pos.line);
    let line_start = crate::lines::line_start_char(text, line).index();
    let content = text.slice(line_start..line_terminator_start(text, line).index());
    (line, wire_offset_to_char(content, pos.character, enc))
}

/// `(line, character)` → char offset.
///
/// Out-of-range input clamps rather than errors — servers send past-end
/// positions routinely. See [`wire_to_line_char_col`] for the clamp
/// contract; this just folds its `(line, column)` pair into one absolute
/// offset; a caller that must additionally land on a grapheme boundary
/// wants that function directly, not this one.
pub fn wire_to_char(text: &Rope, pos: WirePos, enc: PositionEncoding) -> CharOffset {
    let (line, char_col) = wire_to_line_char_col(text, pos, enc);
    // `line_start` advanced by a validated in-line column — not a raw
    // stepping hazard.
    crate::lines::line_start_char(text, line).shift(char_col.index() as isize)
}

/// A wire `(line, character)` range's two ends → `(start_char, end_char)`,
/// via [`wire_to_char`] on each end independently. Each end clamps on its
/// own (`wire_to_char`'s clamp-don't-error contract) — a reversed range
/// (`end` before `start`) is passed through unreordered; callers that must
/// reject one check `end < start` themselves. The inverse of
/// [`char_range_to_wire_range`].
pub fn wire_range_to_char_range(
    text: &Rope,
    start: WirePos,
    end: WirePos,
    enc: PositionEncoding,
) -> ExclusiveRange<CharOffset> {
    ExclusiveRange::new(wire_to_char(text, start, enc), wire_to_char(text, end, enc))
}

/// The byte range of `text` named by a `[start, end)` pair of flat wire
/// code-unit offsets — for offsets that index a server-authored string
/// rather than a document. `ParameterInformation.label`'s pair into its
/// `SignatureInformation.label` is the only such shape in the protocol.
///
/// Unlike [`wire_range_to_char_range`], which passes a reversed range
/// through for the caller to reject, this one orders its ends: the result
/// indexes a `&str` directly, and `&text[3..1]` panics rather than yielding
/// something a caller could inspect. A reversed pair gives an empty range.
///
/// `RopeSlice::from` borrows rather than copies — it counts `text` once to
/// fill in the slice's char and surrogate tallies, then points at the same
/// bytes — so reaching the shared kernel costs no rope allocation.
pub fn wire_offsets_to_byte_range(
    text: &str,
    start: usize,
    end: usize,
    enc: PositionEncoding,
) -> Range<usize> {
    let slice = RopeSlice::from(text);
    let start_byte = slice.char_to_byte(wire_offset_to_char(slice, start, enc).index());
    let end_byte = slice.char_to_byte(wire_offset_to_char(slice, end, enc).index());
    start_byte..end_byte.max(start_byte)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
