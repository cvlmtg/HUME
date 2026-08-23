//! Decoding a `Location`/`LocationLink` wire object — the shape a
//! `textDocument/definition`-family or `textDocument/references` response
//! sends.

use std::str::FromStr;

/// The one position a `Location`/`LocationLink` wire object denotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireLocation {
    pub uri: lsp_types::Uri,
    pub line: usize,
    pub character: usize,
}

/// Decodes a raw `Location` (`{uri, range}`) or `LocationLink`
/// (`{targetUri, targetRange, targetSelectionRange?}`) wire object into the
/// one position it names. `goto-location!` (the jump) and
/// `lsp-locations->display-parts` (the drawer row) both decode through this
/// one function, so a location valid enough to display is exactly one valid
/// enough to jump to — see the `why` below.
///
/// # Why this errors instead of degrading
/// A drawer row exists only if it can be selected, and its `on-select` *is*
/// `goto-location!`. A location missing a `uri` or `range.start` names no
/// destination at all, so a row for it would be a place the user could
/// click that leads nowhere — that must abort loudly, not render silently
/// wrong. This is categorically different from a location whose destination
/// is real but whose *file* can't currently be read (deleted,
/// permission-denied, non-UTF-8): the jump still works there, and only the
/// displayed column is unknowable. Callers of this function make that
/// degradation themselves, downstream of a successful decode — see
/// `location_display_parts` in `hume-editor`.
///
/// `caller` names the builtin in the returned error, since more than one
/// builtin shares this decoder.
pub fn decode_location(loc: &serde_json::Value, caller: &str) -> Result<WireLocation, String> {
    let (uri, range) = if let Some(uri) = loc.get("targetUri") {
        (
            uri,
            loc.get("targetSelectionRange")
                .or_else(|| loc.get("targetRange")),
        )
    } else {
        (
            loc.get("uri")
                .ok_or_else(|| format!("{caller}: missing uri"))?,
            loc.get("range"),
        )
    };
    let uri = uri
        .as_str()
        .ok_or_else(|| format!("{caller}: uri must be a string"))?;
    let range = range.ok_or_else(|| format!("{caller}: missing range"))?;
    let line = range
        .pointer("/start/line")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{caller}: missing range.start.line"))? as usize;
    let character = range
        .pointer("/start/character")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{caller}: missing range.start.character"))?
        as usize;
    let uri = lsp_types::Uri::from_str(uri).map_err(|_| format!("{caller}: bad uri {uri:?}"))?;
    Ok(WireLocation {
        uri,
        line,
        character,
    })
}

#[cfg(test)]
mod tests;
