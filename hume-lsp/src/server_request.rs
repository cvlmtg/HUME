//! Answers to server-initiated requests that need no editor state.

use lsp_types::request::Request as _;

use crate::codec::ResponseError;

/// Answers a server-initiated request. Exhaustive by design: every request
/// gets exactly one response, even the ones this v1 doesn't otherwise
/// support. `workspace/applyEdit` is absent: it's the one
/// server request that needs `&mut Editor` (the edit engine), so the editor
/// glue answers it separately (`apply_edit_request_response`) rather than
/// through this pure lookup table.
///
/// `settings` is the server's configured blob (if any); the caller resolves
/// it from the registry keyed by server id, since this function has no
/// editor state of its own.
pub fn server_request_response(
    method: &str,
    params: &serde_json::Value,
    settings: Option<&serde_json::Value>,
) -> Result<serde_json::Value, ResponseError> {
    use lsp_types::request::{
        RegisterCapability, UnregisterCapability, WorkDoneProgressCreate, WorkspaceConfiguration,
    };
    match method {
        WorkspaceConfiguration::METHOD => Ok(workspace_configuration_response(params, settings)),
        // Acknowledged, no-op: these need no editor state to answer, unlike
        // `workspace/applyEdit` (the one request this lookup can't handle;
        // see `apply_edit_request_response`).
        RegisterCapability::METHOD
        | UnregisterCapability::METHOD
        | WorkDoneProgressCreate::METHOD => Ok(serde_json::Value::Null),
        other => Err(ResponseError {
            code: -32601,
            message: format!("method not found: {other}"),
            data: None,
        }),
    }
}

/// Resolves one requested item's `section` (e.g. `"a.b"`) against `settings`,
/// treating it as the root config object (VS Code semantics). No section (or
/// an empty one) returns the whole blob; a dotted path walks object keys;
/// any miss (missing key, or a non-object encountered mid-path) is `null`.
fn resolve_config_section(
    settings: &serde_json::Value,
    section: Option<&str>,
) -> serde_json::Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    let mut current = settings;
    for part in section.split('.') {
        match current.get(part) {
            Some(next) => current = next,
            None => return serde_json::Value::Null,
        }
    }
    current.clone()
}

/// One entry per requested item, same length and order as `params.items`,
/// per spec (the result array must line up positionally with the request).
/// With no settings blob, every item answers `null`, the same shape a server
/// sees from a client with no matching config.
fn workspace_configuration_response(
    params: &serde_json::Value,
    settings: Option<&serde_json::Value>,
) -> serde_json::Value {
    let items = params.get("items").and_then(|v| v.as_array());
    let Some(settings) = settings else {
        return serde_json::Value::Array(vec![
            serde_json::Value::Null;
            items.map_or(0, |v| v.len())
        ]);
    };
    let values = items
        .into_iter()
        .flatten()
        .map(|item| {
            let section = item.get("section").and_then(|s| s.as_str());
            resolve_config_section(settings, section)
        })
        .collect();
    serde_json::Value::Array(values)
}

#[cfg(test)]
mod tests;
