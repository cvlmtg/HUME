//! Completion orchestration: a Rust store holds the server's items and
//! does the per-keystroke filter/rank; Steel drives `begin!`/
//! `update-filter!`/`top`/`accept!`/`dismiss!`. One singleton session per
//! editor (not per buffer) — starting a new one replaces the old.

use hume_editing::changeset::{Assoc, ChangeSet};
use hume_editing::position_encoding::wire_to_char;
use hume_engine::pipeline::BufferId;
use hume_scripting::hooks::HookId;
use hume_scripting::json::json_to_steel;

use super::LspState;
use super::edits;
use super::introspect;
use crate::editor::{Editor, EditorState, doc_ops, pane_state};
use crate::ops::edit::replace_around_cursors;

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
    /// `anchor`'s value at `begin()`, before any `remap_anchor` call —
    /// paired with `rope_at_begin` as the coordinate system a server's
    /// `textEdit` range was computed against. Unlike `anchor`, never
    /// remapped: it's a fixed reference point, not a position tracked
    /// through edits.
    anchor_at_begin: usize,
    /// The buffer's rope at `begin()` time — an O(1) clone (ropey is
    /// structurally shared). A server's wire `textEdit` range is computed
    /// against the document as it stood at the completion *request*, which
    /// is this snapshot, not whatever the buffer holds by `accept()` time:
    /// if an earlier cursor on the same line has since inserted text (only
    /// possible when the primary isn't the first cursor), decoding the
    /// server's range against the live rope would land on the wrong chars.
    rope_at_begin: ropey::Rope,
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
pub(crate) struct CompletionMenuUi {
    pub(crate) selected: usize,
}

impl CompletionSession {
    /// Char offset where the completed token starts — the anchor the
    /// completion menu positions itself at (not the live cursor, which
    /// drifts as the user types further into the token).
    pub(crate) fn anchor(&self) -> usize {
        self.anchor
    }

