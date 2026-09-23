//! The one open completion session: a Rust store holding what every
//! participating source has answered, ranked per keystroke against each
//! source's *own* token. One singleton session per editor (not per buffer);
//! `orchestrate.rs` opens it, invokes sources into it, feeds their answers
//! back, and tells it about edits — this file knows nothing about *how* a
//! source runs, only what it said and where.
//!
//! One session type serves both completion targets ([`Target`]) — the
//! accept mechanism differs (a buffer edit vs. a splice into the `:`
//! line's input), but ranking and the menu do not. Each target's slots live
//! *inside* its own [`Target`] variant, typed against that target's own id
//! and span shape ([`BufferSourceId`]/[`BufferSpan`] vs.
//! [`MinibufSourceId`]/[`MinibufSpan`]) — there is no third, mismatched
//! combination a caller could construct, unlike a flat span enum that has
//! to be re-checked against the target at every read.
//!
//! Every fact about a source's answer is per-[`Invocation`], never
//! session-wide: the document snapshot its `textEdit` ranges were computed
//! against, the token span it answered for, its items, its `isIncomplete`
//! flag. A second source, or the same source re-invoked against a later
//! document (the LSP `isIncomplete` flow), gets its own — so nothing here
//! has to assume every contributor saw the same buffer.

mod accept;

use std::ops::Range;

use hume_editing::changeset::{Assoc, ChangeSet, PosMapCursor};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::editor::fuzzy::{FuzzyMatcher, FuzzyProfile};
use crate::editor::widget_token;

use super::item::CompletionItem;
use super::registry::{BufferSourceId, MinibufSourceId, SourceRegistry};

/// How a source's items are matched against its token's typed text — a
/// per-source declaration (`registry.rs`), since one session mixes sources
/// with different universes.
///
/// - `Fuzzy` — nucleo scoring (`FuzzyMatcher`). The source's candidate
///   universe is stable (or replaced wholesale by a re-invocation, the LSP
///   `isIncomplete` flow); [`CompletionSession::rank`] re-scores it locally
///   on every keystroke without re-invoking the source.
/// - `String { case_sensitive }` — a boundary-safe prefix gate (`starts_with`,
///   or `eq_ignore_ascii_case` on the matching-length head when
///   `case_sensitive` is `false`), tied score on a match. The source's
///   universe is *also* stable (e.g. "every registered command name") — only
///   the matching rule differs from `Fuzzy`. A `String` match's tied score
///   is always `0`, never above a `Fuzzy` match's own score once anything is
///   typed (nucleo scores every non-empty match above `0`) — deliberate, not
///   a gap: in a buffer with an attached LSP server, its `Fuzzy` items should
///   win once the user narrows by typing, and a `String`-kind source (e.g.
///   `core:buffer-words`) earns its keep where `Fuzzy` sources answer
///   nothing at all (a comment, a string literal, a plain-text buffer with
///   no server) — `#:priority` only ever breaks a tie on the *empty*
///   pattern, where every source scores `0` alike. See `rank`'s own
///   `MatchKind::String` arm.
/// - `Delegated` — the source computed its own finished, already-ordered
///   result fresh from the live input (a directory read, a multi-phase
///   parse); this session does no scoring of its own for these items: a
///   tied score, and `rank`'s own sort key skips the sortText tiebreak for
///   a `Delegated` slot entirely, so the final index-ascending tiebreak
///   preserves the source's own order regardless of what `sort_text` an
///   item happens to carry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// Where an accepted item lands, and each target's own slots — a `Buffer`
/// session refilters the open menu in place as the user types; a `Minibuf`
/// session dismisses on any key but Tab, which only cycles the list already
/// computed (see `CompletionLayer::handler`'s doc). Slots live inside the
/// variant that owns their id/span types, so `rank`/`observe_edit`/`accept`
/// match this enum exactly once and never have to reconcile a slot's own
/// span shape against which arm they're in.
pub(in crate::editor) enum Target {
    Buffer {
        bt: BufferTarget,
        slots: Vec<SourceSlot<BufferSourceId, BufferSpan>>,
    },
    Minibuf {
        mt: MinibufTarget,
        slots: Vec<SourceSlot<MinibufSourceId, MinibufSpan>>,
    },
}

pub(in crate::editor) struct BufferTarget {
    bid: BufferId,
    /// Pane the session began in — `accept` only proceeds while this pane is
    /// still focused. A completion resolved against a pane the user has
    /// since navigated away from has no well-defined live cursor to land at,
    /// and `PaneBufferState`'s own `ensure` would otherwise silently
    /// fabricate one (see `accept`'s pane precondition).
    pane_id: PaneId,
    /// Buffer generation as of the last edit this session observed via
    /// [`CompletionSession::observe_edit`] — `accept` rejects if the buffer
    /// changed by any other path since.
    generation: u64,
    /// The buffer's length as of that same edit — what the next observed
    /// `ChangeSet`'s `len_before` must equal, or an edit reached the buffer
    /// through a path this session never saw.
    len: usize,
    /// Whether an explicit `Trigger::Explicit` (Ctrl-Space) has touched
    /// this session, as opposed to only ever a trigger char — gates the
    /// settle-time "no completions" report (`orchestrate.rs`'s
    /// `contribute`): silent for a session the user never explicitly asked
    /// anything of, reported once they did and got nothing. `false` at
    /// open; [`CompletionSession::mark_explicit_trigger`] sets it, never
    /// unset.
    explicit: bool,
    /// The leftmost live token start among the slots [`CompletionSession::
    /// rank`] just gave at least one ranked candidate — folded into that
    /// same per-slot pass rather than recomputed by a second walk over
    /// the ranked list on every render frame. `None` with nothing ranked.
    menu_anchor: Option<CharOffset>,
}

impl BufferTarget {
    pub(in crate::editor) fn bid(&self) -> BufferId {
        self.bid
    }

