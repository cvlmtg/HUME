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

use ropey::Rope;

use crate::lines::last_ropey_line;

/// Wire-format position encoding negotiated with an LSP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// char offset → `(line, character)` in `enc` code units.
///
/// Total: a `char_idx` past `text.len_chars()` clamps to the document end
/// rather than panicking (ropey's own indexing functions panic past
/// `len_chars()`) — mirrors [`wire_to_char`]'s clamp-don't-error convention.
pub fn char_to_wire(text: &Rope, char_idx: usize, enc: PositionEncoding) -> (usize, usize) {
    let char_idx = char_idx.min(text.len_chars());
    let line = text.char_to_line(char_idx);
    let character = match enc {
        PositionEncoding::Utf8 => text.char_to_byte(char_idx) - text.line_to_byte(line),
        PositionEncoding::Utf16 => {
            let line_start = text.line_to_char(line);
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
    start_char: usize,
    end_char: usize,
    enc: PositionEncoding,
) -> ((usize, usize), (usize, usize)) {
    (
        char_to_wire(text, start_char, enc),
        char_to_wire(text, end_char, enc),
    )
}

/// `(line, character)` → char offset.
///
/// Out-of-range input clamps rather than errors — servers send past-end
/// positions routinely: `line` past EOF clamps to the last line; `character`
/// past the line's content clamps to the line's end. Clamping happens in
/// code-unit space (byte or UTF-16-code-unit) before converting back to a
/// char index, so a `character` that would split a multi-byte char or a
/// UTF-16 surrogate pair clamps down to that char's start rather than
/// landing mid-char — `Rope::byte_to_char`/`utf16_cu_to_char` guarantee this
/// for any in-bounds code-unit index, on-boundary or not.
pub fn wire_to_char(text: &Rope, line: usize, character: usize, enc: PositionEncoding) -> usize {
    let line = line.min(last_ropey_line(text));
    let line_start = text.line_to_char(line);
    let content_end = line_content_end_char(text, line);

    match enc {
        PositionEncoding::Utf8 => {
            let line_start_byte = text.line_to_byte(line);
            let content_end_byte = text.char_to_byte(content_end);
            let byte = (line_start_byte + character).min(content_end_byte);
            text.byte_to_char(byte)
        }
        PositionEncoding::Utf16 => {
            let line_start_utf16 = text.char_to_utf16_cu(line_start);
            let content_end_utf16 = text.char_to_utf16_cu(content_end);
            let cu = (line_start_utf16 + character).min(content_end_utf16);
            text.utf16_cu_to_char(cu)
        }
    }
}

/// A wire `(line, character)` range's two ends → `(start_char, end_char)`,
/// via [`wire_to_char`] on each end independently. Each end clamps on its
/// own (`wire_to_char`'s clamp-don't-error contract) — a reversed range
/// (`end` before `start`) is passed through unreordered; callers that must
/// reject one check `end < start` themselves. The inverse of
/// [`char_range_to_wire_range`].
pub fn wire_range_to_char_range(
    text: &Rope,
    start: (usize, usize),
    end: (usize, usize),
    enc: PositionEncoding,
) -> (usize, usize) {
    (
        wire_to_char(text, start.0, start.1, enc),
        wire_to_char(text, end.0, end.1, enc),
    )
}

/// The char offset one past the last content char on `line` — the position
/// of `line`'s terminator (`\n` or `\r\n`), or the line's own start when it
/// has none (the buffer's trailing empty line, guaranteed by the "buffer
/// always ends with `\n`" invariant).
///
/// This is the wire-domain "end of line": defined by content length, not by
/// where a cursor may land. `hume_editing::lines`' motion-domain helpers
/// (e.g. `line_content_end`) define the latter — including a position sitting
/// *on* the `\n` — and must not be reused here; the two disagree by design
/// for an empty line, and wire math needs this one.
fn line_content_end_char(text: &Rope, line: usize) -> usize {
    if line >= last_ropey_line(text) {
        return text.len_chars();
    }
    let next_start = text.line_to_char(line + 1);
    // A non-final line's span always contains at least its terminator, so
    // `next_start > line_to_char(line) >= 0` and this lookup is in-bounds.
    if next_start >= 2 && text.char(next_start - 1) == '\n' && text.char(next_start - 2) == '\r' {
        next_start - 2
    } else {
        next_start - 1
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