    /// Remaps `anchor` through an Insert-mode edit — called after every
    /// keystroke that lands in the buffer while this session is open, not
    /// just ones at the primary cursor. Without this, a keystroke at a
    /// cursor *before* the primary (multi-cursor Insert mode) shifts the
    /// primary head by more than one char while `anchor` stays put, and
    /// `refilter_lsp_completion_after_edit`'s `slice(anchor..head)` picks up
    /// the drifted text.
    ///
    /// `Assoc::Before`: the anchor marks the token's start, so text inserted
    /// exactly at it belongs to the token and the anchor must stay left of
    /// it — same association `apply_doc_edit_grouped` already uses for
    /// `pinned_anchors`.
    pub(crate) fn remap_anchor(&mut self, cs: &ChangeSet) {
        let mut positions = [self.anchor];
        cs.map_positions(&mut positions, Assoc::Before);
        self.anchor = positions[0];
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
            anchor_at_begin: anchor,
            rope_at_begin: state.buffers.get(bid).text().rope().clone(),
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
    /// over the whole identifier token when absent) at *every* cursor in the
    /// focused pane, as if the completion had been typed at each — a
    /// conforming server's completion range always contains the request
    /// position (LSP spec, `completion.rs`'s `text_edit` doc), so the
    /// primary's own edit, re-expressed as a char count behind/ahead of its
    /// live head, is the same span typing would have consumed at any cursor.
    /// `additionalTextEdits` have no cursor of their own and are applied once,
    /// document-wide. Both land as one undo step — gen-checked against
    /// `generation_at_begin`.
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
        edits::checked_buffer(state, self.bid, Some(self.generation_at_begin))?;
        let encoding = introspect::encoding_for_buffer(state, lsp, self.bid);

        let pid = state.focused_pane_id;
        pane_state::ensure(&mut state.panes.state, &state.buffers, pid, self.bid);
        let head_now = state.panes.state[pid][self.bid].selections.primary().head();

        let (start_now, end_now, new_text) = {
            let text = state.buffers.get(self.bid).text();
            match &item.text_edit {
                Some(te) => {
                    let rope_at_begin = &self.rope_at_begin;
                    let start_b = wire_to_char(
                        rope_at_begin,
                        te.range.start.line as usize,
                        te.range.start.character as usize,
                        encoding,
                    );
                    let end_b = wire_to_char(
                        rope_at_begin,
                        te.range.end.line as usize,
                        te.range.end.character as usize,
                        encoding,
                    );
                    // `self.anchor` already reflects every keystroke since
                    // `begin` (via `remap_anchor`); the server's range was
                    // decoded against the frozen `rope_at_begin` above and
                    // doesn't move on its own, so shifting it by the same
                    // amount the anchor has drifted keeps it pinned to the
                    // token, as if decoded fresh against the live document.
                    let drift = self.anchor as isize - self.anchor_at_begin as isize;
                    let start_now = (start_b as isize + drift).max(0) as usize;
                    let end_from_anchor_now = (end_b as isize + drift).max(0) as usize;
                    // Only extend, never shrink: characters typed since
                    // begin (further narrowing the filter) sit just past the
                    // server's own end and must be replaced too, or they
                    // survive verbatim next to the inserted text.
                    let cursor_now = self.anchor + self.filter.chars().count();
                    let end_now = cursor_now.max(end_from_anchor_now);
                    (start_now, end_now, te.new_text.clone())
                }
                None => {
                    // No server-provided range: replace the whole identifier
                    // token, not just the anchor..cursor span — any prefix
                    // typed *before* triggering completion (e.g. "fo" before
                    // the popup opened) sits before `anchor` and is
                    // otherwise left untouched, duplicating it ahead of
                    // `insert_text`.
                    let start_now = word_start_before(text, self.anchor);
                    let end_now = self.anchor + self.filter.chars().count();
                    (start_now, end_now, item.insert_text.clone())
                }
            }
        };
        // Re-expressed relative to the primary's own live head so the same
        // (back, forward) pair replays at every cursor, `replace_around_
        // cursors`-style. `saturating_sub` only clamps for an off-spec
        // server range that doesn't contain the request position — a
        // conforming server's never does that (LSP spec), so `start_now <=
        // head_now <= end_now` in the reachable, spec-conforming case.
        let back = head_now.saturating_sub(start_now);
        let forward = end_now.saturating_sub(head_now);

        // Captured before any edit lands — a resolve response (if one ends
        // up sent below) is computed against this exact pre-accept document,
        // and its wire positions must be decoded against it, not whatever
        // the buffer holds once the response actually arrives.
        let rope_pre = state.buffers.get(self.bid).text().rope().clone();

        // Insert mode already has a group open (composing this accept into
        // the ongoing session); a Steel-triggered accept outside Insert mode
        // does not, so open one here — both edits below then land as one
        // undo step regardless of caller.
        let opened_group = state.panes.state[pid][self.bid].edit_group.is_none();
        if opened_group {
            doc_ops::begin_edit_group(&state.buffers, &mut state.panes.state, pid, self.bid);
        }

        // additionalTextEdits have no cursor of their own — document-level,
        // applied first so the cursor edit below reads live selections
        // already shifted across them, not the pre-edit positions.
        //
        // Validation (overlap/reversed-range checks) happens before any
        // mutation, so a rejected batch here leaves the buffer untouched —
        // but a group opened just above would otherwise leak, still open
        // and empty, for the next edit to wrongly compose into. Commit it
        // (a no-op: `commit_edit_group` skips recording when nothing was
        // ever composed in) before propagating the error.
        let cs_additional = if item.additional_text_edits.is_empty() {
            None
        } else {
            match edits::apply_text_edits_returning_cs(
                state,
                lsp,
                self.bid,
                item.additional_text_edits.clone(),
                Some(self.generation_at_begin),
            ) {
                Ok(cs) => Some(cs),
                Err(e) => {
                    if opened_group {
                        doc_ops::commit_edit_group(
                            &mut state.buffers,
                            &mut state.panes.state,
                            pid,
                            self.bid,
                        );
                    }
                    return Err(e);
                }
            }
        };

        let cs_cursors = doc_ops::apply_doc_edit_grouped(
            &mut state.buffers,
            &state.config.decorations,
            &mut state.panes.state,
            pid,
            self.bid,
            move |b, s| replace_around_cursors(b, s, back, forward, &new_text),
        );

        if opened_group {
            doc_ops::commit_edit_group(&mut state.buffers, &mut state.panes.state, pid, self.bid);
        }

        // The full pre-accept-document → post-accept-document transform —
        // `maybe_send_resolve` needs it composed, not just the cursor edit's
        // own half, to map a resolve response's positions forward correctly.
        let accept_cs = match cs_additional {
            Some(cs_a) => cs_a.compose(cs_cursors),
            None => cs_cursors,
        };

        // Fire on-completion-accept with the raw (pristine) item after the
        // edit lands — an extension point for anything this store doesn't
        // parse (e.g. `command`); Rust now owns additionalTextEdits/resolve.
        let bid_val = hume_scripting::SteelBufferId::new(self.bid).into_steel_val();
        let item_val = json_to_steel(&item.raw);
        state
            .config
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
        let Some(id) =
            lsp.send_request(server_id, "completionItem/resolve", item.raw.clone(), meta)
        else {
            return; // server gone between the capability check and now
        };
        let callback: super::LspCallback = Box::new(move |editor, outcome| match outcome {
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
/// callers hold that separately: `Editor` via `state.completion_menu_view`,
/// `EditorHostImpl` via its own disjoint `state` borrow). Single definition
/// of "what constitutes an open completion session", shared by
/// `clear_completion_menu` and `completion_accept`.
pub(crate) fn clear_completion_state(lsp: &mut LspState) {
    lsp.completion = None;
    lsp.completion_ui = None;
}

/// Ends any open completion session and clears its menu view — the single
/// chokepoint for "close the completion menu", shared by `Editor` (via
/// `Editor::clear_completion_menu`), `EditorHostImpl`, and `picker::open_picker`
/// (opening a picker closes any live completion session first — one modal
/// owner at a time). `lsp`
/// is `None` at call sites that hold no `LspState` borrow — a no-op there,
/// same as when `lsp` is `Some` but no session is open. Always clears the
/// shared `completion_menu_view` Arc regardless of `lsp`.
pub(crate) fn clear_completion_menu(state: &mut EditorState, lsp: Option<&mut LspState>) {
    if let Some(lsp) = lsp {
        clear_completion_state(lsp);
    }
    *state
        .completion_menu_view
        .write()
        .expect("RwLock not poisoned") = None;
}

impl Editor {
    // ── LSP completion menu ─────────────────────────────────────────────

    /// Ends any open completion session and clears its menu view — shared
    /// by every completion-key handler in `mappings/insert.rs` (`Esc`, a
    /// Backspace crossing the anchor, a successful/failed accept) and by
    /// `take_pending_lsp_completion_dismiss`. A no-op when no session is
    /// open.
    pub(crate) fn clear_completion_menu(&mut self) {
        clear_completion_menu(&mut self.state, Some(&mut self.lsp));
    }

    /// Consumes `set_mode`'s deferred dismissal, if one is pending — called
    /// at every chokepoint between "a mode change could have happened" and
    /// "the next render" (see the flag's own doc comment on `EditorState`).
    pub(crate) fn take_pending_lsp_completion_dismiss(&mut self) {
        if std::mem::take(&mut self.state.lsp_completion_dismiss_pending) {
            self.clear_completion_menu();
        }
    }
}

#[cfg(test)]
mod tests;
