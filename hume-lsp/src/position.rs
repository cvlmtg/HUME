//! `hume_rope::position_encoding::WirePos` ↔ `lsp_types::Range`, plus
//! `WirePos` ↔ the protocol's raw JSON object shape, plus an `lsp_types::Range`
//! → a document's char offsets. [`wire_pos_from_json`] is the one decoder of
//! that shape here. [`position_from_json`] and [`range_from_json`] narrow it
//! into `lsp_types` for *lenient* callers (`completion_item`'s text edit and
//! default edit range) that want `None` on a malformed field;
//! [`decode_wire_pos`], [`decode_range_end`] and [`decode_wire_range`] are its
//! strict form, for callers that report what is wrong.
//!
//! `hume-rope` has no `lsp-types` dependency (see
//! `position_encoding`'s module doc), so the crossing lives here as free
//! functions rather than `impl From<WirePos> for lsp_types::Position`; the
//! orphan rule would refuse that impl in either direction anyway, since
//! neither type is local to this crate.

use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_rope::position_encoding::{PositionEncoding, WirePos};

/// A rope-derived wire range → `lsp_types::Range`, or `None` if `line`/
/// `character` on either end exceeds `u32`, the protocol's own width.
/// Every value this crate produces comes from a real document via
/// `hume_rope::position_encoding`, so overflow here means a corrupt or
/// astronomically large buffer, not a routine input to handle gracefully.
pub fn to_lsp_range(range: ExclusiveRange<WirePos>) -> Option<lsp_types::Range> {
    Some(lsp_types::Range {
        start: to_lsp_position(range.start)?,
        end: to_lsp_position(range.end)?,
    })
}

/// A wire position → `lsp_types::Position`, or `None` past `u32`.
fn to_lsp_position(pos: WirePos) -> Option<lsp_types::Position> {
    Some(lsp_types::Position {
        line: u32::try_from(pos.line).ok()?,
        character: u32::try_from(pos.character).ok()?,
    })
}

/// `lsp_types::Range` → a wire range. Infallible: `u32` always widens into
/// `usize`.
pub fn from_lsp_range(range: &lsp_types::Range) -> ExclusiveRange<WirePos> {
    fn from_position(pos: lsp_types::Position) -> WirePos {
        WirePos {
            line: pos.line as usize,
            character: pos.character as usize,
        }
    }
    ExclusiveRange::new(from_position(range.start), from_position(range.end))
}

/// A wire position as the protocol's own JSON object. No `u32` narrowing and
/// so no `Option`, unlike `to_lsp_range`: JSON numbers carry a `usize`
/// directly.
pub fn to_json_position(pos: WirePos) -> serde_json::Value {
    serde_json::json!({"line": pos.line, "character": pos.character})
}

/// A wire range as the protocol's `{"start", "end"}` object.
pub fn to_json_range(range: ExclusiveRange<WirePos>) -> serde_json::Value {
    serde_json::json!({
        "start": to_json_position(range.start),
        "end": to_json_position(range.end),
    })
}

/// The protocol's `{"line": N, "character": M}` object → a wire position.
/// `None` on a missing or non-numeric field.
pub fn wire_pos_from_json(v: &serde_json::Value) -> Option<WirePos> {
    let field = |key| usize::try_from(v.get(key)?.as_u64()?).ok();
    Some(WirePos {
        line: field("line")?,
        character: field("character")?,
    })
}

/// [`wire_pos_from_json`] for a strict caller. The error names what is
/// wrong, for the caller to prefix with its own name.
pub fn decode_wire_pos(v: &serde_json::Value) -> Result<WirePos, String> {
    wire_pos_from_json(v).ok_or_else(|| {
        format!("position must be an object with numeric 'line' and 'character' fields, got {v}")
    })
}

/// The `end` (`"start"` or `"end"`) position of the protocol's range object,
/// by [`decode_wire_pos`]'s rules.
pub fn decode_range_end(range: &serde_json::Value, end: &str) -> Result<WirePos, String> {
    let pos = range
        .get(end)
        .ok_or_else(|| format!("range has no '{end}' field"))?;
    decode_wire_pos(pos).map_err(|e| format!("range {end}: {e}"))
}

/// The protocol's `{"start": …, "end": …}` object, by [`decode_wire_pos`]'s
/// rules.
pub fn decode_wire_range(range: &serde_json::Value) -> Result<ExclusiveRange<WirePos>, String> {
    Ok(ExclusiveRange::new(
        decode_range_end(range, "start")?,
        decode_range_end(range, "end")?,
    ))
}

/// [`wire_pos_from_json`] as an `lsp_types::Position`: `None` too on an
/// out-of-`u32` field; a lenient caller's own fallback applies from there.
pub fn position_from_json(v: &serde_json::Value) -> Option<lsp_types::Position> {
    to_lsp_position(wire_pos_from_json(v)?)
}

/// The protocol's `{"start": …, "end": …}` object → `lsp_types::Range`, by
/// [`position_from_json`]'s lenient rules.
pub fn range_from_json(v: &serde_json::Value) -> Option<lsp_types::Range> {
    Some(lsp_types::Range {
        start: position_from_json(v.get("start")?)?,
        end: position_from_json(v.get("end")?)?,
    })
}

/// An `lsp_types::Range` → the char span it names in `rope`, by
/// [`hume_rope::position_encoding::wire_range_to_char_range`]'s clamping.
pub fn lsp_range_to_chars(
    rope: &ropey::Rope,
    range: &lsp_types::Range,
    encoding: PositionEncoding,
) -> ExclusiveRange<CharOffset> {
    hume_rope::position_encoding::wire_range_to_char_range(rope, from_lsp_range(range), encoding)
}

#[cfg(test)]
mod tests;
