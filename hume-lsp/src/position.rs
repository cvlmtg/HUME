//! `hume_rope::position_encoding::WirePos` ↔ `lsp_types::Range`, plus
//! `WirePos` → the protocol's raw JSON object shape (outbound only — nothing
//! in this crate decodes JSON back into a `WirePos`). [`position_from_json`]
//! is the one inbound decoder here, and decodes into `lsp_types::Position`
//! rather than `WirePos`: it serves a *lenient* caller
//! (`completion_item::text_edit_from_json_lenient`) that wants `None` on a
//! malformed field, not the per-field error text `location::decode_location`
//! needs — that decoder reads the same JSON shape by hand instead of
//! sharing this one.
//!
//! `hume-rope` deliberately has no `lsp-types` dependency (see
//! `position_encoding`'s module doc), so the crossing lives here as free
//! functions rather than `impl From<WirePos> for lsp_types::Position` — the
//! orphan rule would refuse that impl in either direction anyway, since
//! neither type is local to this crate.

use hume_rope::offset::ExclusiveRange;
use hume_rope::position_encoding::WirePos;

/// A rope-derived wire range → `lsp_types::Range`, or `None` if `line`/
/// `character` on either end exceeds `u32` — the protocol's own width.
/// Every value this crate produces comes from a real document via
/// `hume_rope::position_encoding`, so overflow here means a corrupt or
/// astronomically large buffer, not a routine input to handle gracefully;
/// callers with a genuine "not our fault" source (untrusted plugin input)
/// turn `None` into their own error message instead of unwrapping.
pub fn to_lsp_range(range: ExclusiveRange<WirePos>) -> Option<lsp_types::Range> {
    fn to_position(pos: WirePos) -> Option<lsp_types::Position> {
        Some(lsp_types::Position {
            line: u32::try_from(pos.line).ok()?,
            character: u32::try_from(pos.character).ok()?,
        })
    }
    Some(lsp_types::Range {
        start: to_position(range.start)?,
        end: to_position(range.end)?,
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

/// The protocol's `{"line": N, "character": M}` object → `lsp_types::Position`.
/// `None` on a missing or non-numeric field — a lenient caller's own fallback
/// applies from there. See this module's doc for why `location::decode_location`
/// doesn't share this decoder.
pub fn position_from_json(v: &serde_json::Value) -> Option<lsp_types::Position> {
    Some(lsp_types::Position {
        line: v.get("line")?.as_u64()? as u32,
        character: v.get("character")?.as_u64()? as u32,
    })
}

#[cfg(test)]
mod tests;
