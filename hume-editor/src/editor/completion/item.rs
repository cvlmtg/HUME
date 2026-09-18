//! One completion item, typed via `lsp_types::CompletionItem`, plus the
//! lenient JSON fallback for off-spec servers. Snippet stripping and the
//! lenient `TextEdit` decode this relies on live in `hume_lsp::completion_item`
//! — pure protocol work with no editor dependency — but the item type itself
//! is HUME's completion-store item: `CompletionSession` ranks/filters it and
//! `to_json`/`menu_row_label` render it, neither of which is a wire concern.
//! The wire type is always spelled `lsp_types::CompletionItem`; the bare
//! name here is this store's own item.

use hume_lsp::completion_item::{parse_additional_text_edits_lenient, strip_snippet};

/// One item, typed via `lsp_types::CompletionItem`. `insert_text`/`text_edit`
/// have snippet syntax (`${n:default}`, `$n`) already stripped when the
/// server declared `insertTextFormat: Snippet` — see [`strip_snippet`].
/// `raw` keeps the pristine, unstripped JSON (Steel's `on-completion-accept`
/// hook and `completionItem/resolve` both see the server's original text).
pub(in crate::editor) struct CompletionItem {
    pub(super) label: String,
    /// Raw `CompletionItemKind` number — display-only (icon choice), no
    /// v1 reader maps it to a name. Read straight from JSON rather than the
    /// typed field: `CompletionItemKind` wraps a private `i32` with no
    /// accessor.
    pub(super) kind: Option<i64>,
    pub(super) detail: Option<String>,
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
    /// The name of the source that contributed this item — `"lsp"` for
    /// today's sole caller. Empty at construction (`from_json`/
    /// `from_json_lenient` parse a wire item in isolation, with no source
    /// context of their own); `CompletionSession::add_items` stamps every
    /// item with its own `source` argument immediately after parsing,
    /// before any item is visible outside this module. Used for the
    /// per-source eviction on merge, the rank tiebreaker, and display.
    pub(super) source: Box<str>,
}

impl CompletionItem {
    /// Builds a non-LSP item — every minibuffer completer's constructor.
    /// `filter_text` is always `label` (what the user sees is what a
    /// `MatchKind::String` source matches against); `sort_text` is the
    /// caller's own tiebreak key — the item's own `label` for a `String`
    /// source (alphabetical), or empty for a `MatchKind::Delegated` source,
    /// so the rank key's final
    /// index-ascending tiebreak preserves the delegate's own return order
    /// instead of re-sorting it. Every LSP-only field defaults inert:
    /// `kind`/`detail`/`text_edit` absent, no `additionalTextEdits`, `raw`
    /// null — nothing here is a wire concern.
    pub(super) fn plain(label: String, insert_text: String, sort_text: String) -> Self {
        Self {
            filter_text: label.clone(),
            label,
            insert_text,
            sort_text,
            kind: None,
            detail: None,
            text_edit: None,
            additional_text_edits: Vec::new(),
            has_additional_text_edits: false,
            raw: serde_json::Value::Null,
            source: Box::default(),
        }
    }

    /// Parses one item, strict first: `v` itself is never consumed, so
    /// `raw: v.clone()` (below) still captures the full item, including
    /// fields this projection drops. A strict deserialize into
    /// `lsp_types::CompletionItem` rejects on *any* off-spec field (an
    /// out-of-range `kind`, a malformed `textEdit`, ...), not just the ones
    /// this store reads — [`Self::from_json_lenient`] then recovers what it
    /// can straight from JSON. `Err` only when even that fails (`label`
    /// itself missing/non-string); callers skip the item and report a Trace
    /// line rather than fabricating a placeholder.
    pub(in crate::editor) fn from_json(v: &serde_json::Value) -> Result<Self, serde_json::Error> {
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
            source: Box::default(),
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
        let text_edit = v
            .get("textEdit")
            .and_then(hume_lsp::completion_item::text_edit_from_json_lenient);
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
            source: Box::default(),
        })
    }

    /// The accept-time replacement text — read from outside this module by
    /// the `Minibuf`-target accept path (`input_stack/completion.rs`,
    /// `input_stack/command.rs`), which splices it into the minibuffer's
    /// own input directly rather than through `CompletionSession::accept`
    /// (`Buffer`-target only).
    pub(in crate::editor) fn insert_text(&self) -> &str {
        &self.insert_text
    }

    pub(super) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "label": self.label,
            "kind": self.kind,
            "detail": self.detail,
            "source": self.source,
        })
    }

    /// Formats this item as `"label  detail"`, uniformly styled — per-part
    /// dimming would need segment-styled rows, which no card requires. The
    /// menu's own row label: reads `label`/`detail` directly rather than
    /// going through [`Self::to_json`], since the menu never needs `kind`.
    pub(super) fn menu_row_label(&self) -> String {
        match self.detail.as_deref() {
            Some(detail) if !detail.is_empty() => format!("{}  {detail}", self.label),
            _ => self.label.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
