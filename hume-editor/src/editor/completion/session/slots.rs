//! The bookkeeping shared by both completion targets: one participating
//! source's slot ([`SourceSlot`]), the current ranked list, the fuzzy
//! matcher, and the menu's selected row, bundled as [`SlotSet`], generic
//! over a target's own id type (`BufferSourceId`/`MinibufSourceId`) and
//! span shape (`BufferSpan`/`MinibufSpan`). [`super::BufferSession`]/
//! [`super::MinibufSession`] each hold one, plus whatever is genuinely
//! their own (cross-source dedup is `Buffer`-only, since `Minibuf` invokes
//! exactly one source).

use super::super::item::CompletionItem;
use super::super::registry::{BufferSourceId, MinibufSourceId, SourceRegistry};
use super::MatchKind;
use crate::editor::fuzzy::{FuzzyMatcher, FuzzyPattern, FuzzyProfile};
use crate::editor::widget_token;

enum InvocationState {
    Pending,
    Shown {
        items: Vec<CompletionItem>,
        incomplete: bool,
    },
}

/// One call of one source for one trigger: the id the source answers to,
/// the span it was asked about, and its answer once it has one. Minted by
/// `orchestrate.rs` against the live document, stored in a [`SourceSlot`].
/// Generic over the span shape (`BufferSpan`/`MinibufSpan`): each session
/// type's own `SlotSet` fixes it to that target's own shape, so there is no
/// runtime tag to mismatch. `id`/`state` are this module's own; a target's
/// own `impl Invocation<ItsSpan>` (in `buffer.rs`/`minibuf.rs`) adds the
/// constructor and span-specific reads.
pub(in crate::editor) struct Invocation<S> {
    pub(super) id: u64,
    pub(super) span: S,
    state: InvocationState,
}

impl<S> Invocation<S> {
    pub(super) fn new(span: S) -> Self {
        Self {
            id: widget_token::next(),
            span,
            state: InvocationState::Pending,
        }
    }

    pub(super) fn items(&self) -> &[CompletionItem] {
        match &self.state {
            InvocationState::Shown { items, .. } => items,
            InvocationState::Pending => &[],
        }
    }

    pub(super) fn incomplete(&self) -> bool {
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
/// two ids. A late answer to a superseded call is dropped, so a slow LSP
/// response can never overwrite a newer one. Generic over the id type
/// (`BufferSourceId`/`MinibufSourceId`) and span shape, both fixed by
/// whichever session type's `SlotSet` holds this.
pub(super) struct SourceSlot<Id, S> {
    pub(super) source: Id,
    pub(super) shown: Option<Invocation<S>>,
    pub(super) inflight: Option<Invocation<S>>,
}

impl<Id: Copy + PartialEq, S> SourceSlot<Id, S> {
    fn latest_id(&self) -> Option<u64> {
        self.inflight
            .as_ref()
            .or(self.shown.as_ref())
            .map(|inv| inv.id)
    }

    /// Whether the source has answered with at least one item. A slot
    /// narrowed to zero *matches* is still live (Backspace can bring its
    /// items back); one that answered empty, or whose token the cursor
    /// left, is not.
    fn is_live(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|inv| !inv.items().is_empty())
    }

    /// The item at ranked index `i` of this slot's shown answer. Every
    /// caller already knows `i` came from a still-valid ranked entry (see
    /// [`SlotSet::ranked_indices`]), so this is `.expect`, not `Option`.
    pub(super) fn item(&self, i: usize) -> &CompletionItem {
        &self.shown.as_ref().expect("ranked").items()[i]
    }
}

/// Looks up a source id's static facts in the registry, implemented once
/// per id type so [`SlotSet`]'s generic methods (`rank_with`, `top`) don't
/// need a per-target match to find the right registry half.
pub(super) trait SourceId: Copy + PartialEq {
    fn facts(self, sources: &SourceRegistry) -> (&str, MatchKind, i64);
}

impl SourceId for BufferSourceId {
    fn facts(self, sources: &SourceRegistry) -> (&str, MatchKind, i64) {
        let e = sources.buffer_get(self);
        (&e.name, e.match_kind, e.priority)
    }
}

impl SourceId for MinibufSourceId {
    fn facts(self, sources: &SourceRegistry) -> (&str, MatchKind, i64) {
        let e = sources.minibuf_get(self);
        (&e.name, e.match_kind, e.priority)
    }
}

/// [`SlotSet::rank_with`]'s own scratch state, reborrowed disjointly from
/// `self` and threaded through [`score_slot`] as one bundle rather than
/// three separate parameters: every one of the three is reused across
/// every slot. `dedup_hidden` is `None` for a target with no cross-source
/// dedup concept (`Minibuf` invokes exactly one source, so there is never a
/// second slot to dedup against) rather than an always-empty set a session
/// with nothing to hide would otherwise have to carry.
struct ScoreCtx<'a> {
    matcher: &'a mut FuzzyMatcher,
    rank_scratch: &'a mut Vec<(u32, i64, u32, u32)>,
    dedup_hidden: Option<&'a rustc_hash::FxHashSet<(u32, u32)>>,
}

