//! One completion item, typed via `lsp_types::CompletionItem`, plus the
//! lenient JSON fallback for off-spec servers.

/// One item, typed via `lsp_types::CompletionItem`. `insert_text`/`text_edit`
/// have snippet syntax (`${n:default}`, `$n`) already stripped when the
/// server declared `insertTextFormat: Snippet` — see [`strip_snippet`].
/// `raw` keeps the pristine, unstripped JSON (Steel's `on-completion-accept`
/// hook and `completionItem/resolve` both see the server's original text).
pub(crate) struct StoredCompletionItem {
    pub(crate) label: String,
    /// Raw `CompletionItemKind` number — display-only (icon choice), no
    /// v1 reader maps it to a name. Read straight from JSON rather than the
    /// typed field: `CompletionItemKind` wraps a private `i32` with no
    /// accessor.
    pub(crate) kind: Option<i64>,
    pub(crate) detail: Option<String>,
    pub(super) sort_text: String,
    pub(super) filter_text: String,
    pub(super) insert_text: String,
    pub(super) text_edit: Option<lsp_types::TextEdit>,
    pub(super) additional_text_edits: Vec<lsp_types::TextEdit>,
    /// Distinguishes "server sent no `additionalTextEdits` key at all" from
    /// "server sent an empty array" — an empty array still means "nothing
    /// more to apply *and* don't bother resolving", same as a present-but-
    /// empty list; only the key's absence means resolve might have more to
    /// offer. See `CompletionSession::accept`'s resolve gate.
    pub(super) has_additional_text_edits: bool,
    /// The full response item, unparsed — handed to `on-completion-accept`
    /// so Steel can read `data` or any other field this store doesn't
    /// parse, without Rust needing to grow a reader for every LSP field a
    /// feature might eventually want. Deliberately the *pristine* item
    /// (snippet syntax included) — Steel/resolve should see exactly what
    /// the server sent, not this store's stripped/narrowed projection.
    pub(super) raw: serde_json::Value,
}

impl StoredCompletionItem {
    /// Parses one item, strict first: `v` itself is never consumed, so
    /// `raw: v.clone()` (below) still captures the full item, including
    /// fields this projection drops. A strict deserialize into
    /// `lsp_types::CompletionItem` rejects on *any* off-spec field (an
    /// out-of-range `kind`, a malformed `textEdit`, ...), not just the ones
    /// this store reads — [`Self::from_json_lenient`] then recovers what it
    /// can straight from JSON. `Err` only when even that fails (`label`
    /// itself missing/non-string); callers skip the item and report a Trace
    /// line rather than fabricating a placeholder.
    pub(crate) fn from_json(v: &serde_json::Value) -> Result<Self, serde_json::Error> {
        match serde_json::from_value::<lsp_types::CompletionItem>(v.clone()) {
            Ok(item) => Ok(Self::from_typed(item, v)),
            Err(strict_err) => Self::from_json_lenient(v).ok_or(strict_err),
        }
    }

    /// Builds from an already-typed item — the common case, when the whole
    /// response round-trips through strict deserialize.
    fn from_typed(item: lsp_types::CompletionItem, v: &serde_json::Value) -> Self {
        let label = item.label;
        let kind = v.get("kind").and_then(|x| x.as_i64());
        let sort_text = item.sort_text.unwrap_or_else(|| label.clone());
        let filter_text = item.filter_text.unwrap_or_else(|| label.clone());
        let is_snippet = item.insert_text_format == Some(lsp_types::InsertTextFormat::SNIPPET);
        let insert_text = item.insert_text.unwrap_or_else(|| label.clone());
        let insert_text = if is_snippet {
            strip_snippet(&insert_text)
        } else {
            insert_text
        };
        let text_edit = item.text_edit.map(|te| match te {
            lsp_types::CompletionTextEdit::Edit(te) => te,
            // Preserves the existing "use the narrower insert range" choice.
            lsp_types::CompletionTextEdit::InsertAndReplace(ire) => lsp_types::TextEdit {
                range: ire.insert,
                new_text: ire.new_text,
            },
        });
        let text_edit = text_edit.map(|te| {
            if is_snippet {
                lsp_types::TextEdit {
                    new_text: strip_snippet(&te.new_text),
                    ..te
                }
            } else {
                te
            }
        });
        // `Option<Vec<T>>` fields deserialize key-absent -> `None` (serde's
        // built-in special case for `Option`, no `#[serde(default)]`
        // needed), so `is_some()` here really does mean "the server sent
        // this key" — not "the server sent a non-empty array".
        let has_additional_text_edits = item.additional_text_edits.is_some();
        let additional_text_edits = item.additional_text_edits.unwrap_or_default();
        Self {
            label,
            kind,
            detail: item.detail,
            sort_text,
            filter_text,
            insert_text,
            text_edit,
            additional_text_edits,
            has_additional_text_edits,
            raw: v.clone(),
        }
    }

