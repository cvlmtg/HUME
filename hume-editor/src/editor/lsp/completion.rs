//! Completion orchestration: a Rust store holds the server's items and
//! does the per-keystroke filter/rank; Steel drives `begin!`/
//! `update-filter!`/`top`/`accept!`/`dismiss!`. One singleton session per
//! editor (not per buffer) — starting a new one replaces the old.

use hume_editing::position_encoding::char_to_wire;
use hume_engine::pipeline::BufferId;
use hume_scripting::hooks::HookId;
use hume_scripting::json::json_to_steel;

use super::LspState;
use super::edits::{self, WireEdit};
use super::introspect;
use crate::editor::EditorState;

/// One item as decoded straight from the response hashmap's JSON shape —
/// raw `serde_json::Value` navigation rather than a typed `lsp_types`
/// struct, since `CompletionTextEdit`'s `Edit`/`InsertAndReplace` union
/// only ever needs its `range`/`newText`, not the rest of the LSP shape.
pub(crate) struct StoredCompletionItem {
    pub(crate) label: String,
    /// Raw `CompletionItemKind` number — display-only (icon choice), no
    /// v1 reader maps it to a name.
    pub(crate) kind: Option<i64>,
    pub(crate) detail: Option<String>,
    sort_text: String,
    filter_text: String,
    insert_text: String,
    text_edit: Option<WireEdit>,
    /// The full response item, unparsed — handed to `on-completion-accept`
    /// so Steel can read `additionalTextEdits`, `data` (for
    /// `completionItem/resolve`), or any other field this store doesn't
    /// parse, without Rust needing to grow a reader for every LSP field a
    /// feature might eventually want.
    raw: serde_json::Value,
}

impl StoredCompletionItem {
    fn from_json(v: &serde_json::Value) -> Self {
        let label = v
            .get("label")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        let kind = v.get("kind").and_then(|x| x.as_i64());
        let detail = v
            .get("detail")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let string_or_label = |key: &str| -> String {
            v.get(key)
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| label.clone())
        };
        let sort_text = string_or_label("sortText");
        let filter_text = string_or_label("filterText");
        let insert_text = string_or_label("insertText");
        let text_edit = v.get("textEdit").and_then(text_edit_from_json);
        Self {
            label,
            kind,
            detail,
            sort_text,
            filter_text,
            insert_text,
            text_edit,
            raw: v.clone(),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "label": self.label,
            "kind": self.kind,
            "detail": self.detail,
        })
    }
}

/// Extracts `(range, newText)` from a `CompletionTextEdit` JSON value —
/// either shape (`Edit`: `{"range", "newText"}`, or `InsertReplaceEdit`:
/// `{"insert", "replace", "newText"}`, using the narrower `insert` range).
fn text_edit_from_json(v: &serde_json::Value) -> Option<WireEdit> {
    let range = v.get("range").or_else(|| v.get("insert"))?;
    let new_text = v.get("newText")?.as_str()?.to_string();
    let start = range.get("start")?;
    let end = range.get("end")?;
    Some(WireEdit {
        start_line: start.get("line")?.as_u64()? as usize,
        start_char: start.get("character")?.as_u64()? as usize,
        end_line: end.get("line")?.as_u64()? as usize,
        end_char: end.get("character")?.as_u64()? as usize,
        new_text,
    })
}

/// Case-insensitive (ASCII) subsequence check: every char of `needle` must
/// appear in `haystack`, in order, not necessarily contiguous. Returns the
/// char index of the first matched char (closer-to-start ranks higher), or
/// `None` if `needle` isn't a subsequence of `haystack`.
fn subsequence_match_pos(needle: &str, haystack: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let mut needle_chars = needle.chars();
    let mut want = needle_chars.next();
    let mut first_pos = None;
    for (i, hc) in haystack.chars().enumerate() {
        let Some(nc) = want else { break };
        if hc.eq_ignore_ascii_case(&nc) {
            if first_pos.is_none() {
                first_pos = Some(i);
            }
            want = needle_chars.next();
        }
    }
    if want.is_none() { first_pos } else { None }
}

fn is_prefix_match(needle: &str, haystack: &str) -> bool {
    let mut h = haystack.chars();
    needle
        .chars()
        .all(|n| h.next().is_some_and(|hc| hc.eq_ignore_ascii_case(&n)))
}

pub(crate) struct CompletionSession {
    bid: BufferId,
    /// Char offset where the completed token starts — the primary
    /// selection head at `completion-begin!` time.
    anchor: usize,
    items: Vec<StoredCompletionItem>,
    /// Ranked indices into `items`, rebuilt by every `update_filter` call.
    filtered: Vec<u32>,
    /// Retained across `update_filter` calls so per-keystroke filtering
    /// doesn't allocate a fresh Vec every time.
    rank_scratch: Vec<(bool, usize, u32)>,
    filter: String,
    /// Server's `isIncomplete` flag — gates `on-completion-refilter`:
    /// the hook only fires per-keystroke while this is set, since a complete
    /// list needs no re-request from Steel.
    incomplete: bool,
    /// Buffer generation as of the last `begin`/`update_filter` call —
    /// `accept!` rejects if the buffer changed by any other path since.
    generation_at_begin: u64,
}

/// Insert-mode UI state for an open completion session — kept separate from
/// `CompletionSession` itself (which deliberately has no `selected`) so the
/// session's filtering/accept logic stays free of rendering concerns.
pub(crate) struct LspCompletionUi {
    pub(crate) selected: usize,
}