/// The filter texts one slot's items are scored against, each with the
/// "anchor" value (the live token start, in whatever unit the caller's `A`
/// is) that folds into the menu anchor when an item scored against it
/// survives.
pub(super) struct SlotTokens<A> {
    /// For an item with no token of its own. `None` skips those items (an
    /// out-of-range token against the live document, say).
    pub(super) base: Option<(String, A)>,
    /// One per invocation edit-range start, indexed by an item's
    /// `token_group`. `None` skips that group's items.
    pub(super) groups: Vec<Option<(String, A)>>,
}

impl<A> SlotTokens<A> {
    /// A slot whose items all share one token.
    pub(super) fn single(filter: String, anchor: A) -> Self {
        Self {
            base: Some((filter, anchor)),
            groups: Vec::new(),
        }
    }
}

/// A token's filter text, its parsed pattern and its anchor.
fn parse_token<'t, A: Copy>(
    matcher: &mut FuzzyMatcher,
    token: &'t Option<(String, A)>,
) -> Option<(&'t str, FuzzyPattern, A)> {
    let (filter, anchor) = token.as_ref()?;
    Some((filter.as_str(), matcher.parse(filter), *anchor))
}

/// Scores every item in `items` against its own token's filter per
/// `match_kind`, dropping a no-op item first, and pushes `(score, priority,
/// s, i)` into `ctx.rank_scratch` for each survivor. Returns the leftmost
/// anchor among the surviving items: [`SlotSet::rank_with`]'s own signal to
/// fold this slot's token start into the menu anchor.
fn score_slot<A: Ord + Copy>(
    ctx: &mut ScoreCtx<'_>,
    s: u32,
    items: &[CompletionItem],
    tokens: &SlotTokens<A>,
    match_kind: MatchKind,
    priority: i64,
) -> Option<A> {
    let base = parse_token(ctx.matcher, &tokens.base);
    let groups: Vec<_> = tokens
        .groups
        .iter()
        .map(|token| parse_token(ctx.matcher, token))
        .collect();
    let mut anchor: Option<A> = None;
    for (i, item) in items.iter().enumerate() {
        let token = match item.token_group {
            Some(g) => groups.get(g as usize).and_then(Option::as_ref),
            None => base.as_ref(),
        };
        let Some((filter, pattern, item_anchor)) = token else {
            continue;
        };
        if item.is_noop_for(filter) {
            continue;
        }
        if ctx
            .dedup_hidden
            .is_some_and(|hidden| hidden.contains(&(s, i as u32)))
        {
            continue;
        }
        let score = match match_kind {
            MatchKind::Fuzzy => ctx.matcher.score(pattern, &item.filter_text),
            MatchKind::String { case_sensitive } => {
                super::prefix_matches(&item.filter_text, filter, case_sensitive).then_some(0)
            }
            // The source already produced a finished, ordered result:
            // never excluded here; the rank key's tiebreak chain (skipping
            // sortText for a `Delegated` slot; see `rank_with`'s own sort
            // key) preserves that order via the final index-ascending key.
            MatchKind::Delegated => Some(0),
        };
        if let Some(score) = score {
            ctx.rank_scratch.push((score, priority, s, i as u32));
            anchor = Some(anchor.map_or(*item_anchor, |cur| cur.min(*item_anchor)));
        }
    }
    anchor
}

