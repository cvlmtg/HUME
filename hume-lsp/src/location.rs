//! Decoding a `Location`/`LocationLink` wire object: the shape a
//! `textDocument/definition`-family or `textDocument/references` response
//! sends.

use std::str::FromStr;

use hume_rope::position_encoding::WirePos;

/// The one position a `Location`/`LocationLink` wire object denotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireLocation {
    pub uri: lsp_types::Uri,
    pub pos: WirePos,
}

/// Decodes a raw `Location` (`{uri, range}`) or `LocationLink`
/// (`{targetUri, targetRange, targetSelectionRange?}`) wire object into the
/// one position it names. `goto-location!` (the jump) and
/// `lsp-locations->places` (the drawer row) both decode through this
/// one function, so a location valid enough to display is exactly one valid
/// enough to jump to. See the `why` below.
///
/// # Why this errors instead of degrading
/// A drawer row exists only if it can be selected, and its `on-select` *is*
/// `goto-location!`. A location missing a `uri` or `range.start` names no
/// destination at all, so a row for it would be a place the user could
/// click that leads nowhere. That must abort loudly, not render silently
/// wrong. This is categorically different from a location whose destination
/// is real but whose *file* can't currently be read (deleted,
/// permission-denied, non-UTF-8): the jump still works there, and only the
/// displayed column is unknowable. Callers of this function make that
/// degradation themselves, downstream of a successful decode; see
/// `location_places` in `hume-editor`.
///
/// The error names what is wrong, not the builtin: each caller adds its own
/// name.
pub fn decode_location(loc: &serde_json::Value) -> Result<WireLocation, String> {
    let (uri, range) = if let Some(uri) = loc.get("targetUri") {
        (
            uri,
            loc.get("targetSelectionRange")
                .or_else(|| loc.get("targetRange")),
        )
    } else {
        (
            loc.get("uri").ok_or_else(|| "missing uri".to_string())?,
            loc.get("range"),
        )
    };
    let uri = uri
        .as_str()
        .ok_or_else(|| "uri must be a string".to_string())?;
    let range = range.ok_or_else(|| "missing range".to_string())?;
    let line = range
        .pointer("/start/line")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "missing range.start.line".to_string())?;
    let character = range
        .pointer("/start/character")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "missing range.start.character".to_string())?;
    let uri = lsp_types::Uri::from_str(uri).map_err(|_| format!("bad uri {uri:?}"))?;
    Ok(WireLocation {
        uri,
        pos: WirePos { line, character },
    })
}

#[cfg(test)]
mod tests;