    /// Raw-JSON fallback for an item that fails strict deserialize — reads
    /// exactly the fields this store uses, tolerating an off-spec shape
    /// anywhere else (a real-world server population: `$/progress` and
    /// completion items are where spec drift concentrates, especially
    /// outside the handful of mature, heavily-used servers). `None` only
    /// when `label` is missing/non-string; every other field already
    /// defaults sensibly.
    fn from_json_lenient(v: &serde_json::Value) -> Option<Self> {
        let label = v.get("label")?.as_str()?.to_string();
        let kind = v.get("kind").and_then(|x| x.as_i64());
        let detail = v.get("detail").and_then(|x| x.as_str()).map(str::to_string);
        let string_or_label = |key: &str| -> String {
            v.get(key)
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| label.clone())
        };
        let is_snippet = v.get("insertTextFormat").and_then(|x| x.as_i64()) == Some(2);
        let sort_text = string_or_label("sortText");
        let filter_text = string_or_label("filterText");
        let insert_text = string_or_label("insertText");
        let insert_text = if is_snippet {
            strip_snippet(&insert_text)
        } else {
            insert_text
        };
        let text_edit = v.get("textEdit").and_then(text_edit_from_json_lenient);
        let text_edit = text_edit.map(|te| {
            if is_snippet {
                lsp_types::TextEdit {
                    new_text: strip_snippet(&te.new_text),
                    ..te
                }
            } else {
                te
            }
        });
        let has_additional_text_edits = v.get("additionalTextEdits").is_some();
        let additional_text_edits = parse_additional_text_edits_lenient(v);
        Some(Self {
            label,
            kind,
            detail,
            sort_text,
            filter_text,
            insert_text,
            text_edit,
            additional_text_edits,
            has_additional_text_edits,
            raw: v.clone(),
        })
    }

    pub(super) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "label": self.label,
            "kind": self.kind,
            "detail": self.detail,
        })
    }
}

/// Rewrites `${n:default}` -> `default` (empty string if no `:default`) and
/// bare `$n` -> "" (dropped) in an `insertTextFormat: Snippet` item's text —
/// v1 has no snippet-expansion UI (no tabstop cycling), so inserting raw
/// snippet syntax verbatim would show it literally in the buffer. No
/// choices (`${n|a,b|}`), no nested placeholders, no `\$` escapes. Operates
/// on `char`s (Unicode scalars), matching how this logic worked when it was
/// Steel `string-ref`/`substring` — this is text-content transformation on
/// server-provided strings, not motion/selection code over buffer
/// positions, so grapheme-cluster stepping doesn't apply here.
fn strip_snippet(text: &str) -> String {
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

/// Extracts `(range, newText)` from a `CompletionTextEdit` JSON value for
/// [`StoredCompletionItem::from_json_lenient`] — either shape (`Edit`:
/// `{"range", "newText"}`, or `InsertReplaceEdit`: `{"insert", "replace",
/// "newText"}`, using the narrower `insert` range). Tolerates a
/// malformed/partial shape by returning `None` — drops just the edit, not
/// the whole item; `accept` then falls back to a word-range edit built from
/// `insert_text`.
fn text_edit_from_json_lenient(v: &serde_json::Value) -> Option<lsp_types::TextEdit> {
    let range = v.get("range").or_else(|| v.get("insert"))?;
    let new_text = v.get("newText")?.as_str()?.to_string();
    let start = range.get("start")?;
    let end = range.get("end")?;
    Some(lsp_types::TextEdit {
        range: lsp_types::Range {
            start: lsp_types::Position {
                line: start.get("line")?.as_u64()? as u32,
                character: start.get("character")?.as_u64()? as u32,
            },
            end: lsp_types::Position {
                line: end.get("line")?.as_u64()? as u32,
                character: end.get("character")?.as_u64()? as u32,
            },
        },
        new_text,
    })
}

/// Lenient `additionalTextEdits` reader, shared by `from_json_lenient`
/// (an off-spec completion item) and the `completionItem/resolve` response
/// handler (which never goes through strict deserialize at all — a resolved
/// item that's otherwise off-spec shouldn't lose a well-formed edit list
/// over an unrelated malformed field elsewhere in the response).
pub(super) fn parse_additional_text_edits_lenient(
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
