//! `hume_rope::position_encoding::WirePos` ↔ the protocol's own wire
//! representations: `lsp_types::Range` and the raw JSON object shape.
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

#[cfg(test)]
mod tests;
