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

use hume_editing::changeset::{Assoc, ChangeSet};
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
pub(in crate::editor) struct MinibufTarget {
    input: String,
    cursor: usize,
}

/// The coordinate system one invocation's answer was computed in: the rope
/// at invoke time (an O(1) clone — ropey is structurally shared), the
/// cursor then, and every edit observed since, composed. A server's wire
/// `textEdit` range is computed against the document as it stood at the
/// *request*, which is this snapshot; `accept` decodes against `rope` and
/// maps forward through `cs_since` rather than approximating drift as a
/// scalar shift.
pub(in crate::editor) struct DocSnapshot {
    rope: ropey::Rope,
    head: CharOffset,
    cs_since: ChangeSet,
}

/// Where one invocation's token sits, tracked through edits.
enum SpanTrack {
    Buffer {
        doc: DocSnapshot,
        /// The token in live coordinates — `start` mapped `Assoc::Before`
        /// through every observed edit (text inserted exactly at the token's
        /// start belongs to the token), `end` mapped `Assoc::After` (text
        /// typed at the token's end extends it). `None` while a
        /// `BufferToken::Custom` source is still pending: its span arrives
        /// with its answer.
        live: Option<Range<CharOffset>>,
    },
    /// Byte range in [`MinibufTarget::input`]. Never moves — nothing can
    /// edit the `:` line while a session is open without dismissing it.
    /// `None` while a `MinibufToken::Custom` source is pending.
    Minibuf { bytes: Option<Range<usize>> },
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
    /// A `Buffer`-target invocation. `live` is the token per the source's
    /// `BufferToken` rule, or `None` for `Custom` (the answer names it).
    pub(in crate::editor) fn buffer(
        rope: ropey::Rope,
        head: CharOffset,
        live: Option<Range<CharOffset>>,
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

    /// A `Minibuf`-target invocation. `bytes` is the token per the source's
    /// `MinibufToken` rule, or `None` for `Custom`.
    pub(in crate::editor) fn minibuf(bytes: Option<Range<usize>>) -> Self {
        Self {
            id: widget_token::next(),
            span: SpanTrack::Minibuf { bytes },
            state: InvocationState::Pending,
        }
    }

    /// The seeded filter text for a `Buffer` invocation — `text[live.start
    /// .. head]` as of the invoke — handed to a Steel source as its `prefix`
    /// argument. `""` for a pending `Custom` span.
    pub(in crate::editor) fn prefix(&self, text: &BufferText) -> String {
        match &self.span {
            SpanTrack::Buffer {
                doc,
                live: Some(live),
            } => text
                .slice(ExclusiveRange::new(live.start, doc.head))
                .to_string(),
            _ => String::new(),
        }
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
    /// Ranked `(slot index, item index)` pairs, rebuilt by every
    /// [`Self::rank`] call.
    filtered: Vec<(u32, u32)>,
    /// Retained across `rank` calls so per-keystroke filtering doesn't
    /// allocate a fresh Vec every time. `(score, slot, item)`.
    rank_scratch: Vec<(u32, u32, u32)>,
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
            filtered: Vec::new(),
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
        }))
    }

    /// A `Minibuf`-target session over the `:` line's `input`, cursor at
    /// byte `cursor`.
    pub(in crate::editor) fn open_minibuf(input: String, cursor: usize) -> Self {
        Self::new(Target::Minibuf(MinibufTarget { input, cursor }))
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

    /// Lands an answer for invocation `id`. `Ok(false)` when `id` isn't the
    /// latest call of any slot here — a superseded or already-replaced
    /// invocation, expected-normal for a late async source, never an error.
    /// A repeated answer for a still-latest `shown` id replaces it in place
    /// (a source may stream). `span` is required for, and only honoured by,
    /// an invocation whose token rule left it to the source (`live`/`bytes`
    /// still `None`) — in the *invocation's own* coordinates (the snapshot
    /// a `Buffer` source was handed, the `input` a `Minibuf` one was), never
    /// live ones: a source computes it from what it was given, and the
    /// session maps it forward through whatever was typed since.
    pub(in crate::editor) fn contribute(
        &mut self,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
        span: Option<(usize, usize)>,
    ) -> Result<bool, String> {
        let Some(slot) = self.slots.iter_mut().find(|s| s.latest_id() == Some(id)) else {
            return Ok(false);
        };
        let mut invocation = match slot.inflight.take() {
            Some(inv) => inv,
            None => slot
                .shown
                .take()
                .expect("latest_id came from one of the two"),
        };
        // A rejected span leaves the slot with neither `inflight` nor
        // `shown` — the source answered, and its answer was unusable, which
        // is "nothing from this source", not "still waiting"; the error
        // itself reaches the Steel caller.
        match &mut invocation.span {
            SpanTrack::Buffer {
                doc,
                live: live @ None,
            } => *live = Some(doc.resolve_custom_span(span)?),
            SpanTrack::Minibuf {
                bytes: bytes @ None,
            } => {
                let Target::Minibuf(m) = &self.target else {
                    unreachable!("a Minibuf invocation only ever lives in a Minibuf session")
                };
                *bytes = Some(m.resolve_custom_span(span)?);
            }
            _ => {}
        }
        invocation.state = InvocationState::Shown { items, incomplete };
        slot.shown = Some(invocation);
        Ok(true)
    }

    /// Whether any slot still awaits an answer.
    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.slots.iter().any(|s| s.inflight.is_some())
    }

    /// Whether any source has answered with at least one item — `false`
    /// once every answer is empty or the cursor left every token, at which
    /// point the session has nothing left to show and closes.
    pub(in crate::editor) fn has_live_sources(&self) -> bool {
        self.slots.iter().any(SourceSlot::is_live)
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
    /// for a `Minibuf` one — with its source's `MatchKind`. Rank key:
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
        for (s, slot) in self.slots.iter().enumerate() {
            let Some(inv) = &slot.shown else { continue };
            let entry = sources.get(slot.source);
            // A shown slot always contains `head` after `observe_edit`
            // (see its doc), so an inverted range here can only mean an
            // edit this session never saw — skipped rather than sliced,
            // until the settle-time validity check dismisses the session.
            let filter = match (&inv.span, &self.target, &live) {
                (
                    SpanTrack::Buffer {
                        live: Some(range), ..
                    },
                    Target::Buffer(_),
                    Some(doc),
                ) if range.start <= doc.head => doc
                    .text
                    .slice(ExclusiveRange::new(range.start, doc.head))
                    .to_string(),
                (SpanTrack::Minibuf { bytes: Some(range) }, Target::Minibuf(m), _) => {
                    m.input[range.start..m.cursor].to_owned()
                }
                _ => continue,
            };
            let pattern = self.matcher.parse(&filter);
            for (i, item) in inv.items().iter().enumerate() {
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
                    self.rank_scratch.push((score, s as u32, i as u32));
                }
            }
        }
        let slots = &self.slots;
        let item_of =
            |s: u32, i: u32| &slots[s as usize].shown.as_ref().expect("ranked").items()[i as usize];
        let priority_of = |s: u32| sources.get(slots[s as usize].source).priority;
        self.rank_scratch.sort_unstable_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| priority_of(b.1).cmp(&priority_of(a.1)))
                .then_with(|| {
                    item_of(a.1, a.2)
                        .sort_text
                        .cmp(&item_of(b.1, b.2).sort_text)
                })
                .then((a.1, a.2).cmp(&(b.1, b.2)))
        });
        self.filtered.clear();
        self.filtered
            .extend(self.rank_scratch.iter().map(|&(_, s, i)| (s, i)));
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    /// Number of candidates surviving the current ranking — cheap count for
    /// callers (menu navigation, the visible-menu check) that don't need the
    /// items themselves.
    pub(in crate::editor) fn len(&self) -> usize {
        self.filtered.len()
    }

    /// Whether the current ranking matches nothing. A session can be open
    /// with this `true` — narrowed to empty by continued typing, or awaiting
    /// every source's first answer — in which case no menu is visibly shown.
    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    fn ranked(&self, idx: usize) -> Option<(&SourceSlot, &Invocation, &CompletionItem)> {
        let &(s, i) = self.filtered.get(idx)?;
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
        match &inv.span {
            SpanTrack::Minibuf { bytes: Some(bytes) } => Some((bytes.clone(), item.insert_text())),
            _ => None,
        }
    }

    /// Where the menu anchors for a `Buffer` session: the leftmost live
    /// token start among the sources with a ranked candidate. Stable while
    /// cycling; moves only when ranking changes which sources contribute.
    /// `None` with nothing ranked.
    pub(in crate::editor) fn menu_anchor_char(&self) -> Option<CharOffset> {
        self.ranked_slots()
            .filter_map(|slot| match &slot.shown.as_ref()?.span {
                SpanTrack::Buffer {
                    live: Some(range), ..
                } => Some(range.start),
                _ => None,
            })
            .min()
    }

    /// [`Self::menu_anchor_char`]'s `Minibuf` counterpart, a byte offset
    /// into the `:` line's input.
    pub(in crate::editor) fn menu_anchor_byte(&self) -> Option<usize> {
        self.ranked_slots()
            .filter_map(|slot| match &slot.shown.as_ref()?.span {
                SpanTrack::Minibuf { bytes: Some(range) } => Some(range.start),
                _ => None,
            })
            .min()
    }

    /// Every slot with at least one ranked candidate, each once.
    fn ranked_slots(&self) -> impl Iterator<Item = &SourceSlot> {
        let mut seen = vec![false; self.slots.len()];
        self.filtered.iter().filter_map(move |&(s, _)| {
            let s = s as usize;
            (!std::mem::replace(&mut seen[s], true)).then(|| &self.slots[s])
        })
    }

    /// Moves a menu selection by one row, wrapping at either end — `None`
    /// when `filtered` is empty, so a caller can't divide by, or subtract
    /// from, zero computing the wrapped index itself. `current` is the
    /// caller's own UI state (`CompletionMenuUi` lives outside this type —
    /// see its own doc), not tracked here.
    pub(in crate::editor) fn step_selection(&self, current: usize, forward: bool) -> Option<usize> {
        let n = self.filtered.len();
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
        (0..n.min(self.filtered.len()))
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
        let SpanTrack::Buffer { doc, live } = &mut self.span else {
            return true;
        };
        let crossed = live.as_ref().is_some_and(|range| {
            range.start > CharOffset::new(0) && {
                let mut pair = [range.start.retreat(1), range.start];
                cs.map_positions(&mut pair, Assoc::Before);
                pair[0] == pair[1]
            }
        });
        if let Some(range) = live {
            let mut start = [range.start];
            cs.map_positions(&mut start, Assoc::Before);
            let mut end = [range.end];
            cs.map_positions(&mut end, Assoc::After);
            *range = start[0]..end[0];
        }
        doc.cs_since = doc.cs_since.clone().compose(cs.clone());
        !crossed
    }

    /// Whether `head` is still inside this invocation's live token (a
    /// pending `Custom` span contains everything — nothing to leave yet).
    fn contains(&self, head: CharOffset) -> bool {
        match &self.span {
            SpanTrack::Buffer {
                live: Some(range), ..
            } => range.start <= head && head <= range.end,
            _ => true,
        }
    }
}