    /// Whether this target's pane/buffer/generation still match live state
    /// — `Editor::dismiss_invalid_completion`'s settle-time check. A coarse
    /// yes/no, unlike `accept`'s own preconditions (`checked_buffer`, the
    /// focus and pane-shows-buffer checks in `accept.rs`), which stay
    /// separate because they each need their own distinct Steel-facing
    /// error message; this one only ever feeds a silent background dismiss.
    pub(in crate::editor) fn still_valid(&self, state: &EditorState, view: &EngineView) -> bool {
        state.focus.id() == self.pane_id
            && view.panes.get(self.pane_id).map(|p| p.buffer_id) == Some(self.bid)
            && state.buffers.try_get(self.bid).map(|b| b.text_gen) == Some(self.generation)
    }
}

/// The `:` line as every source saw it. Restored verbatim before each
/// cycle-apply (`orchestrate.rs`), so applying candidate *k* over a slot's
/// span is idempotent in these coordinates — no "what did the previous
/// candidate leave behind" bookkeeping, and two sources with different
/// spans coexist by construction.
pub(in crate::editor::completion) struct MinibufTarget {
    input: String,
    cursor: usize,
    /// [`BufferTarget::menu_anchor`]'s counterpart, a byte offset into
    /// `input`.
    menu_anchor: Option<usize>,
}

/// The coordinate system one invocation's answer was computed in: the rope
/// at invoke time (an O(1) clone — ropey is structurally shared), the
/// cursor then, and every edit observed since, composed. A server's wire
/// `textEdit` range is computed against the document as it stood at the
/// *request*, which is this snapshot; `accept` decodes against `rope` and
/// maps forward through `cs_since` rather than approximating drift as a
/// scalar shift.
struct DocSnapshot {
    rope: ropey::Rope,
    cs_since: ChangeSet,
}

/// The token in live coordinates that a `Buffer` invocation answered for —
/// fixed at invoke time (the source's token rule: the word before the
/// cursor) and tracked through every edit since via [`Invocation::observe`].
/// `pub(in crate::editor)`, not private: `orchestrate.rs` names
/// `Invocation<BufferSpan>` at every `invoke_buffer` call site, the same way
/// it already names `BufferTarget`.
pub(in crate::editor) struct BufferSpan {
    doc: DocSnapshot,
    /// `start` mapped `Assoc::Before` through every observed edit (text
    /// inserted exactly at the token's start belongs to the token), `end`
    /// mapped `Assoc::After` (text typed at the token's end extends it).
    live: Range<CharOffset>,
}

/// Byte range in [`MinibufTarget::input`] that a `Minibuf` invocation
/// answered for. Never moves — nothing can edit the `:` line while a
/// session is open without dismissing it. `pub(in crate::editor)` for the
/// same reason as [`BufferSpan`].
pub(in crate::editor) struct MinibufSpan {
    bytes: Range<usize>,
}

enum InvocationState {
    Pending,
    Shown {
        items: Vec<CompletionItem>,
        incomplete: bool,
    },
}

/// One call of one source for one trigger: the id the source answers to,
/// the span it was asked about, and its answer once it has one. Minted by
/// `orchestrate.rs` against the live document, stored here. Generic over
/// the span shape ([`BufferSpan`]/[`MinibufSpan`]) — each [`Target`]
/// variant's slots fix it to their own target's shape, so there is no
/// runtime tag to mismatch.
pub(in crate::editor) struct Invocation<S> {
    id: u64,
    span: S,
    state: InvocationState,
}

impl<S> Invocation<S> {
    fn items(&self) -> &[CompletionItem] {
        match &self.state {
            InvocationState::Shown { items, .. } => items,
            InvocationState::Pending => &[],
        }
    }

    fn incomplete(&self) -> bool {
        matches!(
            self.state,
            InvocationState::Shown {
                incomplete: true,
                ..
            }
        )
    }
}

impl Invocation<BufferSpan> {
    /// A `Buffer`-target invocation. `live` is the word before the cursor —
    /// every buffer source's token rule — so `live.end` *is* the cursor at
    /// invoke time; there is no separate "head" to track alongside it.
    pub(in crate::editor) fn buffer(rope: ropey::Rope, live: Range<CharOffset>) -> Self {
        let cs_since = ChangeSet::identity(rope.len_chars());
        Self {
            id: widget_token::next(),
            span: BufferSpan {
                doc: DocSnapshot { rope, cs_since },
                live,
            },
            state: InvocationState::Pending,
        }
    }

    /// The seeded filter text for this invocation — `text[live.start ..
    /// live.end]` as of the invoke — handed to a Steel source as its
    /// `prefix` argument.
    pub(in crate::editor) fn prefix(&self, text: &BufferText) -> String {
        token_text(text, self.span.live.start, self.span.live.end)
    }

