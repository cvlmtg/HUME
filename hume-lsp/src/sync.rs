//! `ChangeSet` → `textDocument/didChange`'s incremental
//! `TextDocumentContentChangeEvent` list. The didChange envelope (document
//! version, URI) is the editor glue's job — this is pure text math.

use hume_editing::changeset::{ChangeSet, Operation};
use hume_rope::position_encoding::PositionEncoding;
use lsp_types::{Position, Range, TextDocumentContentChangeEvent};
use ropey::Rope;

/// `before` is the pre-edit text. Events are emitted in document order and,
/// per the LSP spec, each event's range addresses the document state AFTER
/// all previous events in the list were applied.
///
/// A `working` rope (cloned from `before` — O(1), ropey's tree nodes are
/// `Arc`-shared) mutates alongside a char `cursor` as ops are walked, so
/// every emitted range is computed against the correct intermediate state.
/// **The #1 bug this guards against**: computing every range against
/// `before` — ranges after the first event address the *partially updated*
/// document, not the original one.
pub fn changeset_to_content_changes(
    before: &Rope,
    cs: &ChangeSet,
    enc: PositionEncoding,
) -> Vec<TextDocumentContentChangeEvent> {
    let mut working = before.clone();
    let mut cursor = 0usize;
    let mut events = Vec::new();

    for op in cs.ops() {
        match op {
            Operation::Retain(n) => cursor += n,
            Operation::Delete(n) => {
                let range = wire_range(&working, cursor, cursor + n, enc);
                working.remove(cursor..cursor + n);
                events.push(TextDocumentContentChangeEvent {
                    range: Some(range),
                    range_length: None,
                    text: String::new(),
                });
            }
            Operation::Insert(s) => {
                let range = wire_range(&working, cursor, cursor, enc);
                working.insert(cursor, s);
                cursor += s.chars().count();
                events.push(TextDocumentContentChangeEvent {
                    range: Some(range),
                    range_length: None,
                    text: s.clone(),
                });
            }
        }
    }

    events
}

/// `[start, end)` in `rope`'s current state, converted to a wire `Range` via
/// [`hume_rope::position_encoding::char_range_to_wire_range`] — the one
/// lsp_types↔tuple adaptation point in this module, so `hume-rope` stays
/// free of an `lsp_types` dependency.
fn wire_range(rope: &Rope, start: usize, end: usize, enc: PositionEncoding) -> Range {
    let ((start_line, start_character), (end_line, end_character)) =
        hume_rope::position_encoding::char_range_to_wire_range(rope, start, end, enc);
    Range {
        start: Position {
            line: start_line as u32,
            character: start_character as u32,
        },
        end: Position {
            line: end_line as u32,
            character: end_character as u32,
        },
    }
}

/// Independent oracle: applies emitted events to a plain `String` using its
/// own line/character math — no ropey, no `hume_rope::position_encoding`
/// — so it cannot share a bug with `changeset_to_content_changes`. Exposed
/// (behind `test-util`) so consumer crates' invariant tests (e.g.
/// hume-editor's version-sync test) can reuse it instead of re-deriving
/// their own oracle.
#[cfg(any(test, feature = "test-util"))]
pub fn apply_events_to_string_mirror(
    mut text: String,
    events: &[TextDocumentContentChangeEvent],
    enc: PositionEncoding,
) -> String {
    for event in events {
        let range = event.range.expect("always emits ranged events");
        let start = wire_pos_to_byte(&text, range.start, enc);
        let end = wire_pos_to_byte(&text, range.end, enc);
        text.replace_range(start..end, &event.text);
    }
    text
}

/// `Buffer.text_gen` (a monotonic `u64` edit counter) -> the wire's `i32`
/// document version. `text_gen` would need over two billion edits to a
/// single buffer to overflow this — effectively unreachable — but a silent
/// wraparound would desync diagnostics/didChange version correlation in a
/// way that's very hard to diagnose, so this fails loudly instead of `as i32`.
pub fn wire_version(text_gen: u64) -> i32 {
    i32::try_from(text_gen).expect("text_gen overflowed i32 — over 2 billion edits to one buffer")
}

/// `(line, character)` → byte offset in `text`, via plain string scanning —
/// deliberately re-implemented rather than delegating to
/// `hume_rope::position_encoding` so this stays an independent oracle.
/// Splits on `\n` alone, matching the workspace's ropey config (neither
/// `cr_lines` nor `unicode_lines`): no other character terminates a line, in
/// a rope or in this oracle's own string mirror.
#[cfg(any(test, feature = "test-util"))]
fn wire_pos_to_byte(text: &str, pos: Position, enc: PositionEncoding) -> usize {
    let mut line_start = 0usize;
    for _ in 0..pos.line {
        match text[line_start..].find('\n') {
            Some(rel) => line_start += rel + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |rel| line_start + rel);
    let line = &text[line_start..line_end];

    let within_line = match enc {
        PositionEncoding::Utf8 => (pos.character as usize).min(line.len()),
        PositionEncoding::Utf16 => {
            let mut units = 0u32;
            let mut byte_off = 0usize;
            for ch in line.chars() {
                if units >= pos.character {
                    break;
                }
                units += ch.len_utf16() as u32;
                byte_off += ch.len_utf8();
            }
            byte_off
        }
    };
    line_start + within_line
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
