//! Protocol-only decode helpers a `textDocument/completion` item's fields
//! need: stripping snippet syntax HUME's v1 completion UI can't render, and
//! the lenient `TextEdit` fallback for off-spec servers. `StoredCompletionItem`
//! itself — the completion store's item type, ranked/filtered by
//! `CompletionSession` and rendered as a menu row — is not a wire type and
//! stays in `hume-editor/src/editor/lsp/completion/item/mod.rs`; only the
//! decode logic it needs lives here, alongside the sibling `location.rs`
//! wire decoder.

/// Rewrites `${n:default}` -> `default` (empty string if no `:default`) and
/// bare `$n` -> "" (dropped) in an `insertTextFormat: Snippet` item's text —
/// v1 has no snippet-expansion UI (no tabstop cycling), so inserting raw
/// snippet syntax verbatim would show it literally in the buffer. No
/// choices (`${n|a,b|}`), no nested placeholders, no `\$` escapes. Operates
/// on `char`s (Unicode scalars), matching how this logic worked when it was
/// Steel `string-ref`/`substring` — this is text-content transformation on
/// server-provided strings, not motion/selection code over buffer
/// positions, so grapheme-cluster stepping doesn't apply here.
pub fn strip_snippet(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < n {
        if chars[i] == '$' && i + 1 < n && chars[i + 1] == '{' {
            let close = chars[i + 2..]
                .iter()
                .position(|&c| c == '}')
                .map(|p| i + 2 + p);
            let body_end = close.unwrap_or(n);
            let body: String = chars[i + 2..body_end].iter().collect();
            if let Some(colon) = body.find(':') {
                out.push_str(&body[colon + 1..]);
            }
            i = close.map_or(n, |c| c + 1);
        } else if chars[i] == '$' && i + 1 < n && chars[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Extracts `(range, newText)` from a `CompletionTextEdit` JSON value —
/// either shape (`Edit`: `{"range", "newText"}`, or `InsertReplaceEdit`:
/// `{"insert", "replace", "newText"}`, using the narrower `insert` range).
/// Tolerates a malformed/partial shape by returning `None` — the caller
/// drops just the edit, not the whole item.
pub fn text_edit_from_json_lenient(v: &serde_json::Value) -> Option<lsp_types::TextEdit> {
    let range = v.get("range").or_else(|| v.get("insert"))?;
    let new_text = v.get("newText")?.as_str()?.to_string();
    Some(lsp_types::TextEdit {
        range: lsp_types::Range {
            start: crate::position::position_from_json(range.get("start")?)?,
            end: crate::position::position_from_json(range.get("end")?)?,
        },
        new_text,
    })
}

/// Lenient `additionalTextEdits` reader — shared by a completion item's own
/// off-spec fallback parse and a `completionItem/resolve` response handler
/// (which never goes through strict deserialize at all — a resolved item
/// that's otherwise off-spec shouldn't lose a well-formed edit list over an
/// unrelated malformed field elsewhere in the response).
pub fn parse_additional_text_edits_lenient(
    resolved: &serde_json::Value,
) -> Vec<lsp_types::TextEdit> {
    resolved
        .get("additionalTextEdits")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().filter_map(text_edit_from_json_lenient).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