    /// Composes `cs` into this invocation's snapshot and remaps its live
    /// span. Returns `false` when the edit crossed the token's start — see
    /// [`CompletionSession::observe_edit`]. `text`/`chars` are the *live*
    /// (post-`cs`) document and this buffer's word-chars, needed only to
    /// classify a newly-included end-of-token slice — see `end`'s own
    /// comment below.
    fn observe(&mut self, cs: &ChangeSet, text: &BufferText, chars: WordChars<'_>) -> bool {
        let BufferSpan { doc, live: range } = &mut self.span;
        // One cursor over the non-decreasing sequence `[start-1, start,
        // end]` — `map_anchor`'s `anchor_deleted` on `start-1` answers "was
        // the token's start character itself deleted?" (a Backspace at the
        // token's own start) directly, rather than inferring it from two
        // positions mapping to the same spot.
        let mut cursor = PosMapCursor::new(cs.ops());
        let crossed = if range.start > CharOffset::new(0) {
            cursor
                .map_anchor(range.start.retreat(1), Assoc::Before)
                .anchor_deleted
        } else {
            false
        };
        let start = cursor.map_anchor(range.start, Assoc::Before).pos;
        // Two calls on the same, already-visited position — safe:
        // `PosMapCursor::map_anchor` only advances its own state past an
        // op once a *later* position is queried, so re-querying `range.end`
        // immediately after `start` (itself `<= range.end`) never violates
        // the cursor's own non-decreasing-queries contract. The two only
        // ever disagree when an `Insert` op sits exactly at `range.end`:
        // `Assoc::Before` stops short of it, `Assoc::After` extends past it.
        let end_before = cursor.map(range.end, Assoc::Before);
        let end_after = cursor.map(range.end, Assoc::After);
        // An insertion exactly at the *non-empty* token's end only extends
        // the tracked span when every newly-included char is itself
        // word-class — matching the token's own definition ("the word
        // before the cursor"). Unconditionally taking `end_after` let an
        // auto-paired bracket (or any other non-word char) landing there
        // silently join the token: `accept`'s containment check then
        // trivially succeeds once the live head sits exactly at the grown
        // boundary, masking the "cursor left the token" case this span
        // exists to detect. An *empty* token (`range` was already
        // zero-width — no word typed yet, matching every candidate) has no
        // word for a non-word char to violate, so it stays "live" (chasing
        // the cursor) unconditionally until something more definite ends
        // it (a start-side Backspace, an out-of-band cursor jump), the
        // same as any other source not yet narrowed by typing.
        let end = if range.start == range.end
            || (end_after > end_before
                && token_text(text, end_before, end_after)
                    .chars()
                    .all(|c| chars.classify(c) == CharClass::Word))
        {
            end_after
        } else {
            end_before
        };
        *range = start..end;
        // `compose` takes `self` by value; swap the accumulated changeset
        // out (rather than cloning its whole op list, including every
        // typed-text `Insert` op, just to feed it in) and compose in place.
        let prev = std::mem::replace(&mut doc.cs_since, ChangeSet::identity(0));
        doc.cs_since = prev.compose(cs.clone());
        !crossed
    }

    /// Whether `head` is still inside this invocation's live token.
    fn contains(&self, head: CharOffset) -> bool {
        contains_cursor(&self.span.live, head)
    }
}

impl Invocation<MinibufSpan> {
    /// A `Minibuf`-target invocation. `bytes` is the whitespace-delimited
    /// argument the cursor is in — every minibuffer source's token rule,
    /// except a `NativeDelegated` one, which computes its own span
    /// synchronously before this is minted (`orchestrate.rs`'s
    /// `invoke_minibuf_source`).
    pub(in crate::editor) fn minibuf(bytes: Range<usize>) -> Self {
        Self {
            id: widget_token::next(),
            span: MinibufSpan { bytes },
            state: InvocationState::Pending,
        }
    }
}

/// One participating source: the answer currently ranked (`shown`) and, if
/// the source was re-invoked since, the newer call whose answer hasn't
/// landed yet (`inflight`). An emission applies only to the *latest* of the
/// two ids — a late answer to a superseded call is dropped, so a slow LSP
/// response can never overwrite a newer one. Generic over the id type
/// ([`BufferSourceId`]/[`MinibufSourceId`]) and span shape, both fixed by
/// which [`Target`] variant holds this slot.
pub(in crate::editor) struct SourceSlot<Id, S> {
    source: Id,
    shown: Option<Invocation<S>>,
    inflight: Option<Invocation<S>>,
}

impl<Id: Copy + PartialEq, S> SourceSlot<Id, S> {
    fn latest_id(&self) -> Option<u64> {
        self.inflight
            .as_ref()
            .or(self.shown.as_ref())
            .map(|inv| inv.id)
    }

    /// Whether the source has answered with at least one item — a slot
    /// narrowed to zero *matches* is still live (Backspace can bring its
    /// items back); one that answered empty, or whose token the cursor
    /// left, is not.
    fn is_live(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|inv| !inv.items().is_empty())
    }

    /// The item at ranked index `i` of this slot's shown answer — every
    /// caller already knows `i` came from a still-valid `rank_scratch`
    /// entry (see `CompletionSession::ranked_indices`), so this is
    /// `.expect`, not `Option`.
    fn item(&self, i: usize) -> &CompletionItem {
        &self.shown.as_ref().expect("ranked").items()[i]
    }
}

/// Records a fresh call of `source` into `slots`, superseding any still in
/// flight for it. Returns the id the answer must carry. Shared by
/// [`CompletionSession::invoke_buffer`]/[`CompletionSession::invoke_minibuf`]
/// — identical bookkeeping for either target; only the id/span types differ.
fn invoke_into<Id: Copy + PartialEq, S>(
    slots: &mut Vec<SourceSlot<Id, S>>,
    source: Id,
    invocation: Invocation<S>,
) -> u64 {
    let id = invocation.id;
    let slot = match slots.iter().position(|s| s.source == source) {
        Some(i) => &mut slots[i],
        None => {
            slots.push(SourceSlot {
                source,
                shown: None,
                inflight: None,
            });
            slots.last_mut().expect("just pushed")
        }
    };
    slot.inflight = Some(invocation);
    id
}

/// Lands an answer for invocation `id` into `slots`. `false` when `id`
/// isn't the latest call of any slot here — a superseded or
/// already-replaced invocation, expected-normal for a late async source,
/// never an error. Shared by both [`Target`] arms of
/// [`CompletionSession::contribute`].
fn contribute_into<Id: Copy + PartialEq, S>(
    slots: &mut [SourceSlot<Id, S>],
    id: u64,
    items: Vec<CompletionItem>,
    incomplete: bool,
) -> bool {
    let Some(slot) = slots.iter_mut().find(|s| s.latest_id() == Some(id)) else {
        return false;
    };
    let mut invocation = match slot.inflight.take() {
        Some(inv) => inv,
        None => slot
            .shown
            .take()
            .expect("latest_id came from one of the two"),
    };
    invocation.state = InvocationState::Shown { items, incomplete };
    slot.shown = Some(invocation);
    true
}

fn any_pending<Id: Copy + PartialEq, S>(slots: &[SourceSlot<Id, S>]) -> bool {
    slots.iter().any(|s| s.inflight.is_some())
}

fn any_live<Id: Copy + PartialEq, S>(slots: &[SourceSlot<Id, S>]) -> bool {
    slots.iter().any(SourceSlot::is_live)
}

