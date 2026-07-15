//! Completion orchestration: a Rust store holds the server's items and
//! does the per-keystroke filter/rank; Steel drives `begin!`/
//! `update-filter!`/`top`/`accept!`/`dismiss!`. One singleton session per
//! editor (not per buffer) — starting a new one replaces the old.

use hume_editing::position_encoding::char_to_wire;
use hume_engine::pipeline::BufferId;
use hume_scripting::hooks::HookId;
use hume_scripting::json::json_to_steel;

use super::LspState;
use super::edits;
use super::introspect;
use crate::editor::EditorState;

/// One item, typed via `lsp_types::CompletionItem` (the response round-trips
/// through Steel first — `strip-snippet-item` rewrites `insertText`/
/// `textEdit.newText` string values, so this parses post-Steel-mutation
/// JSON, not the raw wire shape, but the type stays the same either way).
pub(crate) struct StoredCompletionItem {
    pub(crate) label: String,
    /// Raw `CompletionItemKind` number — display-only (icon choice), no
    /// v1 reader maps it to a name. Read straight from JSON rather than the
    /// typed field: `CompletionItemKind` wraps a private `i32` with no
    /// accessor.
    pub(crate) kind: Option<i64>,
    pub(crate) detail: Option<String>,
    sort_text: String,
    filter_text: String,
    insert_text: String,
    text_edit: Option<lsp_types::TextEdit>,
    /// The full response item, unparsed — handed to `on-completion-accept`
    /// so Steel can read `additionalTextEdits`, `data` (for
    /// `completionItem/resolve`), or any other field this store doesn't
    /// parse, without Rust needing to grow a reader for every LSP field a
    /// feature might eventually want.
    raw: serde_json::Value,
}

impl StoredCompletionItem {
    /// Parses one item; `v` itself is never consumed, so `raw: v.clone()`
    /// (below) still captures the full item, including fields this
    /// projection drops. `Err` on a spec violation (e.g. a missing
    /// `label`); callers skip the item and report a Trace line rather than
    /// fabricating a placeholder.
    pub(crate) fn from_json(v: &serde_json::Value) -> Result<Self, serde_json::Error> {
        let item: lsp_types::CompletionItem = serde_json::from_value(v.clone())?;
        let label = item.label;
        let kind = v.get("kind").and_then(|x| x.as_i64());
        let sort_text = item.sort_text.unwrap_or_else(|| label.clone());
        let filter_text = item.filter_text.unwrap_or_else(|| label.clone());
        let insert_text = item.insert_text.unwrap_or_else(|| label.clone());
        let text_edit = item.text_edit.map(|te| match te {
            lsp_types::CompletionTextEdit::Edit(te) => te,
            // Preserves the existing "use the narrower insert range" choice.
            lsp_types::CompletionTextEdit::InsertAndReplace(ire) => lsp_types::TextEdit {
                range: ire.insert,
                new_text: ire.new_text,
            },
        });
        Ok(Self {
            label,
            kind,
            detail: item.detail,
            sort_text,
            filter_text,
            insert_text,
            text_edit,
            raw: v.clone(),
        })
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "label": self.label,
            "kind": self.kind,
            "detail": self.detail,
        })
    }
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

/// Scans backward from `pos` over identifier (`Word`-class) chars, stopping
/// at the first non-`Word` boundary — the start of the token immediately
/// preceding `pos`. Grapheme-safe (steps via `prev_grapheme_boundary`, never
/// a raw `-= 1`) since this walks buffer positions, not wire positions.
fn word_start_before(text: &hume_editing::text::Text, pos: usize) -> usize {
    let mut cursor = pos;
    while cursor > 0 {
        let prev = hume_editing::grapheme::prev_grapheme_boundary(text, cursor);
        let Some(ch) = text.char_at(prev) else { break };
        if hume_editing::word::classify_char(ch) != hume_editing::word::CharClass::Word {
            break;
        }
        cursor = prev;
    }
    cursor
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

    /// Number of candidates surviving the current filter — cheap count for
    /// callers (menu navigation, the visible-menu check) that don't need the
    /// items themselves; unlike `top(n).len()`, this doesn't serialize any
    /// candidate to JSON.
    pub(crate) fn len(&self) -> usize {
        self.filtered.len()
    }

    /// Whether the current filter matches nothing. A session can be open
    /// with this `true` — narrowed to empty by continued typing, or an
    /// `isIncomplete` list awaiting an async re-request — in which case no
    /// menu is visibly shown.
    pub(crate) fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    /// Returns `None` when `bid` isn't shown in the focused pane — a normal
    /// race (the async completion response landed after the user switched
    /// panes), not a caller bug, so this is silently absorbed by the caller
    /// rather than raised as a Steel error.
    pub(crate) fn begin(
        state: &EditorState,
        bid: BufferId,
        items: Vec<StoredCompletionItem>,
        incomplete: bool,
    ) -> Option<Self> {
        let pid = state.focused_pane_id;
        let anchor = state
            .panes
            .state
            .get(pid)
            .and_then(|by_buf| by_buf.get(bid))
            .map(|pbs| pbs.selections.primary().head())?;
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
        Some(session)
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

    /// Applies `filtered[idx]`'s `textEdit` (falling back to `insertText`
    /// over the whole identifier token when absent) as one undo step
    /// through the edit engine, gen-checked against `generation_at_begin` —
    /// a buffer edit that bypassed `update_filter` (so never re-stamped the
    /// generation) rejects rather than applying against text the item
    /// wasn't computed for.
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
        let encoding = introspect::encoding_for_buffer(state, lsp, self.bid);
        let cursor = self.anchor + self.filter.chars().count();
        // `char_to_wire` positions are internal rope-derived counters, not
        // untrusted plugin input — a real rope can't exceed u32 lines/columns
        // short of a multi-gigabyte single line, so `expect` here fails loud
        // on a genuine invariant violation rather than silently truncating.
        let to_position = |rope: &ropey::Rope, pos: usize| {
            let (line, character) = char_to_wire(rope, pos, encoding);
            lsp_types::Position {
                line: u32::try_from(line).expect("line exceeds u32"),
                character: u32::try_from(character).expect("character exceeds u32"),
            }
        };
        let text_edit = match &item.text_edit {
            Some(te) => {
                // The server's range was computed against the anchor..cursor
                // span *at request time* — characters typed since (further
                // narrowing the filter) sit just past `end` and must be
                // replaced too, or they survive verbatim next to the
                // inserted text. Only extend, never shrink: a cursor at or
                // before the server's own end leaves its range untouched.
                let rope = state.buffers.get(self.bid).text().rope();
                let end = to_position(rope, cursor);
                let mut te = te.clone();
                if end > te.range.end {
                    te.range.end = end;
                }
                te
            }
            None => {
                // No server-provided range: replace the whole identifier
                // token, not just the anchor..cursor span — any prefix typed
                // *before* triggering completion (e.g. "fo" before the popup
                // opened) sits before `anchor` and is otherwise left
                // untouched, duplicating it ahead of `insert_text`.
                let text = state.buffers.get(self.bid).text();
                let start = word_start_before(text, self.anchor);
                let rope = text.rope();
                lsp_types::TextEdit {
                    range: lsp_types::Range {
                        start: to_position(rope, start),
                        end: to_position(rope, cursor),
                    },
                    new_text: item.insert_text.clone(),
                }
            }
        };
        edits::apply_text_edits(
            state,
            lsp,
            self.bid,
            vec![text_edit],
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