const SPAN_REQUIRED: &str =
    "completion-emit!: this source's token rule is 'custom, so #:span is required";

impl DocSnapshot {
    /// A `Custom`-token answer's own span, validated against this snapshot
    /// (in range, grapheme-snapped, containing the cursor as it stood then,
    /// on one line — a completion token never spans a line, the one bound
    /// left on an otherwise source-chosen value since it doubles as
    /// accept's own replacement span) and mapped forward through every edit
    /// observed since, to live coordinates.
    fn resolve_custom_span(
        &self,
        span: Option<(usize, usize)>,
    ) -> Result<Range<CharOffset>, String> {
        let Some((start, end)) = span else {
            return Err(SPAN_REQUIRED.to_string());
        };
        let rope = &self.rope;
        let mint = |idx: usize| {
            CharOffset::checked(rope, idx)
                .map(|c| hume_rope::grapheme::snap_to_cluster_start(rope.slice(..), c))
                .ok_or_else(|| {
                    format!(
                        "completion-emit!: #:span offset {idx} is out of range \
                         (buffer had {} chars)",
                        rope.len_chars()
                    )
                })
        };
        let (start, end) = (mint(start)?, mint(end)?);
        if !(start <= self.head && self.head <= end) {
            return Err("completion-emit!: #:span does not contain the cursor".to_string());
        }
        if hume_rope::lines::char_to_ropey_line(rope, start)
            != hume_rope::lines::char_to_ropey_line(rope, end)
        {
            return Err("completion-emit!: #:span spans more than one line".to_string());
        }
        let mut pair = [start, end];
        self.cs_since.map_positions(&mut pair[..1], Assoc::Before);
        self.cs_since.map_positions(&mut pair[1..], Assoc::After);
        Ok(pair[0]..pair[1])
    }
}

impl MinibufTarget {
    /// [`DocSnapshot::resolve_custom_span`]'s `:`-line counterpart: byte
    /// offsets into `input`, on char boundaries, containing the cursor.
    fn resolve_custom_span(&self, span: Option<(usize, usize)>) -> Result<Range<usize>, String> {
        let Some((start, end)) = span else {
            return Err(SPAN_REQUIRED.to_string());
        };
        if !(start <= self.cursor && self.cursor <= end && end <= self.input.len()) {
            return Err(format!(
                "completion-emit!: #:span ({start}, {end}) must contain the cursor ({}) \
                 within the input ({} bytes)",
                self.cursor,
                self.input.len()
            ));
        }
        if !self.input.is_char_boundary(start) || !self.input.is_char_boundary(end) {
            return Err("completion-emit!: #:span is not on a character boundary".to_string());
        }
        Ok(start..end)
    }
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