/// [`CompletionSession::rank`]'s own scratch state, reborrowed disjointly
/// from `self` (see `rank`'s own destructure) and threaded through
/// [`score_slot`] as one bundle rather than three separate parameters —
/// every one of the three is reused across every slot of both `Target`
/// arms, unlike `score_slot`'s other, per-slot arguments.
struct ScoreCtx<'a> {
    matcher: &'a mut FuzzyMatcher,
    rank_scratch: &'a mut Vec<(u32, i64, u32, u32)>,
    dedup_hidden: &'a rustc_hash::FxHashSet<(u32, u32)>,
}

/// Scores every item in `items` against `filter` per `match_kind`, dropping
/// a no-op item first, and pushes `(score, priority, s, i)` into
/// `ctx.rank_scratch` for each survivor. Returns whether anything survived
/// — [`CompletionSession::rank`]'s own signal to fold this slot's token
/// start into the menu anchor. Shared by the `Buffer` and `Minibuf` arms of
/// `rank`, which differ only in how they compute `filter` and their own
/// anchor's unit.
fn score_slot(
    ctx: &mut ScoreCtx<'_>,
    s: u32,
    items: &[CompletionItem],
    filter: &str,
    match_kind: MatchKind,
    priority: i64,
) -> bool {
    let pattern = ctx.matcher.parse(filter);
    let mut contributed = false;
    for (i, item) in items.iter().enumerate() {
        if item.is_noop_for(filter) {
            continue;
        }
        if ctx.dedup_hidden.contains(&(s, i as u32)) {
            continue;
        }
        let score = match match_kind {
            MatchKind::Fuzzy => ctx.matcher.score(&pattern, &item.filter_text),
            MatchKind::String { case_sensitive } => {
                prefix_matches(&item.filter_text, filter, case_sensitive).then_some(0)
            }
            // The source already produced a finished, ordered result —
            // never excluded here; the rank key's tiebreak chain (empty
            // `sort_text`, see the item constructor these sources use)
            // preserves that order via the final index-ascending key.
            MatchKind::Delegated => Some(0),
        };
        if let Some(score) = score {
            ctx.rank_scratch.push((score, priority, s, i as u32));
            contributed = true;
        }
    }
    contributed
}

/// The document a `Buffer` session's tokens are read against when ranking
/// — the live text and the primary cursor's head.
pub(in crate::editor) struct LiveDoc<'a> {
    pub(in crate::editor) text: &'a BufferText,
    pub(in crate::editor) head: CharOffset,
}

pub(in crate::editor) struct CompletionSession {
    target: Target,
    /// `(score, priority, slot, item)` for every surviving candidate,
    /// rebuilt and sorted by every [`Self::rank`] call — this *is* the
    /// session's own ranked list (`len`/`ranked_indices`/`rows_in` all read
    /// it directly). Retained across calls so per-keystroke filtering
    /// doesn't allocate a fresh Vec every time. `priority` is
    /// `BufferSourceEntry`/`MinibufSourceEntry`'s own `i64`, not narrowed —
    /// this tuple is sorted, never used as a lookup key, so there's no
    /// reason to risk a truncating cast.
    rank_scratch: Vec<(u32, i64, u32, u32)>,
    /// Reusable scoring engine — `FuzzyProfile::Autocomplete` (see its doc)
    /// distinguishes this from the picker's own instance. One instance per
    /// session, consulted only for a `MatchKind::Fuzzy` slot.
    matcher: FuzzyMatcher,
    /// `(slot, item)` pairs a lower-priority `Buffer` slot's plain item
    /// (see [`CompletionItem::is_plain`]) is hidden because a
    /// strictly-higher-priority slot already shows an item with the same
    /// `filter_text` — rebuilt by [`Self::recompute_dedup`] only when the
    /// shown item *set* changes (an answer lands, a slot is dropped), not
    /// every keystroke; [`Self::rank`] only ever reads it. Always empty for
    /// a `Minibuf` session — it invokes exactly one source, so there is
    /// never a second slot to dedup against.
    dedup_hidden: rustc_hash::FxHashSet<(u32, u32)>,
}

/// UI state for an open completion session — kept separate from
/// `CompletionSession` itself (which deliberately has no `selected`) so the
/// session's filtering/accept logic stays free of rendering concerns.
/// Shared by both targets — the menu-navigation keys (Tab/Down/BackTab/Up)
/// are already target-agnostic.
pub(in crate::editor) struct CompletionMenuUi {
    pub(in crate::editor) selected: usize,
}

impl CompletionSession {
    fn new(target: Target) -> Self {
        Self {
            target,
            rank_scratch: Vec::new(),
            matcher: FuzzyMatcher::new(FuzzyProfile::Autocomplete),
            dedup_hidden: rustc_hash::FxHashSet::default(),
        }
    }

    /// A `Buffer`-target session on `bid`, shown in `pane_id`, whose text is
    /// `len` chars at generation `generation` — the state the first
    /// [`Self::observe_edit`] checks against.
    pub(in crate::editor) fn open_buffer(
        bid: BufferId,
        pane_id: PaneId,
        generation: u64,
        len: usize,
    ) -> Self {
        Self::new(Target::Buffer {
            bt: BufferTarget {
                bid,
                pane_id,
                generation,
                len,
                explicit: false,
                menu_anchor: None,
            },
            slots: Vec::new(),
        })
    }

    /// Whether an explicit `Trigger::Explicit` (Ctrl-Space) has touched
    /// this session — see [`BufferTarget::explicit`]'s doc. `false` for a
    /// `Minibuf` session; this axis only applies to `Buffer` ones.
    pub(in crate::editor) fn is_explicit(&self) -> bool {
        match &self.target {
            Target::Buffer { bt, .. } => bt.explicit,
            Target::Minibuf { .. } => false,
        }
    }

    /// Records that an explicit trigger touched this session — called by
    /// `trigger_buffer_completion` on both the fresh-open and the
    /// reused-session path, so a session a trigger char opened still
    /// reports once the user follows up with Ctrl-Space. A no-op for a
    /// `Minibuf` session (unreachable: only a `Buffer` trigger calls this).
    pub(in crate::editor) fn mark_explicit_trigger(&mut self) {
        if let Target::Buffer { bt, .. } = &mut self.target {
            bt.explicit = true;
        }
    }

