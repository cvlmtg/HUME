//! Completion orchestration: a Rust store holds the server's items and
//! does the per-keystroke filter/rank; Steel drives `begin!`/
//! `update-filter!`/`top`/`accept!`/`dismiss!`. One singleton session per
//! editor (not per buffer) — starting a new one replaces the old.

mod accept;
mod item;

use hume_editing::changeset::{Assoc, ChangeSet};
use hume_engine::pipeline::{BufferId, PaneId};

use super::LspState;
use crate::editor::{Editor, EditorState};
use crate::lock_ext::LockExt;

pub(crate) use item::StoredCompletionItem;

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
    /// Pane the session began in — `accept` only proceeds while this pane is
    /// still focused. A completion resolved against a pane the user has
    /// since navigated away from has no well-defined live cursor to land at,
    /// and `PaneBufferState`'s own `ensure` would otherwise silently
    /// fabricate one (see `accept`'s pane precondition).
    pane_id: PaneId,
    /// `anchor()`'s value at `begin()` time — paired with `rope_at_begin` as
    /// the coordinate system a server's `textEdit` range was computed
    /// against. Unlike the derived `anchor()`, never remapped: it's a fixed
    /// reference point, not a position tracked through edits.
    anchor_at_begin: usize,
    /// The buffer's rope at `begin()` time — an O(1) clone (ropey is
    /// structurally shared). A server's wire `textEdit` range is computed
    /// against the document as it stood at the completion *request*, which
    /// is this snapshot, not whatever the buffer holds by `accept()` time:
    /// if an earlier cursor on the same line has since inserted text (only
    /// possible when the primary isn't the first cursor), decoding the
    /// server's range against the live rope would land on the wrong chars.
    rope_at_begin: ropey::Rope,
    /// Every edit observed on this session's buffer since `begin` (via
    /// `observe_edit`), composed into one changeset — the single source of
    /// truth for "where a begin-time position sits now." Paired with
    /// `rope_at_begin`, this is the coordinate transform a server's wire
    /// positions (computed against the request document) need in order to
    /// land correctly on the live document: decode once against the frozen
    /// snapshot, then map forward through every keystroke since, rather than
    /// approximating drift as a scalar shift.
    cs_since_begin: ChangeSet,
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
    /// drifts as the user types further into the token). Derived by mapping
    /// `anchor_at_begin` forward through every edit observed so far —
    /// `Assoc::Before`: the anchor marks the token's start, so text inserted
    /// exactly at it belongs to the token and the anchor must stay left of
    /// it, same association `apply_doc_edit_grouped` uses for
    /// `pinned_anchors`.
    pub(crate) fn anchor(&self) -> usize {
        let mut positions = [self.anchor_at_begin];
        self.cs_since_begin
            .map_positions(&mut positions, Assoc::Before);
        positions[0]
    }

    /// Records an Insert-mode edit that landed on this session's buffer —
    /// called after every keystroke that lands in the buffer while this
    /// session is open, not just ones at the primary cursor. Without this, a
    /// keystroke at a cursor *before* the primary (multi-cursor Insert mode)
    /// shifts the primary head by more than one char while `anchor()` stays
    /// put, and `refilter_lsp_completion_after_edit`'s `slice(anchor..head)`
    /// picks up the drifted text.
    ///
    /// Returns `false` — leaving `cs_since_begin` untouched — when `cs`
    /// wasn't produced against this session's own tracked document length
    /// (`cs.len_before() != cs_since_begin`'s `len_after()`): an edit reached
    /// the buffer through a path this session never observed, which
    /// `ChangeSet::compose` would otherwise turn into a hard panic (its
    /// `len_before`/`len_after` check is a release `assert_eq!`, not a
    /// `debug_assert!`). The caller must dismiss the session in that case —
    /// there's no shorter edit history to fall back to.
    pub(crate) fn observe_edit(&mut self, cs: &ChangeSet) -> bool {
        if cs.len_before() != self.cs_since_begin.len_after() {
            return false;
        }
        self.cs_since_begin = self.cs_since_begin.clone().compose(cs.clone());
        true
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
        let rope_at_begin = state.buffers.get(bid).text().rope().clone();
        let mut session = Self {
            bid,
            pane_id: pid,
            anchor_at_begin: anchor,
            cs_since_begin: ChangeSet::identity(rope_at_begin.len_chars()),
            rope_at_begin,
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
    *state.completion_menu_view.write_unpoisoned() = None;
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
