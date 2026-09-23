//! One completion item, decoded leniently from JSON — tolerant of an
//! off-spec shape anywhere but `label` itself, since real-world servers
//! concentrate their spec drift exactly here (outside the handful of
//! mature, heavily-used ones). Snippet stripping and the lenient `TextEdit`
//! decode this relies on live in `hume_lsp::completion_item` — pure
//! protocol work with no editor dependency — but the item type itself is
//! HUME's completion-store item: `CompletionSession` ranks/filters it and
//! `to_json`/`menu_row` render it, neither of which is a wire concern.
//! The wire type is always spelled `lsp_types::CompletionItem`; the bare
//! name here is this store's own item.

use hume_lsp::completion_item::{parse_additional_text_edits_lenient, strip_snippet};

/// One item, decoded from a `textDocument/completion` response element.
/// `insert_text`/`text_edit` have snippet syntax (`${n:default}`, `$n`)
/// already stripped when the server declared `insertTextFormat: Snippet` —
/// see [`strip_snippet`]. `raw` keeps the pristine, unstripped JSON (Steel's
/// `on-completion-accept` hook and `completionItem/resolve` both see the
/// server's original text).
pub(in crate::editor) struct CompletionItem {
    pub(super) label: String,
    /// Display-only (icon choice) — no v1 reader maps it to a name.
    pub(super) kind: Option<lsp_types::CompletionItemKind>,
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
    /// `filter_text`/`sort_text` are both always `label`: `filter_text`
    /// because what the user sees is what a `MatchKind::String` source
    /// matches against, `sort_text` because it's the only tiebreak key a
    /// `String` source's items need (alphabetical) — a `MatchKind::Delegated`
    /// source's own order is instead preserved by `rank`'s Delegated arm,
    /// which skips the sortText tiebreak key entirely rather than relying on
    /// every Delegated constructor leaving it empty by convention. Every
    /// LSP-only field defaults inert: `kind`/`detail`/`text_edit` absent, no
    /// `additionalTextEdits`, `raw` null — nothing here is a wire concern.
    pub(super) fn plain(label: String, insert_text: String) -> Self {
        Self {
            filter_text: label.clone(),
            sort_text: label.clone(),
            label,
            insert_text,
            kind: None,
            detail: None,
            text_edit: None,
            additional_text_edits: Vec::new(),
            has_additional_text_edits: false,
            raw: serde_json::Value::Null,
        }
    }

    /// Parses one item, tolerant of an off-spec shape anywhere but `label`
    /// itself — every other field already defaults sensibly (falling back
    /// to `label`, or absent). `None` only when `label` is missing or
    /// non-string; callers skip the item and report a Trace line rather
    /// than fabricating a placeholder. Reads against `&v` and moves `v`
    /// into `raw` at the end, so every item in a response pays one JSON
    /// walk, not a clone plus a second deserialize pass.
    pub(in crate::editor) fn from_json(v: serde_json::Value) -> Option<Self> {
        use serde::Deserialize;

        let label = v.get("label")?.as_str()?.to_string();
        let kind = v
            .get("kind")
            .and_then(|k| lsp_types::CompletionItemKind::deserialize(k).ok());
        let detail = v.get("detail").and_then(|x| x.as_str()).map(str::to_string);
        let string_or_label = |key: &str| -> String {
            v.get(key)
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| label.clone())
        };
        let is_snippet = v
            .get("insertTextFormat")
            .and_then(|f| lsp_types::InsertTextFormat::deserialize(f).ok())
            == Some(lsp_types::InsertTextFormat::SNIPPET);
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
        // A `null` `additionalTextEdits` counts as absent, same as the key
        // being missing entirely — only a genuine (possibly empty) array
        // means "the server answered this and there's nothing more to
        // resolve" (see this field's own doc).
        let has_additional_text_edits = v
            .get("additionalTextEdits")
            .is_some_and(|x| !x.is_null());
        let additional_text_edits = parse_additional_text_edits_lenient(&v);
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
            raw: v,
        })
    }

    /// Whether this item names a directory to descend into — the `:` line's
    /// Enter handler (`input_stack/completion.rs`) reads this generically
    /// rather than checking which source produced the candidate; any
    /// source's item can opt in the same way `path.rs`'s directory entries
    /// do.
    pub(in crate::editor) fn is_folder(&self) -> bool {
        self.kind == Some(lsp_types::CompletionItemKind::FOLDER)
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
    /// `typed` in place — `rank` drops these before scoring, so no source
    /// has to filter its own already-typed token back out by hand.
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
        self.is_plain() && self.insert_text == typed
    }

    /// No `textEdit`, no `additionalTextEdits` — accepting this item does
    /// nothing beyond inserting `insert_text` at the cursor. Shared by
    /// [`Self::is_noop_for`] and `CompletionSession::recompute_dedup`'s
    /// cross-source duplicate check: only a plain item is ever hidden as
    /// someone else's duplicate, since an item carrying edits does
    /// something a duplicate-looking plain item from another source
    /// wouldn't.
    pub(super) fn is_plain(&self) -> bool {
        self.text_edit.is_none() && self.additional_text_edits.is_empty()
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

/// Strips snippet syntax from `te.new_text` when `is_snippet` — applies the
/// same rule [`strip_snippet`] applies to `insert_text` to the item's own
/// `text_edit`.
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