    /// A `Minibuf`-target session over the `:` line's `input`, cursor at
    /// byte `cursor`.
    pub(in crate::editor) fn open_minibuf(input: String, cursor: usize) -> Self {
        Self::new(Target::Minibuf {
            mt: MinibufTarget {
                input,
                cursor,
                menu_anchor: None,
            },
            slots: Vec::new(),
        })
    }

    /// The `Buffer`-target fields, or `None` for a `Minibuf` session — the
    /// one way code outside this module reaches `BufferTarget`'s own
    /// accessors, so a `Minibuf` session can't be asked for a buffer-only
    /// fact by mistake: the compiler forces every caller to handle `None`
    /// rather than trusting an `.expect()` that a `Minibuf` session never
    /// reaches it.
    pub(in crate::editor) fn buffer(&self) -> Option<&BufferTarget> {
        match &self.target {
            Target::Buffer { bt, .. } => Some(bt),
            Target::Minibuf { .. } => None,
        }
    }

    /// The `:` line as this `Minibuf` session's sources saw it — `None` for
    /// a `Buffer` session. The one way code outside this module learns
    /// which target a session has, since `Target` itself stays private.
    pub(in crate::editor) fn minibuf_input(&self) -> Option<&str> {
        match &self.target {
            Target::Minibuf { mt, .. } => Some(&mt.input),
            Target::Buffer { .. } => None,
        }
    }

    // ── Sources in and out ───────────────────────────────────────────────────

    /// Records a fresh call of a `Buffer` source, superseding any still in
    /// flight for it — see [`invoke_into`]. `None` on a `Minibuf` session:
    /// structurally unreachable (`invoke_buffer_sources`'s only two callers,
    /// `trigger_buffer_completion` and `completion_observe_edit`, both only
    /// ever hold a `Buffer`-target session by the time they call this), but
    /// there is no `CompletionSession::Buffer` subtype to prove it at the
    /// type level without splitting the session type the accept mechanism
    /// deliberately shares — see this module's own doc.
    pub(in crate::editor) fn invoke_buffer(
        &mut self,
        source: BufferSourceId,
        invocation: Invocation<BufferSpan>,
    ) -> Option<u64> {
        let Target::Buffer { slots, .. } = &mut self.target else {
            return None;
        };
        Some(invoke_into(slots, source, invocation))
    }

    /// [`Self::invoke_buffer`]'s `Minibuf` counterpart.
    pub(in crate::editor) fn invoke_minibuf(
        &mut self,
        source: MinibufSourceId,
        invocation: Invocation<MinibufSpan>,
    ) -> Option<u64> {
        let Target::Minibuf { slots, .. } = &mut self.target else {
            return None;
        };
        Some(invoke_into(slots, source, invocation))
    }

    /// Lands an answer for invocation `id` — see [`contribute_into`]. Then,
    /// if it landed, rebuilds [`Self::dedup_hidden`] against the item set
    /// as it now stands (a no-op for a `Minibuf` session — see that field's
    /// own doc).
    pub(in crate::editor) fn contribute(
        &mut self,
        sources: &SourceRegistry,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        let landed = match &mut self.target {
            Target::Buffer { slots, .. } => contribute_into(slots, id, items, incomplete),
            Target::Minibuf { slots, .. } => contribute_into(slots, id, items, incomplete),
        };
        if landed {
            self.recompute_dedup(sources);
        }
        landed
    }

    /// Whether any slot still awaits an answer.
    pub(in crate::editor) fn is_pending(&self) -> bool {
        match &self.target {
            Target::Buffer { slots, .. } => any_pending(slots),
            Target::Minibuf { slots, .. } => any_pending(slots),
        }
    }

    /// Clears every still-`inflight` invocation, across every slot of
    /// whichever target this session has — leaving any existing `shown`
    /// answer untouched. Returns whether anything was actually cleared.
    ///
    /// The recovery path for a Steel call batch that failed before any of
    /// its queued sources could reach `completion-emit!` (see
    /// `EditorState::settle_completion_after_call_failure`'s own call
    /// site): without this, a source that raises — or one that simply never
    /// answers — leaves its slot permanently `Pending`, since nothing else
    /// ever tells this session the call didn't happen. For a `Buffer`
    /// session that's a silent-but-real cost: `sources_to_reinvoke` calls
    /// it again on every subsequent edit, so a broken source errors on
    /// every keystroke instead of just once. For a `Minibuf` session
    /// (no edit-driven reinvocation loop — see `MinibufSpan`'s own doc)
    /// it's worse: `is_pending()` never becomes `false`, so
    /// `settle_minibuf_session` never promotes or dismisses the popup and
    /// every subsequent Tab is silently swallowed.
    ///
    /// Safe regardless of *why* the batch failed, or whether it even
    /// touched completion at all: a source that was merely slow gets asked
    /// again through the ordinary edit-driven reinvocation path (`Buffer`)
    /// or a fresh trigger (either target); one that's genuinely broken
    /// simply stops being asked until then, rather than erroring forever.
    pub(in crate::editor) fn drop_stalled_invocations(&mut self) -> bool {
        fn drop_in<Id, S>(slots: &mut [SourceSlot<Id, S>]) -> bool {
            let mut any = false;
            for slot in slots {
                if slot.inflight.take().is_some() {
                    any = true;
                }
            }
            any
        }
        match &mut self.target {
            Target::Buffer { slots, .. } => drop_in(slots),
            Target::Minibuf { slots, .. } => drop_in(slots),
        }
    }

    /// Whether any source has answered with at least one item — `false`
    /// once every answer is empty or the cursor left every token, at which
    /// point the session has nothing left to show and closes.
    fn has_live_sources(&self) -> bool {
        match &self.target {
            Target::Buffer { slots, .. } => any_live(slots),
            Target::Minibuf { slots, .. } => any_live(slots),
        }
    }