/// The generic core both completion targets share: every participating
/// source's slot, the current ranked list, the fuzzy matcher, and the
/// menu's selected row. `Id`/`S` are fixed to one target's own id/span
/// types by whichever session type holds this.
pub(super) struct SlotSet<Id, S> {
    slots: Vec<SourceSlot<Id, S>>,
    /// `(score, priority, slot, item)` for every surviving candidate,
    /// rebuilt and sorted by every [`Self::rank_with`] call. This *is* the
    /// set's own ranked list (`len`/`ranked_indices`/`rows_in` all read it
    /// directly). Retained across calls so per-keystroke filtering doesn't
    /// allocate a fresh `Vec` every time. `priority` is
    /// `BufferSourceEntry`/`MinibufSourceEntry`'s own `i64`, not narrowed:
    /// this tuple is sorted, never used as a lookup key, so there's no
    /// reason to risk a truncating cast.
    ranked: Vec<(u32, i64, u32, u32)>,
    /// Reusable scoring engine. `FuzzyProfile::Autocomplete` (see its doc)
    /// distinguishes this from the picker's own instance. One instance per
    /// session, consulted only for a `MatchKind::Fuzzy` slot.
    matcher: FuzzyMatcher,
    /// The menu's selected row. Reset to `0` by every [`Self::rank_with`]
    /// call, not by a separate step a caller might forget: every path that
    /// re-ranks the list changes what row `0` even means, so resetting it
    /// anywhere but at the one place the list itself is rebuilt would leave
    /// a window where the two disagree.
    selected: usize,
}

impl<Id, S> SlotSet<Id, S> {
    pub(super) fn new() -> Self {
        Self {
            slots: Vec::new(),
            ranked: Vec::new(),
            matcher: FuzzyMatcher::new(FuzzyProfile::Autocomplete),
            selected: 0,
        }
    }
}

impl<Id: Copy + PartialEq, S> SlotSet<Id, S> {
    /// Records a fresh call of `source`, superseding any still in flight
    /// for it. Returns the id the answer must carry.
    pub(super) fn invoke(&mut self, source: Id, invocation: Invocation<S>) -> u64 {
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
    /// latest call of any slot here: a superseded or already-replaced
    /// invocation, expected-normal for a late async source, never an
    /// error.
    /// `prepare` sees the answering source, the invocation's span and the
    /// items before they are stored.
    pub(super) fn contribute(
        &mut self,
        id: u64,
        mut items: Vec<CompletionItem>,
        incomplete: bool,
        prepare: impl FnOnce(Id, &mut S, &mut [CompletionItem]),
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
        prepare(slot.source, &mut invocation.span, &mut items);
        invocation.state = InvocationState::Shown { items, incomplete };
        slot.shown = Some(invocation);
        true
    }

    pub(super) fn is_pending(&self) -> bool {
        self.slots.iter().any(|s| s.inflight.is_some())
    }

    pub(super) fn is_live(&self) -> bool {
        self.slots.iter().any(SourceSlot::is_live)
    }

    /// Whether this set has nothing left to show and nothing on its way:
    /// every source answered empty, or the cursor typed out of every
    /// token.
    pub(super) fn is_spent(&self) -> bool {
        !self.is_pending() && !self.is_live()
    }

    /// Clears every still-`inflight` invocation, leaving any existing
    /// `shown` answer untouched. Returns whether anything was actually
    /// cleared. See `BufferSession::drop_stalled_invocations`'s own doc for
    /// why this recovery is always safe.
    pub(super) fn drop_stalled(&mut self) -> bool {
        let mut any = false;
        for slot in &mut self.slots {
            if slot.inflight.take().is_some() {
                any = true;
            }
        }
        any
    }

    /// The sources to call again after an edit: those that flagged their
    /// latest answer `isIncomplete`, and those still pending (their
    /// in-flight call saw an older document, so it is superseded rather
    /// than waited for).
    pub(super) fn sources_to_reinvoke(&self) -> Vec<Id> {
        self.slots
            .iter()
            .filter(|s| {
                s.inflight.is_some() || s.shown.as_ref().is_some_and(Invocation::incomplete)
            })
            .map(|s| s.source)
            .collect()
    }

    pub(super) fn slots_mut(&mut self) -> &mut [SourceSlot<Id, S>] {
        &mut self.slots
    }

    /// Number of candidates surviving the current ranking.
    pub(super) fn len(&self) -> usize {
        self.ranked.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ranked.is_empty()
    }

    /// The raw `(slot, item)` indices behind ranked position `idx`. Every
    /// other reader resolves `idx` through this first.
    fn ranked_indices(&self, idx: usize) -> Option<(u32, u32)> {
        let &(_, _, s, i) = self.ranked.get(idx)?;
        Some((s, i))
    }

    fn item(&self, s: u32, i: u32) -> &CompletionItem {
        self.slots[s as usize].item(i as usize)
    }

    pub(super) fn selected_item(&self, idx: usize) -> Option<&CompletionItem> {
        let (s, i) = self.ranked_indices(idx)?;
        Some(self.item(s, i))
    }

    /// The source/invocation/item behind ranked position `idx`. Readers
    /// that only want the item should use [`Self::selected_item`] instead.
    pub(super) fn ranked(&self, idx: usize) -> Option<(Id, &Invocation<S>, &CompletionItem)> {
        let (s, i) = self.ranked_indices(idx)?;
        let slot = &self.slots[s as usize];
        let inv = slot.shown.as_ref()?;
        Some((slot.source, inv, &inv.items()[i as usize]))
    }

    /// Row content for the candidates at `range` in ranked order: the only
    /// way rows leave this set, and `range` is `menu_window`'s own window
    /// (`hume_ui::popup`), so a frame formats at most `MAX_MENU_ROWS` rows
    /// and never the whole ranked list.
    pub(super) fn rows_in(&self, range: std::ops::Range<usize>) -> Vec<hume_ui::popup::MenuRow> {
        range
            .filter_map(|idx| self.selected_item(idx))
            .map(CompletionItem::menu_row)
            .collect()
    }

    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    /// Moves the menu selection by one row, wrapping at either end. `false`
    /// on an empty ranked list: a no-op, so a caller can't divide by, or
    /// subtract from, zero.
    pub(super) fn step_selection(&mut self, forward: bool) -> bool {
        let n = self.ranked.len();
        if n == 0 {
            return false;
        }
        self.selected = if forward {
            (self.selected + 1) % n
        } else {
            self.selected.checked_sub(1).unwrap_or(n - 1)
        };
        true
    }
}

impl<Id: SourceId, S> SlotSet<Id, S> {
    pub(super) fn top(&self, n: usize, sources: &SourceRegistry) -> Vec<serde_json::Value> {
        (0..n.min(self.ranked.len()))
            .filter_map(|idx| {
                let (s, i) = self.ranked_indices(idx)?;
                let slot = &self.slots[s as usize];
                let (name, ..) = slot.source.facts(sources);
                Some(slot.item(i as usize).to_json(name))
            })
            .collect()
    }

