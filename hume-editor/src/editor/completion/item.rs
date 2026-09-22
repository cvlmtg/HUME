//! One completion item, typed via `lsp_types::CompletionItem`, plus the
//! lenient JSON fallback for off-spec servers. Snippet stripping and the
//! lenient `TextEdit` decode this relies on live in `hume_lsp::completion_item`
//! — pure protocol work with no editor dependency — but the item type itself
//! is HUME's completion-store item: `CompletionSession` ranks/filters it and
//! `to_json`/`menu_row` render it, neither of which is a wire concern.
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
        }
    }

    /// Parses one item, strict first. Takes `v` by value and deserializes
    /// against `&v` (`serde_json::Value` implements `Deserializer` for a
    /// reference) rather than `serde_json::from_value(v.clone())`, so the
    /// common (`Ok`) path moves `v` into `raw` (below) instead of cloning it
    /// — every item in a response pays this once, every keystroke a
    /// streaming LSP source re-answers. A strict deserialize into
    /// `lsp_types::CompletionItem` rejects on *any* off-spec field (an
    /// out-of-range `kind`, a malformed `textEdit`, ...), not just the ones
    /// this store reads — [`Self::from_json_lenient`] then recovers what it
    /// can straight from JSON (against `&v`, since `v` is still needed for
    /// its own `raw` on that path too). `Err` only when even that fails
    /// (`label` itself missing/non-string); callers skip the item and
    /// report a Trace line rather than fabricating a placeholder.
    pub(in crate::editor) fn from_json(v: serde_json::Value) -> Result<Self, serde_json::Error> {
        match <lsp_types::CompletionItem as serde::Deserialize>::deserialize(&v) {
            Ok(item) => Ok(Self::from_typed(item, v)),
            Err(strict_err) => Self::from_json_lenient(&v).ok_or(strict_err),
        }
    }

    /// Builds from an already-typed item — the common case, when the whole
    /// response round-trips through strict deserialize. Takes `v` by value
    /// and moves it into `raw` — see [`Self::from_json`]'s own doc.
    fn from_typed(item: lsp_types::CompletionItem, v: serde_json::Value) -> Self {
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
        let text_edit = text_edit.map(|te| strip_snippet_from_edit(te, is_snippet));
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
            raw: v,
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
        let text_edit = text_edit.map(|te| strip_snippet_from_edit(te, is_snippet));
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

    /// The accept-time replacement text — read from outside this module by
    /// the `Minibuf`-target accept path (`input_stack/completion.rs`,
    /// `input_stack/command.rs`), which splices it into the minibuffer's
    /// own input directly rather than through `CompletionSession::accept`
    /// (`Buffer`-target only).
    pub(in crate::editor) fn insert_text(&self) -> &str {
        &self.insert_text
    }

    /// Whether accepting this item would change nothing beyond leaving
    /// `typed` in place — `rank` drops these before scoring, so a source
    /// doesn't have to filter its own already-typed token back out by hand
    /// (three call sites did, independently, before this method existed).
    ///
    /// An LSP item survives this by construction, not by exemption: HUME
    /// advertises no `completionItem.resolveSupport`
    /// (`hume-lsp/src/client.rs`), so a server must send whatever
    /// `additionalTextEdits` it has *with* the item, not lazily via
    /// `completionItem/resolve` — an auto-import on an otherwise-typed name
    /// arrives as a non-empty `additional_text_edits` here, which is a real
    /// edit, not a no-op. A `text_edit` is excluded from this check for the
    /// same reason: its range may cover more than the typed token (a
    /// case-correction, a snippet), so `insert_text` alone can't stand in
    /// for what accepting it actually does.
    pub(super) fn is_noop_for(&self, typed: &str) -> bool {
        self.text_edit.is_none()
            && self.additional_text_edits.is_empty()
            && self.insert_text == typed
    }

    /// `source` is the contributing source's name. `session.rs` pairs each
    /// item with its source's index into its own source list rather than
    /// storing the name on `Self`, so the caller — the only one that knows
    /// it — passes it in for the Steel-visible `"source"` key
    /// `completion-top` surfaces.
    pub(super) fn to_json(&self, source: &str) -> serde_json::Value {
        serde_json::json!({
            "label": &self.label,
            "kind": self.kind,
            "detail": self.detail.as_deref(),
            "source": source,
        })
    }

    /// This item's menu row: `label` as the main column, `detail` (when
    /// non-empty) as the right-aligned trailing one — reads both directly
    /// rather than going through [`Self::to_json`], since the menu never
    /// needs `kind`. Column layout/alignment is `resolve_menu`'s job
    /// (`hume_ui::popup`), not this store's. Called fresh every frame the
    /// menu is open (`CompletionSession::rows_in`), but only for the
    /// handful of rows in the visible window — a `String` clone per row per
    /// frame there is cheaper than paying an `Arc<str>` conversion for
    /// every parsed item, most of which are never scrolled into view.
    pub(super) fn menu_row(&self) -> hume_ui::popup::MenuRow {
        hume_ui::popup::MenuRow {
            main: self.label.clone(),
            trailing: self.detail.clone().filter(|d| !d.is_empty()),
        }
    }
}

/// Strips snippet syntax from `te.new_text` when `is_snippet` — shared by
/// the strict (`from_typed`) and lenient (`from_json_lenient`) decode paths,
/// which both apply this same rule to the item's own `text_edit`.
fn strip_snippet_from_edit(te: lsp_types::TextEdit, is_snippet: bool) -> lsp_types::TextEdit {
    if is_snippet {
        lsp_types::TextEdit {
            new_text: strip_snippet(&te.new_text),
            ..te
        }
    } else {
        te
    }
}

#[cfg(test)]
mod tests;