    /// Whether the session has nothing left to show and nothing on its way
    /// — every source answered empty, or the cursor typed out of every
    /// token. The one rule `orchestrate.rs` checks, at every point a
    /// session might have just run out: right after opening/re-invoking,
    /// and after each answer lands.
    pub(in crate::editor) fn is_spent(&self) -> bool {
        !self.is_pending() && !self.has_live_sources()
    }

    /// The `Buffer` sources to call again after an edit: those that flagged
    /// their latest answer `isIncomplete`, and those still pending (their
    /// in-flight call saw an older document, so it is superseded rather
    /// than waited for). `[]` for a `Minibuf` session — its only caller,
    /// `EditorState::completion_observe_edit`, is `Buffer`-only.
    pub(in crate::editor) fn sources_to_reinvoke(&self) -> Vec<BufferSourceId> {
        let Target::Buffer { slots, .. } = &self.target else {
            return Vec::new();
        };
        slots
            .iter()
            .filter(|s| {
                s.inflight.is_some() || s.shown.as_ref().is_some_and(Invocation::incomplete)
            })
            .map(|s| s.source)
            .collect()
    }

    // ── Edits ────────────────────────────────────────────────────────────────

    /// Records an Insert-mode edit that landed on this `Buffer` session's
    /// buffer — every keystroke, not just ones at the primary cursor (a
    /// keystroke at a cursor *before* the primary shifts every token). A
    /// no-op (`true`) for a `Minibuf` session — nothing can edit the `:`
    /// line while a session is open (see [`MinibufSpan`]'s doc), so this is
    /// never actually called against one. Composes `cs` into each
    /// invocation's `cs_since`, remaps each live span, and drops the answer
    /// of any slot whose token the cursor has left: `head` outside `[start,
    /// end]`, or the character *before* the token deleted (a Backspace at
    /// the token's start — detected by `start` and `start - 1` mapping to
    /// the same live position, which no edit elsewhere can cause).
    ///
    /// Returns `false` — session untouched — when `cs` wasn't produced
    /// against this session's own tracked document length: an edit reached
    /// the buffer through a path this session never observed, which
    /// `ChangeSet::compose` would otherwise turn into a hard panic (its
    /// `len_before`/`len_after` check is a release `assert_eq!`). The caller
    /// must dismiss the session in that case. `text_gen` is the buffer's
    /// generation *after* `cs` landed. `text`/`chars` are the live
    /// (post-`cs`) document and this buffer's word-chars — threaded through
    /// to [`Invocation::observe`], which needs them only to classify a
    /// newly-included end-of-token slice.
    pub(in crate::editor) fn observe_edit(
        &mut self,
        sources: &SourceRegistry,
        cs: &ChangeSet,
        text_gen: u64,
        head: CharOffset,
        text: &BufferText,
        chars: WordChars<'_>,
    ) -> bool {
        let Target::Buffer { bt, slots } = &mut self.target else {
            return true;
        };
        if cs.len_before() != bt.len {
            return false;
        }
        bt.len = cs.len_after();
        bt.generation = text_gen;
        // Whether any slot's `shown` answer was actually dropped below —
        // the one thing that can change which items dedup compares against,
        // so `recompute_dedup` runs only then, not on every edit.
        let mut dropped = false;
        for slot in slots {
            if let Some(inv) = &mut slot.inflight {
                inv.observe(cs, text, chars);
            }
            if let Some(inv) = &mut slot.shown
                && !inv.observe(cs, text, chars)
            {
                slot.shown = None;
                dropped = true;
            }
            if let Some(inv) = &slot.shown
                && !inv.contains(head)
            {
                slot.shown = None;
                dropped = true;
            }
        }
        if dropped {
            self.recompute_dedup(sources);
        }
        true
    }

