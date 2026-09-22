//! The one open completion session: a Rust store holding what every
//! participating source has answered, ranked per keystroke against each
//! source's *own* token. One singleton session per editor (not per buffer);
//! `orchestrate.rs` opens it, invokes sources into it, feeds their answers
//! back, and tells it about edits — this file knows nothing about *how* a
//! source runs, only what it said and where.
//!
//! One session type serves both completion targets ([`Target`]) — the
//! accept mechanism differs (a buffer edit vs. a splice into the `:`
//! line's input), but slots, invocations, ranking, and the menu do not.
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
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::editor::fuzzy::{FuzzyMatcher, FuzzyProfile};
use crate::editor::widget_token;

use super::item::CompletionItem;
use super::registry::{SourceId, SourceRegistry};

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
///   the matching rule differs from `Fuzzy`.
/// - `Delegated` — the source computed its own finished, already-ordered
///   result fresh from the live input (a directory read, a multi-phase
///   parse); this session does no scoring of its own for these items. Given
///   a tied score and an empty `sort_text` at construction (see the item
///   constructor `Delegated` sources use), the rank key's final tiebreak —
///   index ascending — preserves the source's own order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// Where an accepted item lands — the one axis `accept` branches on, and
/// the one axis further-typing behavior follows (see `CompletionLayer::
/// handler`'s doc): a `Buffer` session refilters the open menu in place as
/// the user types; a `Minibuf` session dismisses on any key but Tab, which
/// only cycles the list already computed.
pub(in crate::editor) enum Target {
    Buffer(BufferTarget),
    Minibuf(MinibufTarget),
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
    /// `filtered` on every render frame. `None` with nothing ranked.
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
    head: CharOffset,
    cs_since: ChangeSet,
}

/// A ranked slot's own token start, in whichever unit its target uses —
/// `rank`'s own local, folded into `menu_anchor` as each slot is visited.
enum TokenStart {
    Buffer(CharOffset),
    Minibuf(usize),
}

/// Where one invocation's token sits, tracked through edits. Both variants'
/// spans are fixed at invoke time — the source's declared token rule
/// (`'word` for a buffer source, `'arg` for a minibuffer one) — and never
/// renamed by the answer itself.
enum SpanTrack {
    Buffer {
        doc: DocSnapshot,
        /// The token in live coordinates — `start` mapped `Assoc::Before`
        /// through every observed edit (text inserted exactly at the token's
        /// start belongs to the token), `end` mapped `Assoc::After` (text
        /// typed at the token's end extends it).
        live: Range<CharOffset>,
    },
    /// Byte range in [`MinibufTarget::input`]. Never moves — nothing can
    /// edit the `:` line while a session is open without dismissing it.
    Minibuf { bytes: Range<usize> },
}

enum InvocationState {
    Pending,
    Shown {
        items: Vec<CompletionItem>,
        incomplete: bool,
    },
}

/// One call of one source for one trigger: the id the source answers to,
/// the span it was asked about (or will name), and its answer once it has
/// one. Minted by `orchestrate.rs` against the live document, stored here.
pub(in crate::editor) struct Invocation {
    id: u64,
    span: SpanTrack,
    state: InvocationState,
}

impl Invocation {
    /// A `Buffer`-target invocation. `live` is the word before the cursor —
    /// every buffer source's token rule.
    pub(in crate::editor) fn buffer(
        rope: ropey::Rope,
        head: CharOffset,
        live: Range<CharOffset>,
    ) -> Self {
        let cs_since = ChangeSet::identity(rope.len_chars());
        Self {
            id: widget_token::next(),
            span: SpanTrack::Buffer {
                doc: DocSnapshot {
                    rope,
                    head,
                    cs_since,
                },
                live,
            },
            state: InvocationState::Pending,
        }
    }

    /// A `Minibuf`-target invocation. `bytes` is the whitespace-delimited
    /// argument the cursor is in — every minibuffer source's token rule,
    /// except a `NativeDelegated` one, which computes its own span
    /// synchronously before this is minted (`orchestrate.rs`'s
    /// `invoke_minibuf_source`).
    pub(in crate::editor) fn minibuf(bytes: Range<usize>) -> Self {
        Self {
            id: widget_token::next(),
            span: SpanTrack::Minibuf { bytes },
            state: InvocationState::Pending,
        }
    }