    /// Re-scores every shown item against its own slot's token text, with
    /// its source's `MatchKind`, dropping any item that's a no-op against
    /// that text first (`CompletionItem::is_noop_for`) regardless of
    /// `MatchKind`, and any item `hidden` already marked as a
    /// lower-priority duplicate. Rank key: score descending, then source
    /// priority descending (a tiebreaker only, match quality stays king,
    /// applied before sortText so a higher-priority source's item wins a
    /// tie regardless of how its label sorts; direction matches
    /// `register_sign_source`'s own `(priority desc, name asc)`), then
    /// sortText ascending (the server's own ordering hint, the *only*
    /// signal left on an empty filter: nucleo scores every haystack `0`
    /// for an empty pattern), skipped for a `Delegated` slot, which
    /// contributes no distinguishing sortText regardless of what its own
    /// items' `sort_text` field holds; then slot and item index (sortText
    /// is very often duplicated across a server's items).
    ///
    /// `token_of` returns the filter texts and anchors this invocation's
    /// items are scored against ([`SlotTokens`]), or `None` to skip the
    /// whole slot (no live document to read a token from). Resets
    /// [`Self::selected`] to `0` and returns the leftmost anchor among
    /// contributing slots, or `None` with nothing ranked.
    pub(super) fn rank_with<A: Ord + Copy>(
        &mut self,
        sources: &SourceRegistry,
        hidden: Option<&rustc_hash::FxHashSet<(u32, u32)>>,
        mut token_of: impl FnMut(&Invocation<S>) -> Option<SlotTokens<A>>,
    ) -> Option<A> {
        let Self {
            slots,
            ranked,
            matcher,
            selected,
        } = self;
        ranked.clear();
        let mut ctx = ScoreCtx {
            matcher,
            rank_scratch: ranked,
            dedup_hidden: hidden,
        };
        let mut anchor: Option<A> = None;
        for (s, slot) in slots.iter().enumerate() {
            let Some(inv) = &slot.shown else { continue };
            let Some(tokens) = token_of(inv) else {
                continue;
            };
            let (_, match_kind, priority) = slot.source.facts(sources);
            let contributed = score_slot(
                &mut ctx,
                s as u32,
                inv.items(),
                &tokens,
                match_kind,
                priority,
            );
            if let Some(a) = contributed {
                anchor = Some(anchor.map_or(a, |cur: A| cur.min(a)));
            }
        }
        let sort_key_of = |s: u32, i: u32| -> &str {
            let slot = &slots[s as usize];
            if slot.source.facts(sources).1 == MatchKind::Delegated {
                ""
            } else {
                &slot.item(i as usize).sort_text
            }
        };
        ctx.rank_scratch.sort_unstable_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| sort_key_of(a.2, a.3).cmp(sort_key_of(b.2, b.3)))
                .then((a.2, a.3).cmp(&(b.2, b.3)))
        });
        *selected = 0;
        anchor
    }
}