impl CompletionSession {
    /// Char offset where the completed token starts — the anchor the
    /// completion menu positions itself at (not the live cursor, which
    /// drifts as the user types further into the token).
    pub(crate) fn anchor(&self) -> usize {
        self.anchor
    }

    /// The server's `isIncomplete` flag from the response that began this
    /// session — gates `on-completion-refilter`.
    pub(crate) fn incomplete(&self) -> bool {
        self.incomplete
    }

    pub(crate) fn bid(&self) -> BufferId {
        self.bid
    }

    pub(crate) fn begin(
        state: &EditorState,
        bid: BufferId,
        items_json: &[serde_json::Value],
        incomplete: bool,
    ) -> Self {
        let items: Vec<StoredCompletionItem> = items_json
            .iter()
            .map(StoredCompletionItem::from_json)
            .collect();
        let pid = state.focused_pane_id;
        let anchor = state
            .panes
            .state
            .get(pid)
            .and_then(|by_buf| by_buf.get(bid))
            .map(|pbs| pbs.selections.primary().head())
            .unwrap_or(0);
        let mut session = Self {
            bid,
            anchor,
            items,
            filtered: Vec::new(),
            rank_scratch: Vec::new(),
            filter: String::new(),
            incomplete,
            // Real value stamped by `update_filter`, just below.
            generation_at_begin: 0,
        };
        session.update_filter(state, String::new());
        session
    }

    /// Re-ranks `items` against `text`, re-stamping `generation_at_begin` —
    /// the expected flow is "user types a char into the buffer (bumping
    /// text_gen), then this is called with the new filter text," so a
    /// legitimate keystroke must not itself look like the buffer-changed-
    /// out-from-under-us case `accept!` guards against.
    pub(crate) fn update_filter(&mut self, state: &EditorState, text: String) {
        self.filter = text;
        self.generation_at_begin = state.buffers.get(self.bid).text_gen;
        self.rank_scratch.clear();
        for (i, item) in self.items.iter().enumerate() {
            if let Some(pos) = subsequence_match_pos(&self.filter, &item.filter_text) {
                let prefix = is_prefix_match(&self.filter, &item.filter_text);
                self.rank_scratch.push((prefix, pos, i as u32));
            }
        }
        self.rank_scratch.sort_by(|a, b| {
            b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then_with(|| {
                self.items[a.2 as usize]
                    .sort_text
                    .cmp(&self.items[b.2 as usize].sort_text)
            })
        });
        self.filtered.clear();
        self.filtered
            .extend(self.rank_scratch.iter().map(|&(_, _, i)| i));
    }

    pub(crate) fn top(&self, n: usize) -> Vec<serde_json::Value> {
        self.filtered
            .iter()
            .take(n)
            .map(|&i| self.items[i as usize].to_json())
            .collect()
    }

    /// Applies `filtered[idx]`'s `textEdit` (falling back to `insertText` at
    /// the anchor..cursor span when absent) as one undo step through the edit
    /// engine, gen-checked against `generation_at_begin` — a buffer edit
    /// that bypassed `update_filter` (so never re-stamped the generation)
    /// rejects rather than applying against text the item wasn't computed
    /// for.
    pub(crate) fn accept(
        &self,
        state: &mut EditorState,
        lsp: &LspState,
        idx: usize,
    ) -> Result<(), String> {
        let &item_idx = self
            .filtered
            .get(idx)
            .ok_or_else(|| "completion-accept!: index out of range".to_string())?;
        let item = &self.items[item_idx as usize];
        let wire_edit = match &item.text_edit {
            Some(te) => te.clone(),
            None => {
                let encoding = introspect::encoding_for_buffer(state, lsp, self.bid);
                let rope = state.buffers.get(self.bid).text().rope();
                let cursor = self.anchor + self.filter.chars().count();
                let (start_line, start_char) = char_to_wire(rope, self.anchor, encoding);
                let (end_line, end_char) = char_to_wire(rope, cursor, encoding);
                WireEdit {
                    start_line,
                    start_char,
                    end_line,
                    end_char,
                    new_text: item.insert_text.clone(),
                }
            }
        };
        edits::apply_text_edits(
            state,
            lsp,
            self.bid,
            vec![wire_edit],
            Some(self.generation_at_begin),
        )?;

        // Fire on-completion-accept with the raw item *after* the main
        // edit lands — Steel reads `additionalTextEdits`/`data` from `item`
        // and applies them itself (auto-import edits, resolve-on-accept);
        // Rust never parses those fields.
        let bid_val = hume_scripting::SteelBufferId::new(self.bid).into_steel_val();
        let item_val = json_to_steel(&item.raw);
        state
            .pending_hooks
            .push((HookId::OnCompletionAccept, vec![bid_val, item_val]));
        Ok(())
    }
}

impl EditorState {
    // ── LSP completion menu ─────────────────────────────────────────────

    /// Ends any open completion session and clears its menu view — shared
    /// by `set_mode` (any exit from Insert) and `mappings/insert.rs`'s key
    /// handling (`Esc`, a Backspace crossing the anchor, a successful/failed
    /// accept). A no-op when no session is open.
    pub(crate) fn clear_lsp_completion(&mut self) {
        self.lsp_completion = None;
        self.lsp_completion_ui = None;
        *self
            .lsp_completion_view
            .write()
            .expect("RwLock not poisoned") = None;
    }
}