    /// The seeded filter text for a `Buffer` invocation — `text[live.start
    /// .. head]` as of the invoke — handed to a Steel source as its `prefix`
    /// argument.
    pub(in crate::editor) fn prefix(&self, text: &BufferText) -> String {
        let SpanTrack::Buffer { doc, live } = &self.span else {
            return String::new();
        };
        token_text(text, live.start, doc.head)
    }

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

/// One participating source: the answer currently ranked (`shown`) and, if
/// the source was re-invoked since, the newer call whose answer hasn't
/// landed yet (`inflight`). An emission applies only to the *latest* of the
/// two ids — a late answer to a superseded call is dropped, so a slow LSP
/// response can never overwrite a newer one.
struct SourceSlot {
    source: SourceId,
    shown: Option<Invocation>,
    inflight: Option<Invocation>,
}

impl SourceSlot {
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
}

/// The document a `Buffer` session's tokens are read against when ranking
/// — the live text and the primary cursor's head.
pub(in crate::editor) struct LiveDoc<'a> {
    pub(in crate::editor) text: &'a BufferText,
    pub(in crate::editor) head: CharOffset,
}

pub(in crate::editor) struct CompletionSession {
    target: Target,
    slots: Vec<SourceSlot>,
    /// `(score, priority, slot, item)` for every surviving candidate,
    /// rebuilt and sorted by every [`Self::rank`] call — this *is* the
    /// session's own ranked list (`len`/`ranked`/`rows_in` all read it
    /// directly; there is no separate `filtered` copy to keep in sync).
    /// Retained across calls so per-keystroke filtering doesn't allocate a
    /// fresh Vec every time. `priority` is `SourceEntry::priority`'s own
    /// `i64`, not narrowed — this tuple is sorted, never used as a lookup
    /// key, so there's no reason to risk a truncating cast.
    rank_scratch: Vec<(u32, i64, u32, u32)>,
    /// Reusable scoring engine — `FuzzyProfile::Autocomplete` (see its doc)
    /// distinguishes this from the picker's own instance. One instance per
    /// session, consulted only for a `MatchKind::Fuzzy` slot.
    matcher: FuzzyMatcher,
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
            slots: Vec::new(),
            rank_scratch: Vec::new(),
            matcher: FuzzyMatcher::new(FuzzyProfile::Autocomplete),
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
        Self::new(Target::Buffer(BufferTarget {
            bid,
            pane_id,
            generation,
            len,
            explicit: false,
            menu_anchor: None,
        }))
    }

    /// Whether an explicit `Trigger::Explicit` (Ctrl-Space) has touched
    /// this session — see [`BufferTarget::explicit`]'s doc. `false` for a
    /// `Minibuf` session; this axis only applies to `Buffer` ones.
    pub(in crate::editor) fn is_explicit(&self) -> bool {
        self.buffer().is_some_and(|bt| bt.explicit)
    }

    /// Records that an explicit trigger touched this session — called by
    /// `trigger_buffer_completion` on both the fresh-open and the
    /// reused-session path, so a session a trigger char opened still
    /// reports once the user follows up with Ctrl-Space. A no-op for a
    /// `Minibuf` session (unreachable: only a `Buffer` trigger calls this).
    pub(in crate::editor) fn mark_explicit_trigger(&mut self) {
        if let Target::Buffer(bt) = &mut self.target {
            bt.explicit = true;
        }
    }

    /// A `Minibuf`-target session over the `:` line's `input`, cursor at
    /// byte `cursor`.
    pub(in crate::editor) fn open_minibuf(input: String, cursor: usize) -> Self {
        Self::new(Target::Minibuf(MinibufTarget {
            input,
            cursor,
            menu_anchor: None,
        }))
    }

    /// The `Buffer`-target fields, or `None` for a `Minibuf` session — the
    /// one way code outside this module reaches `BufferTarget`'s own
    /// accessors, so a `Minibuf` session can't be asked for a buffer-only
    /// fact by mistake: the compiler forces every caller to handle `None`
    /// rather than trusting an `.expect()` that a `Minibuf` session never
    /// reaches it.
    pub(in crate::editor) fn buffer(&self) -> Option<&BufferTarget> {
        match &self.target {
            Target::Buffer(b) => Some(b),
            Target::Minibuf(_) => None,
        }
    }

    /// The `:` line as this `Minibuf` session's sources saw it — `None` for
    /// a `Buffer` session. The one way code outside this module learns
    /// which target a session has, since `Target` itself stays private.
    pub(in crate::editor) fn minibuf_input(&self) -> Option<&str> {
        match &self.target {
            Target::Minibuf(m) => Some(&m.input),
            Target::Buffer(_) => None,
        }
    }

    // ── Sources in and out ───────────────────────────────────────────────────

    /// Records a fresh call of `source`, superseding any still in flight
    /// for it (whose id is now stale). Returns the id the answer must carry.
    pub(in crate::editor) fn invoke(&mut self, source: SourceId, invocation: Invocation) -> u64 {
        let id = invocation.id;
        let slot = match self.slots.iter().position(|s| s.source == source) {
            Some(i) => &mut self.slots[i],
            None => {
                self.slots.push(SourceSlot {
                    source,
                    shown: None,
                    inflight: None,
                });
                self.slots.last_mut().expect("just pushed")
            }
        };
        slot.inflight = Some(invocation);
        id
    }

    /// Lands an answer for invocation `id`. `false` when `id` isn't the
    /// latest call of any slot here — a superseded or already-replaced
    /// invocation, expected-normal for a late async source, never an error.
    /// A repeated answer for a still-latest `shown` id replaces it in place
    /// (a source may stream); the invocation's span is untouched — it was
    /// computed once, from the source's token rule, at invoke time.
    pub(in crate::editor) fn contribute(
        &mut self,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        let Some(slot) = self.slots.iter_mut().find(|s| s.latest_id() == Some(id)) else {
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

    /// Whether any slot still awaits an answer.
    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.slots.iter().any(|s| s.inflight.is_some())
    }

    /// Whether any source has answered with at least one item — `false`
    /// once every answer is empty or the cursor left every token, at which
    /// point the session has nothing left to show and closes.
    fn has_live_sources(&self) -> bool {
        self.slots.iter().any(SourceSlot::is_live)
    }

    /// Whether the session has nothing left to show and nothing on its way
    /// — every source answered empty, or the cursor typed out of every
    /// token. The one rule `orchestrate.rs` checks, at every point a
    /// session might have just run out: right after opening/re-invoking,
    /// and after each answer lands.
    pub(in crate::editor) fn is_spent(&self) -> bool {
        !self.is_pending() && !self.has_live_sources()
    }

    /// The sources to call again after an edit: those that flagged their
    /// latest answer `isIncomplete`, and those still pending (their in-
    /// flight call saw an older document, so it is superseded rather than
    /// waited for).
    pub(in crate::editor) fn sources_to_reinvoke(&self) -> Vec<SourceId> {
        self.slots
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
    /// keystroke at a cursor *before* the primary shifts every token).
    /// Composes `cs` into each invocation's `cs_since`, remaps each live
    /// span, and drops the answer of any slot whose token the cursor has
    /// left: `head` outside `[start, end]`, or the character *before* the
    /// token deleted (a Backspace at the token's start — detected by `start`
    /// and `start - 1` mapping to the same live position, which no edit
    /// elsewhere can cause).
    ///
    /// Returns `false` — session untouched — when `cs` wasn't produced
    /// against this session's own tracked document length: an edit reached
    /// the buffer through a path this session never observed, which
    /// `ChangeSet::compose` would otherwise turn into a hard panic (its
    /// `len_before`/`len_after` check is a release `assert_eq!`). The caller
    /// must dismiss the session in that case. `text_gen` is the buffer's
    /// generation *after* `cs` landed.
    pub(in crate::editor) fn observe_edit(
        &mut self,
        cs: &ChangeSet,
        text_gen: u64,
        head: CharOffset,
    ) -> bool {
        let Target::Buffer(bt) = &mut self.target else {
            return true;
        };
        if cs.len_before() != bt.len {
            return false;
        }
        bt.len = cs.len_after();
        bt.generation = text_gen;
        for slot in &mut self.slots {
            if let Some(inv) = &mut slot.inflight {
                inv.observe(cs);
            }
            if let Some(inv) = &mut slot.shown
                && !inv.observe(cs)
            {
                slot.shown = None;
            }
            if let Some(inv) = &slot.shown
                && !inv.contains(head)
            {
                slot.shown = None;
            }
        }
        true
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    /// Re-scores every shown item against its own slot's token text —
    /// `live[start..head]` for a `Buffer` session, `input[start..cursor]`
    /// for a `Minibuf` one — with its source's `MatchKind`, dropping any
    /// item that's a no-op against that text first (`CompletionItem::
    /// is_noop_for`) regardless of `MatchKind`. Rank key:
    /// score descending, then source priority descending (a tiebreaker
    /// only — match quality stays king — applied before sortText so a
    /// higher-priority source's item wins a tie regardless of how its label
    /// sorts; direction matches `register_sign_source`'s own `(priority
    /// desc, name asc)`), then sortText ascending (the server's own ordering
    /// hint, the *only* signal left on an empty filter — nucleo scores every
    /// haystack `0` for an empty pattern), then slot and item index (sortText
    /// is very often duplicated across a server's items).
    pub(in crate::editor) fn rank(&mut self, sources: &SourceRegistry, live: Option<LiveDoc<'_>>) {
        self.rank_scratch.clear();
        // The leftmost token start among the slots that end up contributing
        // at least one scored item — folded into this same per-slot pass
        // (each slot visited once) rather than a second walk over
        // `rank_scratch` every time a caller needs it
        // (`menu_anchor_char`/`menu_anchor_byte`, both on the per-frame
        // render path).
        let mut buffer_anchor: Option<CharOffset> = None;
        let mut minibuf_anchor: Option<usize> = None;
        for (s, slot) in self.slots.iter().enumerate() {
            let Some(inv) = &slot.shown else { continue };
            let entry = sources.get(slot.source);
            // A shown slot always contains `head` after `observe_edit`
            // (see its doc), so an inverted range here can only mean an
            // edit this session never saw — skipped rather than sliced,
            // until the settle-time validity check dismisses the session.
            let (filter, token_start) = match (&inv.span, &self.target, &live) {
                (SpanTrack::Buffer { live: range, .. }, Target::Buffer(_), Some(doc)) => {
                    if range.start > doc.head {
                        continue;
                    }
                    let text = token_text(doc.text, range.start, doc.head);
                    (text, TokenStart::Buffer(range.start))
                }
                (SpanTrack::Minibuf { bytes: range }, Target::Minibuf(m), _) => (
                    m.input[range.start..m.cursor].to_owned(),
                    TokenStart::Minibuf(range.start),
                ),
                _ => continue,
            };
            let pattern = self.matcher.parse(&filter);
            let mut contributed = false;
            for (i, item) in inv.items().iter().enumerate() {
                if item.is_noop_for(&filter) {
                    continue;
                }
                let score = match entry.match_kind {
                    MatchKind::Fuzzy => self.matcher.score(&pattern, &item.filter_text),
                    MatchKind::String { case_sensitive } => {
                        prefix_matches(&item.filter_text, &filter, case_sensitive).then_some(0)
                    }
                    // The source already produced a finished, ordered result
                    // — never excluded here; the rank key's tiebreak chain
                    // (empty `sort_text`, see the item constructor these
                    // sources use) preserves that order via the final
                    // index-ascending key.
                    MatchKind::Delegated => Some(0),
                };
                if let Some(score) = score {
                    self.rank_scratch
                        .push((score, entry.priority, s as u32, i as u32));
                    contributed = true;
                }
            }
            if contributed {
                match token_start {
                    TokenStart::Buffer(start) => {
                        buffer_anchor = Some(buffer_anchor.map_or(start, |a| a.min(start)));
                    }
                    TokenStart::Minibuf(start) => {
                        minibuf_anchor = Some(minibuf_anchor.map_or(start, |a| a.min(start)));
                    }
                }
            }
        }
        let slots = &self.slots;
        let item_of =
            |s: u32, i: u32| &slots[s as usize].shown.as_ref().expect("ranked").items()[i as usize];
        self.rank_scratch.sort_unstable_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| {
                    item_of(a.2, a.3)
                        .sort_text
                        .cmp(&item_of(b.2, b.3).sort_text)
                })
                .then((a.2, a.3).cmp(&(b.2, b.3)))
        });
        match &mut self.target {
            Target::Buffer(bt) => bt.menu_anchor = buffer_anchor,
            Target::Minibuf(m) => m.menu_anchor = minibuf_anchor,
        }
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

    fn ranked(&self, idx: usize) -> Option<(&SourceSlot, &Invocation, &CompletionItem)> {
        let &(_, _, s, i) = self.rank_scratch.get(idx)?;
        let slot = &self.slots[s as usize];
        let inv = slot.shown.as_ref()?;
        Some((slot, inv, &inv.items()[i as usize]))
    }

    /// The item behind `filtered[idx]`, for a caller (`accept`) that already
    /// has a UI selection index rather than a raw item index.
    pub(in crate::editor) fn selected_item(&self, idx: usize) -> Option<&CompletionItem> {
        self.ranked(idx).map(|(_, _, item)| item)
    }

    /// The `:` line span the ranked candidate at `idx` replaces, with its
    /// `insert_text` — `None` for a `Buffer` session or an unranked `idx`.
    pub(in crate::editor) fn minibuf_apply(&self, idx: usize) -> Option<(Range<usize>, &str)> {
        let (_, inv, item) = self.ranked(idx)?;
        let SpanTrack::Minibuf { bytes } = &inv.span else {
            return None;
        };
        Some((bytes.clone(), item.insert_text()))
    }

    /// Where the menu anchors for a `Buffer` session: the leftmost live
    /// token start among the sources with a ranked candidate. Stable while
    /// cycling; moves only when ranking changes which sources contribute.
    /// `None` with nothing ranked. Computed once per [`Self::rank`] call,
    /// not per call to this accessor — both this and
    /// [`Self::menu_anchor_byte`] run on the per-frame render path.
    pub(in crate::editor) fn menu_anchor_char(&self) -> Option<CharOffset> {
        match &self.target {
            Target::Buffer(bt) => bt.menu_anchor,
            Target::Minibuf(_) => None,
        }
    }

    /// [`Self::menu_anchor_char`]'s `Minibuf` counterpart, a byte offset
    /// into the `:` line's input.
    pub(in crate::editor) fn menu_anchor_byte(&self) -> Option<usize> {
        match &self.target {
            Target::Minibuf(m) => m.menu_anchor,
            Target::Buffer(_) => None,
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
            .filter_map(|idx| self.ranked(idx))
            .map(|(slot, _, item)| item.to_json(&sources.get(slot.source).name))
            .collect()
    }

    /// Row content for the candidates at `range` in ranked order — the only
    /// way rows leave this session, and `range` is `menu_window`'s own
    /// window (`hume_ui::popup`), so a frame formats at most `MAX_MENU_ROWS`
    /// rows (each an `Arc<str>` refcount bump) and never the whole filtered
    /// list. There is deliberately no full-list accessor: the render side
    /// measures width over whatever it's handed, so the one thing that keeps
    /// a scrolled-away candidate from inflating the box is that nothing can
    /// hand it over.
    pub(in crate::editor) fn rows_in(&self, range: Range<usize>) -> Vec<hume_ui::popup::MenuRow> {
        range
            .filter_map(|idx| self.ranked(idx))
            .map(|(_, _, item)| item.menu_row())
            .collect()
    }
}

impl Invocation {
    /// Composes `cs` into a `Buffer` invocation's snapshot and remaps its
    /// live span. Returns `false` when the edit crossed the token's start —
    /// see [`CompletionSession::observe_edit`].
    fn observe(&mut self, cs: &ChangeSet) -> bool {
        let SpanTrack::Buffer { doc, live: range } = &mut self.span else {
            return true;
        };
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
        let end = cursor.map(range.end, Assoc::After);
        *range = start..end;
        doc.cs_since = doc.cs_since.clone().compose(cs.clone());
        !crossed
    }

    /// Whether `head` is still inside this invocation's live token.
    fn contains(&self, head: CharOffset) -> bool {
        let SpanTrack::Buffer { live, .. } = &self.span else {
            return true;
        };
        contains_cursor(live, head)
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
