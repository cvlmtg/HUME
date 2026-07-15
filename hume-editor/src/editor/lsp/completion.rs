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
use crate::editor::{Editor, EditorState};

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
    sort_text: String,
    filter_text: String,
    insert_text: String,
    text_edit: Option<lsp_types::TextEdit>,
    additional_text_edits: Vec<lsp_types::TextEdit>,
    /// Distinguishes "server sent no `additionalTextEdits` key at all" from
    /// "server sent an empty array" — an empty array still means "nothing
    /// more to apply *and* don't bother resolving", same as a present-but-
    /// empty list; only the key's absence means resolve might have more to
    /// offer. See `CompletionSession::accept`'s resolve gate.
    has_additional_text_edits: bool,
    /// The full response item, unparsed — handed to `on-completion-accept`
    /// so Steel can read `data` or any other field this store doesn't
    /// parse, without Rust needing to grow a reader for every LSP field a
    /// feature might eventually want. Deliberately the *pristine* item
    /// (snippet syntax included) — Steel/resolve should see exactly what
    /// the server sent, not this store's stripped/narrowed projection.
    raw: serde_json::Value,
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
        let detail = v
            .get("detail")
            .and_then(|x| x.as_str())
            .map(str::to_string);
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

    fn to_json(&self) -> serde_json::Value {
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
    /// over the whole identifier token when absent) plus any
    /// `additionalTextEdits` — atomically, as one undo step — gen-checked
    /// against `generation_at_begin`. All edits are wire positions computed
    /// by the server against the *same* pre-accept document (LSP spec:
    /// `additionalTextEdits` never overlap the main edit), so batching them
    /// into one `ChangeSet` needs no relative position adjustment between
    /// them — unlike a resolve response's edits (see below), which are
    /// computed later, against that same pre-accept document, and so must
    /// be mapped *forward* through this edit before they mean anything.
    ///
    /// If the item lacks `additionalTextEdits` entirely (not just an empty
    /// array — see [`StoredCompletionItem::has_additional_text_edits`]) and
    /// the server advertises `completionProvider.resolveProvider`, sends
    /// `completionItem/resolve` and applies whatever it returns once the
    /// response lands (via the ordinary `LspCallback`/`stale_check`
    /// machinery every other `lsp-request` uses — dropped silently if the
    /// buffer has moved past `generation_at_begin`'s successor by then,
    /// same staleness discipline as any other LSP response).
    pub(crate) fn accept(
        &self,
        state: &mut EditorState,
        lsp: &mut LspState,
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

        // Captured *before* the edit lands — a resolve response (if one
        // ends up sent below) is computed against this exact pre-accept
        // document, and its wire positions must be decoded against it, not
        // whatever the buffer holds once the response actually arrives.
        let rope_pre = state.buffers.get(self.bid).text().rope().clone();

        let mut all_edits = Vec::with_capacity(1 + item.additional_text_edits.len());
        all_edits.push(text_edit);
        all_edits.extend(item.additional_text_edits.iter().cloned());
        let accept_cs = edits::apply_text_edits_returning_cs(
            state,
            lsp,
            self.bid,
            all_edits,
            Some(self.generation_at_begin),
        )?;

        // Fire on-completion-accept with the raw (pristine) item after the
        // edit lands — an extension point for anything this store doesn't
        // parse (e.g. `command`); Rust now owns additionalTextEdits/resolve.
        let bid_val = hume_scripting::SteelBufferId::new(self.bid).into_steel_val();
        let item_val = json_to_steel(&item.raw);
        state
            .pending_hooks
            .push((HookId::OnCompletionAccept, vec![bid_val, item_val]));

        if !item.has_additional_text_edits {
            self.maybe_send_resolve(state, lsp, item, rope_pre, accept_cs, encoding);
        }
        Ok(())
    }

    /// Sends `completionItem/resolve` when the server advertised
    /// `completionProvider.resolveProvider` — best-effort: a resolution
    /// error, timeout, or a server that's gone by send time only logs, it
    /// never fails the accept that already landed.
    fn maybe_send_resolve(
        &self,
        state: &mut EditorState,
        lsp: &mut LspState,
        item: &StoredCompletionItem,
        rope_pre: ropey::Rope,
        accept_cs: hume_editing::changeset::ChangeSet,
        encoding: hume_editing::position_encoding::PositionEncoding,
    ) {
        let Some(server_id) = state.buffers.try_get(self.bid).and_then(|b| b.lsp_server) else {
            return;
        };
        let resolve_provider = lsp
            .servers
            .get(&server_id)
            .and_then(|e| e.capabilities_json.as_ref())
            .and_then(|caps| caps.get("completionProvider"))
            .and_then(|cp| cp.get("resolveProvider"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !resolve_provider {
            return;
        }

        // Same discipline `lsp-request` itself uses (bridge.rs): a request
        // minted here must not reach the wire ahead of the didChange
        // describing the edit `accept` just applied.
        super::sync::flush_lsp_pending_changes(state, lsp);
        let bid = self.bid;
        let timeout_ms = state.settings.lsp_request_timeout_ms as u64;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let meta = hume_lsp::client::RequestMeta {
            method: "completionItem/resolve".to_string(),
            allow_stale: false,
            deadline,
        };
        let gen_after = state.buffers.get(bid).text_gen;
        let Some(id) = lsp.send_request(
            server_id,
            "completionItem/resolve",
            item.raw.clone(),
            meta,
        ) else {
            return; // server gone between the capability check and now
        };
        let callback: super::LspCallback = Box::new(move |editor, outcome| {
            match outcome {
                hume_lsp::client::Outcome::Ok(resolved) => {
                    let resolved_edits = parse_additional_text_edits_lenient(&resolved);
                    if let Err(e) = edits::apply_resolved_additional_edits(
                        &mut editor.state,
                        bid,
                        &rope_pre,
                        &accept_cs,
                        encoding,
                        &resolved_edits,
                    ) {
                        editor.report(
                            crate::editor::Severity::Error,
                            format!("lsp completion resolve: {e}"),
                        );
                    }
                }
                hume_lsp::client::Outcome::Err(e) => {
                    editor.report(
                        crate::editor::Severity::Error,
                        format!("lsp completion resolve: {} ({})", e.message, e.code),
                    );
                }
                hume_lsp::client::Outcome::TimedOut => {
                    editor.report(
                        crate::editor::Severity::Error,
                        "lsp completion resolve: timeout".to_string(),
                    );
                }
            }
        });
        lsp.register_callback(server_id, id, Some((bid, gen_after)), callback);
    }
}

/// Lenient `additionalTextEdits` reader, shared by `from_json_lenient`
/// (an off-spec completion item) and the `completionItem/resolve` response
/// handler (which never goes through strict deserialize at all — a resolved
/// item that's otherwise off-spec shouldn't lose a well-formed edit list
/// over an unrelated malformed field elsewhere in the response).
fn parse_additional_text_edits_lenient(resolved: &serde_json::Value) -> Vec<lsp_types::TextEdit> {
    resolved
        .get("additionalTextEdits")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().filter_map(text_edit_from_json_lenient).collect())
        .unwrap_or_default()
}

/// Clears `lsp`'s completion session + menu UI (not the shared view Arc —
/// callers hold that separately: `Editor` via `state.lsp_completion_view`,
/// `EditorHostImpl` via its own disjoint `state` borrow). Single definition
/// of "what constitutes an open completion session", shared by
/// `Editor::clear_lsp_completion`, `EditorHostImpl::clear_lsp_completion`,
/// and `completion_accept`.
pub(crate) fn clear_completion_state(lsp: &mut LspState) {
    lsp.completion = None;
    lsp.completion_ui = None;
}

impl Editor {
    // ── LSP completion menu ─────────────────────────────────────────────

    /// Ends any open completion session and clears its menu view — shared
    /// by every completion-key handler in `mappings/insert.rs` (`Esc`, a
    /// Backspace crossing the anchor, a successful/failed accept) and by
    /// `take_pending_lsp_completion_dismiss`. A no-op when no session is
    /// open.
    pub(crate) fn clear_lsp_completion(&mut self) {
        clear_completion_state(&mut self.lsp);
        *self
            .state
            .lsp_completion_view
            .write()
            .expect("RwLock not poisoned") = None;
    }

    /// Consumes `set_mode`'s deferred dismissal, if one is pending — called
    /// at every chokepoint between "a mode change could have happened" and
    /// "the next render" (see the flag's own doc comment on `EditorState`).
    pub(crate) fn take_pending_lsp_completion_dismiss(&mut self) {
        if std::mem::take(&mut self.state.lsp_completion_dismiss_pending) {
            self.clear_lsp_completion();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_snippet_default_placeholder_becomes_its_default_text() {
        assert_eq!(strip_snippet("${1:foo}"), "foo");
    }

    #[test]
    fn strip_snippet_bare_tabstop_is_dropped() {
        assert_eq!(strip_snippet("before$0after"), "beforeafter");
    }

    #[test]
    fn strip_snippet_multi_digit_tabstop_is_dropped() {
        assert_eq!(strip_snippet("$12"), "");
    }

    #[test]
    fn strip_snippet_empty_default_becomes_empty_string() {
        assert_eq!(strip_snippet("${1:}"), "");
    }

    #[test]
    fn strip_snippet_placeholder_with_no_colon_becomes_empty_string() {
        assert_eq!(strip_snippet("${1}"), "");
    }

    #[test]
    fn strip_snippet_unterminated_placeholder_consumes_to_end_of_string() {
        assert_eq!(strip_snippet("${1:foo"), "foo");
    }

    #[test]
    fn strip_snippet_the_lsp_md_documented_example() {
        assert_eq!(
            strip_snippet("for ${1:x} in ${2:iter} {\n    $0\n}"),
            "for x in iter {\n    \n}"
        );
    }

    #[test]
    fn strip_snippet_leaves_plain_text_untouched() {
        assert_eq!(strip_snippet("no snippet syntax here"), "no snippet syntax here");
    }

    #[test]
    fn strip_snippet_a_dollar_followed_by_a_digit_is_always_a_tabstop_reference() {
        // "$5" is a bare tabstop ref (dropped) even mid-word — "$5.00" is
        // not special-cased as currency; only the digit run after `$` is
        // consumed.
        assert_eq!(strip_snippet("$5.00"), ".00");
    }

    #[test]
    fn strip_snippet_a_dollar_with_no_following_brace_or_digit_is_copied_literally() {
        assert_eq!(strip_snippet("price: $x"), "price: $x");
    }

    /// End-to-end: `from_typed` only strips when the server declared
    /// `insertTextFormat: Snippet` (2) — a plain-text item's `$` literals
    /// must survive untouched.
    #[test]
    fn from_typed_strips_snippet_insert_text_only_when_format_is_snippet() {
        let v = serde_json::json!({
            "label": "foo",
            "insertText": "${1:foo}(${2:bar})",
            "insertTextFormat": 2,
        });
        let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
        assert_eq!(item.insert_text, "foo(bar)");
        assert_eq!(
            item.raw.get("insertText").and_then(|v| v.as_str()),
            Some("${1:foo}(${2:bar})"),
            "raw must keep the pristine snippet text for on-completion-accept/resolve"
        );
    }

    #[test]
    fn from_typed_leaves_insert_text_untouched_without_snippet_format() {
        let v = serde_json::json!({
            "label": "foo",
            "insertText": "$100 literal",
        });
        let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
        assert_eq!(item.insert_text, "$100 literal");
    }

    #[test]
    fn from_typed_strips_snippet_text_edit_new_text() {
        let v = serde_json::json!({
            "label": "foo",
            "insertTextFormat": 2,
            "textEdit": {
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                "newText": "${1:foo}",
            },
        });
        let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
        assert_eq!(item.text_edit.unwrap().new_text, "foo");
    }

    /// `from_json_lenient` (the recovery path for an off-spec item) must
    /// strip snippet syntax too — not just the strict `from_typed` path.
    #[test]
    fn from_json_lenient_also_strips_snippet_insert_text() {
        // A non-numeric `kind` forces the whole item through the lenient
        // fallback (same trick as `string_kind_recovers_via_lenient_fallback`
        // below), while `insertTextFormat`/`insertText` stay well-formed.
        let v = serde_json::json!({
            "label": "foo",
            "kind": "Function",
            "insertTextFormat": 2,
            "insertText": "${1:foo}",
        });
        assert_strict_parse_fails(&v);
        let item = StoredCompletionItem::from_json(&v).expect("label present — must recover");
        assert_eq!(item.insert_text, "foo");
        assert_eq!(
            item.raw.get("insertText").and_then(|v| v.as_str()),
            Some("${1:foo}"),
            "raw must keep the pristine snippet text"
        );
    }

    /// Independent oracle: strict `lsp_types::CompletionItem` deserialize
    /// really does reject `v` on its own — otherwise a test using this
    /// wouldn't be exercising `from_json_lenient` at all, just re-testing
    /// the strict path.
    fn assert_strict_parse_fails(v: &serde_json::Value) {
        assert!(
            serde_json::from_value::<lsp_types::CompletionItem>(v.clone()).is_err(),
            "test input must be strict-parse-rejecting to exercise the lenient fallback: {v}"
        );
    }

    #[test]
    fn well_formed_item_never_touches_the_lenient_path() {
        // Sanity check for the two tests below: a spec-compliant item must
        // NOT need `from_json_lenient` — if this failed, `strict_parse_fails`
        // in those tests wouldn't prove anything.
        let v = serde_json::json!({"label": "ok", "kind": 3});
        assert!(serde_json::from_value::<lsp_types::CompletionItem>(v.clone()).is_ok());
        let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
        assert_eq!(item.label, "ok");
        assert_eq!(item.kind, Some(3));
    }

    #[test]
    fn string_kind_recovers_via_lenient_fallback() {
        // A server sending a human-readable kind string instead of the LSP
        // numeric enum: `CompletionItemKind` is a transparent i32 newtype,
        // so a JSON string for `kind` fails strict deserialize of the whole
        // item, not just that field.
        let v = serde_json::json!({"label": "foo", "kind": "Function"});
        assert_strict_parse_fails(&v);

        let item = StoredCompletionItem::from_json(&v).expect("label present — must recover");
        assert_eq!(item.label, "foo");
        // The lenient reader can't make sense of a non-numeric kind either
        // — dropped, not faked as some default kind.
        assert_eq!(item.kind, None);
        // Undefaulted text fields still fall back to `label`, same as the
        // strict path's `unwrap_or_else(|| label.clone())`.
        assert_eq!(item.sort_text, "foo");
        assert_eq!(item.filter_text, "foo");
        assert_eq!(item.insert_text, "foo");
    }

    #[test]
    fn malformed_text_edit_recovers_the_item_without_the_edit() {
        // `newText` missing fails both `CompletionTextEdit` union variants
        // (`Edit`/`InsertAndReplace`), which fails the whole item's strict
        // parse even though only the edit is broken.
        let v = serde_json::json!({
            "label": "bar",
            "detail": "a detail",
            "textEdit": {
                "range": {
                    "start": {"line": 1, "character": 2},
                    "end": {"line": 1, "character": 5},
                },
            },
        });
        assert_strict_parse_fails(&v);

        let item = StoredCompletionItem::from_json(&v).expect("label present — must recover");
        assert_eq!(item.label, "bar");
        assert_eq!(item.detail.as_deref(), Some("a detail"));
        assert!(
            item.text_edit.is_none(),
            "a malformed textEdit must be dropped, not the whole item"
        );
    }

    #[test]
    fn missing_label_is_rejected_by_both_strict_and_lenient() {
        let v = serde_json::json!({"kind": 1});
        assert_strict_parse_fails(&v);
        assert!(
            StoredCompletionItem::from_json(&v).is_err(),
            "no label recoverable — item must still be dropped"
        );
    }
}
