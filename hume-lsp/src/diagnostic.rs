//! Diagnostic decoding: a pushed `textDocument/publishDiagnostics` payload
//! and a pulled `textDocument/diagnostic` report, each diagnostic parsed on
//! its own so one the server got wrong does not cost the file the others.

/// One diagnostic of a publish: the parsed form, and the wire value it was
/// parsed from. A diagnostic echoed back to a server (a code action's
/// context) is `raw`, so every field the server sent survives, including
/// ones `lsp_types` does not model.
#[derive(Debug)]
pub struct WireDiagnostic {
    pub diagnostic: lsp_types::Diagnostic,
    pub raw: serde_json::Value,
}

/// A `textDocument/publishDiagnostics` payload with each diagnostic paired
/// with its wire value.
#[derive(Debug)]
pub struct PublishedDiagnostics {
    pub uri: lsp_types::Uri,
    pub version: Option<i32>,
    pub diagnostics: Vec<WireDiagnostic>,
    /// The parse error of each diagnostic the server sent that is not in
    /// `diagnostics`.
    pub skipped: Vec<String>,
}

/// A `textDocument/diagnostic` report.
#[derive(Debug)]
pub enum PullReport {
    /// The server's diagnostics are the ones it last reported.
    Unchanged { result_id: Option<String> },
    /// The server's diagnostics for the document, replacing the last report.
    Full {
        result_id: Option<String>,
        diagnostics: Vec<WireDiagnostic>,
        /// The parse error of each item that is not in `diagnostics`.
        skipped: Vec<String>,
    },
}

/// A `publishDiagnostics` payload. `None`, with `params` untouched, when the
/// payload has no usable `uri`, `version` or `diagnostics` array, or when it
/// holds diagnostics and none parse: storing an empty list would read as the
/// server clearing the file.
pub(crate) fn classify_publish(params: &mut serde_json::Value) -> Option<PublishedDiagnostics> {
    use serde::Deserialize as _;

    let uri = lsp_types::Uri::deserialize(params.get("uri")?).ok()?;
    let version = match params.get("version") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(i32::deserialize(v).ok()?),
    };
    let (diagnostics, skipped) =
        parse_wire_diagnostics(params.get_mut("diagnostics")?.as_array_mut()?)?;
    Some(PublishedDiagnostics {
        uri,
        version,
        diagnostics,
        skipped,
    })
}

/// A `textDocument/diagnostic` answer. The error completes "the server sent
/// …" for the caller's report.
pub fn parse_pull_report(mut report: serde_json::Value) -> Result<PullReport, String> {
    let result_id = report
        .get("resultId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    match report.get("kind").and_then(serde_json::Value::as_str) {
        Some("unchanged") => Ok(PullReport::Unchanged { result_id }),
        Some("full") => {
            let items = report
                .get_mut("items")
                .and_then(serde_json::Value::as_array_mut)
                .ok_or("a diagnostics report without items")?;
            let (diagnostics, skipped) = parse_wire_diagnostics(items)
                .ok_or("a diagnostics report none of whose items parse")?;
            Ok(PullReport::Full {
                result_id,
                diagnostics,
                skipped,
            })
        }
        _ => Err("a diagnostics report of an unknown kind".to_owned()),
    }
}

/// Parses each wire diagnostic on its own, moving the values out of `wire`
/// into the result. Returns the diagnostics that parsed and the parse error
/// of each that did not. `None`, with `wire` untouched, when `wire` holds
/// diagnostics and none parse: storing an empty list would read as the
/// server clearing the file.
fn parse_wire_diagnostics(
    wire: &mut [serde_json::Value],
) -> Option<(Vec<WireDiagnostic>, Vec<String>)> {
    use serde::Deserialize as _;

    let parsed: Vec<Result<lsp_types::Diagnostic, String>> = wire
        .iter()
        .map(|value| lsp_types::Diagnostic::deserialize(value).map_err(|e| e.to_string()))
        .collect();
    if !parsed.is_empty() && parsed.iter().all(Result::is_err) {
        return None;
    }
    let mut diagnostics = Vec::with_capacity(parsed.len());
    let mut skipped = Vec::new();
    for (parsed, raw) in parsed.into_iter().zip(wire) {
        match parsed {
            Ok(diagnostic) => diagnostics.push(WireDiagnostic {
                diagnostic,
                raw: raw.take(),
            }),
            Err(error) => skipped.push(error),
        }
    }
    Some((diagnostics, skipped))
}

#[cfg(test)]
mod tests;
