//! Arguments that carry a server's wire values: a JSON handle tagged with
//! its server's position encoding, and the positions, ranges and text edits
//! read from one.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use super::errors::generic_err;

/// A wire-position argument: a [`JsonHandle`](crate::json::JsonHandle) onto
/// a server response, paired with the encoding that server negotiated. A
/// value a plugin built by hand has no producing server to have negotiated
/// one with, so it raises rather than being decoded under a guessed encoding.
pub(crate) fn tagged_json_arg(
    val: &SteelVal,
    ctx_name: &str,
) -> Result<
    (
        crate::json::JsonHandle,
        hume_rope::position_encoding::PositionEncoding,
    ),
    SteelErr,
> {
    let Some(handle) = crate::json::downcast_json_handle(val) else {
        steel::stop!(TypeMismatch => "{}: expected a JSON handle from a server response", ctx_name);
    };
    let encoding = handle.position_encoding(ctx_name).map_err(generic_err)?;
    Ok((handle, encoding))
}

/// Decodes a wire `{"line" "character"}` hashmap. The eventual conversion
/// (`hume_rope::position_encoding`) clamps a column or answers `None` for a
/// line rather than erroring on a shape, so this boundary check is the
/// only place a malformed shape gets caught instead of silently producing a
/// plausible-looking offset.
pub(crate) fn wire_position(
    v: &serde_json::Value,
) -> Result<hume_rope::position_encoding::WirePos, SteelErr> {
    hume_lsp::position::decode_wire_pos(v).map_err(generic_err)
}

/// Decodes a wire `{"start" ... "end" ...}` range into its two positions.
pub(crate) fn wire_range(
    v: &serde_json::Value,
) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>, SteelErr> {
    hume_lsp::position::decode_wire_range(v).map_err(generic_err)
}

/// Decodes a wire `{"range" {"start" ... "end" ...} "newText" ...}`
/// `TextEdit` (a `JsonHandle`'s own resolved value, e.g. one element of a
/// `textDocument/formatting` response handed straight to
/// `apply-text-edits!`) into a [`WireTextEdit`](crate::host::WireTextEdit).
/// `encoding` is the handle's own tag, passed in rather than re-read here so
/// this stays the plain JSON-shape decoder [`wire_position`] already is.
fn wire_text_edit_from_json(
    v: &serde_json::Value,
    encoding: hume_rope::position_encoding::PositionEncoding,
    ctx_name: &str,
) -> Result<crate::host::WireTextEdit, SteelErr> {
    let range = v
        .get("range")
        .ok_or_else(|| generic_err(format!("{ctx_name}: text edit handle has no 'range' field")))
        .and_then(wire_range)?;
    let new_text = v
        .get("newText")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            generic_err(format!(
                "{ctx_name}: text edit handle has no string 'newText' field"
            ))
        })?
        .to_string();
    Ok(crate::host::WireTextEdit {
        range,
        new_text,
        encoding,
    })
}

/// Decodes one `apply-text-edits!` entry into a
/// [`WireTextEdit`](crate::host::WireTextEdit): a `JsonHandle` onto a wire
/// `TextEdit` (an unconverted element straight from a
/// `textDocument/formatting`-shaped response), never a hand-built shape: a
/// hand-built tuple would have no producing server to read an encoding off,
/// and `JsonHandle::position_encoding` refuses to guess one.
pub(crate) fn wire_text_edit_arg(val: SteelVal) -> Result<crate::host::WireTextEdit, SteelErr> {
    let (handle, encoding) = tagged_json_arg(&val, "edits")?;
    wire_text_edit_from_json(handle.value(), encoding, "edits")
}

#[cfg(test)]
mod tests;