    /// Rebuilds [`Self::dedup_hidden`] from the `Buffer` slots' current
    /// shown items: a plain item ([`CompletionItem::is_plain`]) is hidden
    /// when a strictly-higher-priority slot's shown answer has an item with
    /// the same `filter_text`. An item carrying edits is never hidden —
    /// accepting it does something a duplicate-*looking* plain item from
    /// another source wouldn't, so it stays regardless of what else
    /// duplicates its label. Priority is a static, per-source fact
    /// (`BufferSourceEntry::priority`), so this only ever needs to run when
    /// the shown item *set* changes ([`Self::contribute`] landing an
    /// answer, [`Self::observe_edit`] dropping a slot) — not on every
    /// keystroke, unlike scoring itself. A no-op for a `Minibuf` session
    /// (`dedup_hidden` stays empty — see that field's own doc).
    fn recompute_dedup(&mut self, sources: &SourceRegistry) {
        self.dedup_hidden.clear();
        let Target::Buffer { slots, .. } = &self.target else {
            return;
        };
        // The highest priority among every shown item (plain or not, any
        // slot) carrying a given `filter_text` — O(total items), not the
        // O(items²) an all-pairs scan across slots costs (a 20k-word
        // buffer-words slot against a 1k-item LSP slot is ~20M string
        // compares per landed answer with the naive version). A slot's own
        // priority never exceeds its own contribution to this map, so
        // below, a strict `>` against it already excludes comparing a slot
        // against itself — no separate index check needed.
        let mut max_priority: rustc_hash::FxHashMap<&str, i64> = rustc_hash::FxHashMap::default();
        for slot in slots {
            let Some(inv) = &slot.shown else { continue };
            let priority = sources.buffer_get(slot.source).priority;
            for item in inv.items() {
                max_priority
                    .entry(item.filter_text.as_str())
                    .and_modify(|p| *p = (*p).max(priority))
                    .or_insert(priority);
            }
        }
        for (s, slot) in slots.iter().enumerate() {
            let Some(inv) = &slot.shown else { continue };
            let priority = sources.buffer_get(slot.source).priority;
            for (i, item) in inv.items().iter().enumerate() {
                if !item.is_plain() {
                    continue;
                }
                let outranked = max_priority
                    .get(item.filter_text.as_str())
                    .is_some_and(|&p| p > priority);
                if outranked {
                    self.dedup_hidden.insert((s as u32, i as u32));
                }
            }
        }
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    /// Re-scores every shown item against its own slot's token text —
    /// `live[start..head]` for a `Buffer` session, `input[start..cursor]`
    /// for a `Minibuf` one — with its source's `MatchKind`, dropping any
    /// item that's a no-op against that text first (`CompletionItem::
    /// is_noop_for`) regardless of `MatchKind`, and any item
    /// [`Self::dedup_hidden`] already marked as a lower-priority duplicate —
    /// that set is rebuilt only when the shown items change, not here, so
    /// this is a cheap membership check, not a fresh cross-slot comparison
    /// every keystroke. Rank key: score descending,
    /// then source priority descending (a tiebreaker only — match quality
    /// stays king — applied before sortText so a higher-priority source's
    /// item wins a tie regardless of how its label sorts; direction matches
    /// `register_sign_source`'s own `(priority desc, name asc)`), then
    /// sortText ascending (the server's own ordering hint, the *only*
    /// signal left on an empty filter — nucleo scores every haystack `0`
    /// for an empty pattern), then slot and item index (sortText is very
    /// often duplicated across a server's items). Matches on `target` once
    /// (`score_slot` does the per-item work shared by both arms), so there
    /// is nowhere a slot's span shape could disagree with which arm scored
    /// it.
    pub(in crate::editor) fn rank(&mut self, sources: &SourceRegistry, live: Option<LiveDoc<'_>>) {
        let Self {
            target,
            rank_scratch,
            matcher,
            dedup_hidden,
        } = self;
        rank_scratch.clear();
        let mut ctx = ScoreCtx {
            matcher,
            rank_scratch,
            dedup_hidden: &*dedup_hidden,
        };
        // The leftmost token start among the slots that end up contributing
        // at least one scored item — folded into the same per-slot pass
        // rather than a second walk over `rank_scratch` every time a caller
        // needs it (`menu_anchor_char`/`menu_anchor_byte`, both on the
        // per-frame render path).
        match target {
            Target::Buffer { bt, slots } => {
                let mut anchor: Option<CharOffset> = None;
                if let Some(doc) = &live {
                    for (s, slot) in slots.iter().enumerate() {
                        let Some(inv) = &slot.shown else { continue };
                        let start = inv.span.live.start;
                        // A shown slot always contains `head` after
                        // `observe_edit` (see its doc), so an inverted
                        // range here can only mean an edit this session
                        // never saw — skipped rather than sliced, until the
                        // settle-time validity check dismisses the session.
                        if start > doc.head {
                            continue;
                        }
                        let filter = token_text(doc.text, start, doc.head);
                        let entry = sources.buffer_get(slot.source);
                        let contributed = score_slot(
                            &mut ctx,
                            s as u32,
                            inv.items(),
                            &filter,
                            entry.match_kind,
                            entry.priority,
                        );
                        if contributed {
                            anchor = Some(anchor.map_or(start, |a| a.min(start)));
                        }
                    }
                }
                bt.menu_anchor = anchor;
            }
            Target::Minibuf { mt, slots } => {
                let mut anchor: Option<usize> = None;
                for (s, slot) in slots.iter().enumerate() {
                    let Some(inv) = &slot.shown else { continue };
                    let start = inv.span.bytes.start;
                    let filter = mt.input[start..mt.cursor].to_owned();
                    let entry = sources.minibuf_get(slot.source);
                    let contributed = score_slot(
                        &mut ctx,
                        s as u32,
                        inv.items(),
                        &filter,
                        entry.match_kind,
                        entry.priority,
                    );
                    if contributed {
                        anchor = Some(anchor.map_or(start, |a| a.min(start)));
                    }
                }
                mt.menu_anchor = anchor;
            }
        }
        // A `Delegated` slot contributes no distinguishing sortText to the
        // tiebreak below, regardless of what its own items' `sort_text`
        // field holds (every `plain()` item now sets it to `label`, purely
        // for a `String`-kind source's own tiebreak) — this is where that
        // exclusion belongs, in the one place that reads the key, rather
        // than relying on a `Delegated` source's item constructor to leave
        // it empty by convention.
        let sort_key_of = |s: u32, i: u32| -> &str {
            match &*target {
                Target::Buffer { slots, .. } => {
                    let slot = &slots[s as usize];
                    if sources.buffer_get(slot.source).match_kind == MatchKind::Delegated {
                        ""
                    } else {
                        &slot.item(i as usize).sort_text
                    }
                }
                Target::Minibuf { slots, .. } => {
                    let slot = &slots[s as usize];
                    if sources.minibuf_get(slot.source).match_kind == MatchKind::Delegated {
                        ""
                    } else {
                        &slot.item(i as usize).sort_text
                    }
                }
            }
        };
        ctx.rank_scratch.sort_unstable_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| sort_key_of(a.2, a.3).cmp(sort_key_of(b.2, b.3)))
                .then((a.2, a.3).cmp(&(b.2, b.3)))
        });
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    /// Number of candidates surviving the current ranking — cheap count for
    /// callers (menu navigation, the visible-menu check) that don't need the
    /// items themselves.
    pub(in crate::editor) fn len(&self) -> usize {
        self.rank_scratch.len()
    }

    /// Whether the current ranking matches nothing. A session can be open
    /// with this `true` — narrowed to empty by continued typing, or awaiting
    /// every source's first answer — in which case no menu is visibly shown.
    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.rank_scratch.is_empty()
    }

    /// The raw `(slot, item)` indices behind ranked position `idx` — every
    /// other reader in this file resolves `idx` through this first.
    fn ranked_indices(&self, idx: usize) -> Option<(u32, u32)> {
        let &(_, _, s, i) = self.rank_scratch.get(idx)?;
        Some((s, i))
    }

    /// The source/invocation/item at ranked `idx` for a `Buffer` session —
    /// `None` for a `Minibuf` session or an unranked `idx`. `accept` is the
    /// only caller that needs the source id (to read `BufferSourceEntry::
    /// resolve`) and the invocation itself (for its span); every other
    /// reader just wants the item ([`Self::selected_item`]).
    fn ranked_buffer(
        &self,
        idx: usize,
    ) -> Option<(BufferSourceId, &Invocation<BufferSpan>, &CompletionItem)> {
        let Target::Buffer { slots, .. } = &self.target else {
            return None;
        };
        let (s, i) = self.ranked_indices(idx)?;
        let slot = &slots[s as usize];
        let inv = slot.shown.as_ref()?;
        Some((slot.source, inv, &inv.items()[i as usize]))
    }

    /// The item behind ranked position `idx`, for a caller (`accept`) that
    /// already has a UI selection index rather than a raw item index.
    pub(in crate::editor) fn selected_item(&self, idx: usize) -> Option<&CompletionItem> {
        let (s, i) = self.ranked_indices(idx)?;
        Some(match &self.target {
            Target::Buffer { slots, .. } => slots[s as usize].item(i as usize),
            Target::Minibuf { slots, .. } => slots[s as usize].item(i as usize),
        })
    }

    /// The `:` line span the ranked candidate at `idx` replaces, with its
    /// `insert_text` — `None` for a `Buffer` session or an unranked `idx`.
    pub(in crate::editor) fn minibuf_apply(&self, idx: usize) -> Option<(Range<usize>, &str)> {
        let Target::Minibuf { slots, .. } = &self.target else {
            return None;
        };
        let (s, i) = self.ranked_indices(idx)?;
        let inv = slots[s as usize].shown.as_ref()?;
        Some((
            inv.span.bytes.clone(),
            inv.items()[i as usize].insert_text(),
        ))
    }

    /// Where the menu anchors for a `Buffer` session: the leftmost live
    /// token start among the sources with a ranked candidate. Stable while
    /// cycling; moves only when ranking changes which sources contribute.
    /// `None` with nothing ranked. Computed once per [`Self::rank`] call,
    /// not per call to this accessor — both this and
    /// [`Self::menu_anchor_byte`] run on the per-frame render path.
    pub(in crate::editor) fn menu_anchor_char(&self) -> Option<CharOffset> {
        match &self.target {
            Target::Buffer { bt, .. } => bt.menu_anchor,
            Target::Minibuf { .. } => None,
        }
    }

    /// [`Self::menu_anchor_char`]'s `Minibuf` counterpart, a byte offset
    /// into the `:` line's input.
    pub(in crate::editor) fn menu_anchor_byte(&self) -> Option<usize> {
        match &self.target {
            Target::Minibuf { mt, .. } => mt.menu_anchor,
            Target::Buffer { .. } => None,
        }
    }

    /// Moves a menu selection by one row, wrapping at either end — `None`
    /// when the ranked list is empty, so a caller can't divide by, or
    /// subtract from, zero computing the wrapped index itself. `current` is
    /// the caller's own UI state (`CompletionMenuUi` lives outside this type
    /// — see its own doc), not tracked here.
    pub(in crate::editor) fn step_selection(&self, current: usize, forward: bool) -> Option<usize> {
        let n = self.rank_scratch.len();
        if n == 0 {
            return None;
        }
        Some(if forward {
            (current + 1) % n
        } else {
            current.checked_sub(1).unwrap_or(n - 1)
        })
    }

    pub(in crate::editor) fn top(
        &self,
        n: usize,
        sources: &SourceRegistry,
    ) -> Vec<serde_json::Value> {
        (0..n.min(self.rank_scratch.len()))
            .filter_map(|idx| {
                let (s, i) = self.ranked_indices(idx)?;
                Some(match &self.target {
                    Target::Buffer { slots, .. } => {
                        let slot = &slots[s as usize];
                        slot.item(i as usize)
                            .to_json(&sources.buffer_get(slot.source).name)
                    }
                    Target::Minibuf { slots, .. } => {
                        let slot = &slots[s as usize];
                        slot.item(i as usize)
                            .to_json(&sources.minibuf_get(slot.source).name)
                    }
                })
            })
            .collect()
    }

    /// Row content for the candidates at `range` in ranked order — the only
    /// way rows leave this session, and `range` is `menu_window`'s own
    /// window (`hume_ui::popup`), so a frame formats at most `MAX_MENU_ROWS`
    /// rows and never the whole ranked list. There is deliberately no
    /// full-list accessor: the render side measures width over whatever
    /// it's handed, so the one thing that keeps a scrolled-away candidate
    /// from inflating the box is that nothing can hand it over.
    pub(in crate::editor) fn rows_in(&self, range: Range<usize>) -> Vec<hume_ui::popup::MenuRow> {
        range
            .filter_map(|idx| self.selected_item(idx))
            .map(CompletionItem::menu_row)
            .collect()
    }
}

/// Whether `pos` sits within the closed interval `[range.start, range.end]`
/// — the completion model's own span-containment convention (a token or
/// replacement span always includes both its own endpoints, since the
/// cursor is allowed to sit exactly at either — unlike
/// `hume_rope::offset::ExclusiveRange`'s half-open one, which doesn't apply
/// here). Shared by every span-containment check in this module and by
/// `accept.rs`.
fn contains_cursor<T: PartialOrd>(range: &Range<T>, pos: T) -> bool {
    range.start <= pos && pos <= range.end
}

/// `text[start..head]` — a `Buffer` invocation's token text, in whatever
/// document `text` is (the invocation's own snapshot for
/// [`Invocation::prefix`]'s Steel-facing seed, the live buffer for
/// [`CompletionSession::rank`]'s scoring pass). Both callers already know
/// `start <= head` before calling this.
fn token_text(text: &BufferText, start: CharOffset, head: CharOffset) -> String {
    text.slice(ExclusiveRange::new(start, head)).to_string()
}

/// Boundary-safe prefix check shared by every `MatchKind::String` source —
/// `str::get` returns `None` (never a panic) when `prefix.len()` lands off a
/// char boundary or past `haystack`'s end, matching `complete_command`'s own
/// original safety for non-ASCII names.
fn prefix_matches(haystack: &str, prefix: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        haystack.starts_with(prefix)
    } else {
        haystack
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    }
}

#[cfg(test)]
mod tests;
