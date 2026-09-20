//! Completion orchestration: a Rust store holds every contributing source's
//! items and does the per-keystroke filter/rank; Steel drives `begin!`/
//! `add-items!`/`update-filter!`/`top`/`accept!`/`dismiss!`. One singleton
//! session per editor (not per buffer) — starting a new one (`begin!`)
//! replaces the old; merging into the current one (`add-items!`) is a
//! second source joining, gated on `begin!`'s own session token so a stale
//! source's late answer can't land in the wrong session.
//!
//! One session type serves both completion systems — [`CompletionTarget`]
//! picks where an accepted item lands (a buffer edit, or a splice into the
//! minibuffer's own input) — but not how a source's items are gathered: a
//! [`MatchKind::Delegated`] source (`:e`/`:set`) still does its own
//! per-keystroke work outside this session, since its candidate universe
//! (a directory listing, a parse phase) isn't stable the way a `Fuzzy` or
//! `String` source's is. See [`MatchKind`]'s own doc.

mod accept;

use hume_editing::changeset::{Assoc, ChangeSet};
use hume_engine::pipeline::{BufferId, PaneId};
use hume_rope::offset::CharOffset;
use rustc_hash::FxHashMap;

use crate::editor::EditorState;
use crate::editor::fuzzy::{FuzzyMatcher, FuzzyProfile};
use crate::editor::widget_token;

use super::item::CompletionItem;

/// How a source's items are matched against the typed filter — a per-source
/// declaration, since one session can (in principle) mix sources with
/// different universes.
///
/// - `Fuzzy` — nucleo scoring (`FuzzyMatcher`). The source's candidate
///   universe is stable (or grows incrementally via `completion-add-items!`,
///   the LSP `isIncomplete` flow); [`CompletionSession::update_filter`]
///   re-scores it locally on every keystroke without re-invoking the source.
/// - `String { case_sensitive }` — a boundary-safe prefix gate (`starts_with`,
///   or `eq_ignore_ascii_case` on the matching-length head when
///   `case_sensitive` is `false`), tied score on a match. The source's
///   universe is *also* stable (e.g. "every registered command name") — only
///   the matching rule differs from `Fuzzy`.
/// - `Delegated` — the source computes its own finished, already-ordered
///   result fresh from the live input every attempt (a directory read, a
///   multi-phase parse); this session does no scoring of its own for these
///   items. Given a tied score and an empty `sort_text` at construction
///   (see the item constructor `Delegated` sources use), the rank key's
///   final tiebreak — index ascending — preserves the source's own order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// What happens while a completion popup is open and the user keeps typing —
/// a UI modality decided once, by whichever source calls [`CompletionSession::
/// begin_buffer`]/[`CompletionSession::begin_minibuf`], and unchanged for the
/// rest of the session's life: it cannot be two things at once.
///
/// - `CycleApply` — any further typing dismisses the popup outright; Tab
///   only cycles the list already computed. Today's minibuffer (`:`) UX,
///   pairing naturally with `MatchKind::String`/`Delegated` sources, whose
///   universes are re-derived fresh on the *next* attempt rather than
///   refiltered live.
/// - `SelectAccept` — further typing refilters the open menu in place.
///   Today's Insert-mode/LSP UX, pairing with `MatchKind::Fuzzy`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum Interaction {
    CycleApply,
    SelectAccept,
}

/// Fields meaningful only for a [`CompletionTarget::Buffer`] session — moved
/// behind the variant rather than left flat on [`CompletionSession`], since a
/// [`CompletionTarget::Minibuf`] session has no buffer, pane, or generation
/// to track at all.
struct BufferTarget {
    bid: BufferId,
    /// Pane the session began in — `accept` only proceeds while this pane is
    /// still focused. A completion resolved against a pane the user has
    /// since navigated away from has no well-defined live cursor to land at,
    /// and `PaneBufferState`'s own `ensure` would otherwise silently
    /// fabricate one (see `accept`'s pane precondition).
    pane_id: PaneId,
    /// `anchor()`'s value at `begin_buffer()` time — paired with
    /// `rope_at_begin` as the coordinate system a server's `textEdit` range
    /// was computed against. Unlike the derived `anchor()`, never remapped:
    /// it's a fixed reference point, not a position tracked through edits.
    anchor_at_begin: CharOffset,
    /// The buffer's rope at `begin_buffer()` time — an O(1) clone (ropey is
    /// structurally shared). A server's wire `textEdit` range is computed
    /// against the document as it stood at the completion *request*, which
    /// is this snapshot, not whatever the buffer holds by `accept()` time:
    /// if an earlier cursor on the same line has since inserted text (only
    /// possible when the primary isn't the first cursor), decoding the
    /// server's range against the live rope would land on the wrong chars.
    rope_at_begin: ropey::Rope,
    /// Every edit observed on this session's buffer since `begin_buffer`
    /// (via `observe_edit`), composed into one changeset — the single
    /// source of truth for "where a begin-time position sits now." Paired
    /// with `rope_at_begin`, this is the coordinate transform a server's
    /// wire positions (computed against the request document) need in
    /// order to land correctly on the live document: decode once against
    /// the frozen snapshot, then map forward through every keystroke
    /// since, rather than approximating drift as a scalar shift.
    cs_since_begin: ChangeSet,
    /// Buffer generation as of the last `begin_buffer`/`update_filter` call
    /// — `accept!` rejects if the buffer changed by any other path since.
    generation_at_begin: u64,
}

/// Where an accepted item lands — the one axis `accept()` itself branches
/// on; unrelated to how a source's items were matched ([`MatchKind`]) or
/// what further typing does ([`Interaction`]).
enum CompletionTarget {
    Buffer(BufferTarget),
    /// Byte offset in the minibuffer's own input where the completed token
    /// starts — the anchor `accept` splices `insert_text` from, over
    /// `span_start..cursor`, via `Layer::minibuf_mut` (never a direct field
    /// access — `CompletionLayer` doesn't own the minibuffer, `CommandLayer`
    /// does).
    Minibuf {
        span_start: usize,
    },
}

pub(in crate::editor) struct CompletionSession {
    target: CompletionTarget,
    items: Vec<CompletionItem>,
    /// Ranked indices into `items`, rebuilt by every `update_filter` call.
    filtered: Vec<u32>,
    /// Retained across `update_filter` calls so per-keystroke filtering
    /// doesn't allocate a fresh Vec every time. `(score, item index)`.
    rank_scratch: Vec<(u32, u32)>,
    filter: String,
    /// Reusable scoring engine — `FuzzyProfile::Autocomplete` (see its doc)
    /// distinguishes this from the picker's own instance. Only consulted for
    /// a `MatchKind::Fuzzy` item; unused (but always present — one instance
    /// per session, not per source) otherwise.
    matcher: FuzzyMatcher,
    /// Every source that has contributed to this session, keyed by name —
    /// its latest priority, match kind, and `isIncomplete` flag. One entry
    /// per source that has ever called [`Self::add_items`] (including the
    /// first, via `begin_buffer`/`begin_minibuf`); an entry is overwritten,
    /// never removed, by a same-source re-add. `incomplete()` is the OR
    /// across every entry's flag — `update_filter`'s rank key reads each
    /// item's own entry for its priority tiebreaker and match kind.
    sources: FxHashMap<Box<str>, SourceState>,
    /// Decided once, by whichever source first calls `begin_buffer`/
    /// `begin_minibuf` — see [`Interaction`]'s own doc.
    interaction: Interaction,
    /// Identifies this session to Steel and to
    /// `input_stack::completion::session_for_token`, the guard
    /// `completion-add-items!` checks before reaching a `&mut
    /// CompletionSession` at all — mirrors `PickerSession::token`'s own
    /// doc and purpose exactly.
    token: u64,
    /// Row labels for the current `filtered` set, pre-measured to a menu box
    /// width, built lazily by [`Self::menu_rows`] and invalidated by
    /// `update_filter`. `filtered` only changes there — not on menu
    /// navigation (selecting a different row) or on an unrelated frame
    /// redraw — so caching here means a per-frame render sync doesn't
    /// re-format and re-measure every candidate for a menu whose contents
    /// haven't moved. `MenuRows` carries its labels by `Arc`, so a caller
    /// building a `PopupState` (which itself shares its `lines` by `Arc`)
    /// gets a cheap refcount bump instead of a fresh clone of every label.
    menu_cache: Option<hume_ui::popup::MenuRows>,
}

/// UI state for an open completion session — kept separate from
/// `CompletionSession` itself (which deliberately has no `selected`) so the
/// session's filtering/accept logic stays free of rendering concerns.
/// Shared by both targets — the menu-navigation keys (Tab/Down/BackTab/Up)
/// are already target-agnostic.
pub(in crate::editor) struct CompletionMenuUi {
    pub(in crate::editor) selected: usize,
}

/// One source's latest contribution metadata — see
/// `CompletionSession::sources`'s doc for why priority, match kind, and
/// `isIncomplete` live together in one map rather than several that would
/// have to stay in sync.
struct SourceState {
    priority: i64,
    match_kind: MatchKind,
    incomplete: bool,
}

impl CompletionSession {
    /// The `Buffer`-target fields, or `None` for a `Minibuf` session.
    fn buffer_target(&self) -> Option<&BufferTarget> {
        match &self.target {
            CompletionTarget::Buffer(b) => Some(b),
            CompletionTarget::Minibuf { .. } => None,
        }
    }

    /// Char offset where the completed token starts — the anchor the
    /// completion menu positions itself at (not the live cursor, which
    /// drifts as the user types further into the token). Derived by mapping
    /// `anchor_at_begin` forward through every edit observed so far —
    /// `Assoc::Before`: the anchor marks the token's start, so text inserted
    /// exactly at it belongs to the token and the anchor must stay left of
    /// it, same association `apply_doc_edit_grouped` uses for a
    /// `TypedRun`'s own anchors.
    ///
    /// A completion accept's own replacement edit (`accept.rs`) is one more
    /// edit `apply_doc_edit_grouped` remaps an open `TypedRun` through, same
    /// as any keystroke — so if the accepted item's `textEdit`
    /// replaces text typed before the Insert session began (e.g. `A` mid-
    /// identifier, type one char, then accept), the selected typed run on
    /// Esc grows to cover the whole replacement, not just the char actually
    /// keyed. That's intended, not a pin-tracking bug: the accept's own edit
    /// rewrote that whole span, so every character in it was written by this
    /// session, and selecting the freshly completed token is the useful
    /// outcome.
    ///
    /// `Buffer`-target only — every caller reaches this from an Insert-mode-
    /// specific code path, where the topmost `CompletionLayer` (if any) is
    /// never `Minibuf`-targeted (that target only ever sits above `Command`).
    pub(in crate::editor) fn anchor(&self) -> CharOffset {
        let b = self
            .buffer_target()
            .expect("anchor() is Buffer-target only");
        let mut positions = [b.anchor_at_begin];
        b.cs_since_begin
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
    /// there's no shorter edit history to fall back to. `Buffer`-target
    /// only, same reasoning as `anchor()`.
    pub(in crate::editor) fn observe_edit(&mut self, cs: &ChangeSet) -> bool {
        let b = match &mut self.target {
            CompletionTarget::Buffer(b) => b,
            CompletionTarget::Minibuf { .. } => {
                unreachable!("observe_edit is Buffer-target only")
            }
        };
        if cs.len_before() != b.cs_since_begin.len_after() {
            return false;
        }
        b.cs_since_begin = b.cs_since_begin.clone().compose(cs.clone());
        true
    }

    /// Whether any source's *latest* contribution was `isIncomplete` — gates
    /// `on-completion-refilter`. Recomputed fresh from every source's own
    /// flag on each read, so it moves in both directions: a slow source
    /// arriving incomplete via `completion-add-items!` can flip this from
    /// `false` to `true` on a session that began complete, and it flips
    /// back once that source's own re-add reports `false` (a source's
    /// re-add always carries its current flag, so a resolved source simply
    /// stops being counted).
    pub(in crate::editor) fn incomplete(&self) -> bool {
        self.sources.values().any(|s| s.incomplete)
    }

    /// `Buffer`-target only — same reasoning as `anchor()`.
    pub(in crate::editor) fn bid(&self) -> BufferId {
        self.buffer_target()
            .expect("bid() is Buffer-target only")
            .bid
    }

    /// The minibuffer-splice anchor, for a `Minibuf`-target session —
    /// `None` for `Buffer`. The one way code outside this module learns
    /// which target a session has, since `CompletionTarget` itself stays
    /// private.
    pub(in crate::editor) fn minibuf_span_start(&self) -> Option<usize> {
        match &self.target {
            CompletionTarget::Minibuf { span_start } => Some(*span_start),
            CompletionTarget::Buffer(_) => None,
        }
    }

    /// The interaction model this session was opened with — see
    /// [`Interaction`]'s own doc.
    pub(in crate::editor) fn interaction(&self) -> Interaction {
        self.interaction
    }

    /// Identifies this session to Steel — see the `token` field's own doc
    /// for the race it closes.
    pub(in crate::editor) fn token(&self) -> u64 {
        self.token
    }

    /// Number of candidates surviving the current filter — cheap count for
    /// callers (menu navigation, the visible-menu check) that don't need the
    /// items themselves; unlike `top(n).len()`, this doesn't serialize any
    /// candidate to JSON.
    pub(in crate::editor) fn len(&self) -> usize {
        self.filtered.len()
    }

    /// Whether the current filter matches nothing. A session can be open
    /// with this `true` — narrowed to empty by continued typing, or an
    /// `isIncomplete` list awaiting an async re-request — in which case no
    /// menu is visibly shown.
    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    /// The item behind `filtered[idx]`, for a caller (`accept`) that already
    /// has a UI selection index rather than a raw item index.
    pub(in crate::editor) fn selected_item(&self, idx: usize) -> Option<&CompletionItem> {
        self.filtered.get(idx).map(|&i| &self.items[i as usize])
    }

    fn new(target: CompletionTarget, interaction: Interaction) -> Self {
        Self {
            target,
            items: Vec::new(),
            filtered: Vec::new(),
            rank_scratch: Vec::new(),
            filter: String::new(),
            matcher: FuzzyMatcher::new(FuzzyProfile::Autocomplete),
            sources: FxHashMap::default(),
            interaction,
            token: widget_token::next(),
            menu_cache: None,
        }
    }

    /// Returns `None` when `bid` isn't shown in the focused pane — a normal
    /// race (the async completion response landed after the user switched
    /// panes), not a caller bug, so this is silently absorbed by the caller
    /// rather than raised as a Steel error.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn begin_buffer(
        state: &EditorState,
        bid: BufferId,
        source: Box<str>,
        priority: i64,
        match_kind: MatchKind,
        interaction: Interaction,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> Option<Self> {
        let pid = state.focus.id();
        let anchor = state
            .focused_buffer_state(bid)?
            .selections()
            .primary()
            .head();
        let rope_at_begin = state.buffers.get(bid).text().rope().clone();
        let mut session = Self::new(
            CompletionTarget::Buffer(BufferTarget {
                bid,
                pane_id: pid,
                anchor_at_begin: anchor,
                cs_since_begin: ChangeSet::identity(rope_at_begin.len_chars()),
                rope_at_begin,
                // Real value stamped by `add_items` -> `update_filter`, below.
                generation_at_begin: 0,
            }),
            interaction,
        );
        session.add_items(
            state.buffers.get(bid).text_gen,
            source,
            priority,
            match_kind,
            incomplete,
            items,
        );
        Some(session)
    }

    /// A minibuffer completion session — always a single source, never
    /// `isIncomplete` (native completers finish synchronously), so `begin`
    /// and the merge step are one call here rather than two.
    pub(in crate::editor) fn begin_minibuf(
        span_start: usize,
        source: Box<str>,
        match_kind: MatchKind,
        items: Vec<CompletionItem>,
    ) -> Self {
        let mut session = Self::new(
            CompletionTarget::Minibuf { span_start },
            Interaction::CycleApply,
        );
        // `text_gen` is inert for a Minibuf-target session (no buffer, no
        // generation to stamp); `0` is never read.
        session.add_items(0, source, 0, match_kind, false, items);
        session
    }

    /// Merges `items` into the session under `source`, replacing that
    /// source's prior contribution wholesale — an add first evicts, then
    /// inserts, so the `isIncomplete` refilter flow (which re-invokes the
    /// same source against the same session) is idempotent rather than
    /// duplicating rows. `begin_buffer`/`begin_minibuf` are themselves the
    /// first call: the first source's own items arrive through this same
    /// path, so there is exactly one insertion point, not two. Re-ranks
    /// against the filter text already in effect (not an empty one) — a
    /// mid-session add must respect what the user has already typed.
    pub(in crate::editor) fn add_items(
        &mut self,
        text_gen: u64,
        source: Box<str>,
        priority: i64,
        match_kind: MatchKind,
        incomplete: bool,
        mut items: Vec<CompletionItem>,
    ) {
        for item in &mut items {
            item.source = source.clone();
        }
        self.items
            .retain(|item| item.source.as_ref() != source.as_ref());
        self.items.extend(items);
        self.sources.insert(
            source,
            SourceState {
                priority,
                match_kind,
                incomplete,
            },
        );
        self.update_filter(text_gen, self.filter.clone());
    }

    /// Re-ranks `items` against `text`, re-stamping the `Buffer` target's
    /// generation to `text_gen` (inert for `Minibuf` — there is no buffer to
    /// stamp) — the expected flow is "user types a char into the buffer
    /// (bumping its `text_gen`), then this is called with that new value and
    /// the new filter text," so a legitimate keystroke must not itself look
    /// like the buffer-changed-out-from-under-us case `accept!` guards
    /// against. Takes `text_gen` rather than `&EditorState`: every caller
    /// now reaches this method through a mutable borrow of the session that
    /// is itself nested inside `EditorState.input`, so a second, immutable
    /// borrow of the whole struct alongside it would alias.
    pub(in crate::editor) fn update_filter(&mut self, text_gen: u64, text: String) {
        self.filter = text;
        if let CompletionTarget::Buffer(b) = &mut self.target {
            b.generation_at_begin = text_gen;
        }
        self.menu_cache = None;
        self.rank_scratch.clear();
        let pattern = self.matcher.parse(&self.filter);
        for (i, item) in self.items.iter().enumerate() {
            let source = self
                .sources
                .get(item.source.as_ref())
                .expect("every item's source has a live entry, stamped by add_items");
            let score = match source.match_kind {
                MatchKind::Fuzzy => self.matcher.score(&pattern, &item.filter_text),
                MatchKind::String { case_sensitive } => {
                    prefix_matches(&item.filter_text, &self.filter, case_sensitive).then_some(0)
                }
                // The source already produced a finished, ordered result —
                // never excluded here; the rank key's tiebreak chain (empty
                // `sort_text`, see the item constructor these sources use)
                // preserves that order via the final index-ascending key.
                MatchKind::Delegated => Some(0),
            };
            if let Some(score) = score {
                self.rank_scratch.push((score, i as u32));
            }
        }
        // Score descending, then source priority descending — a tiebreaker
        // only (match quality stays king), applied before sortText so a
        // higher-priority source's item wins a tie regardless of how its
        // label sorts. Priority direction matches `register_sign_source`'s
        // own `(priority desc, name asc)` convention: a higher number is a
        // more important source, same as it's a more important sign.
        // sortText ascending next — the server's own ordering hint, which
        // is the *only* signal left on an empty filter with a single
        // source (nucleo scores every haystack `0` for an empty pattern, so
        // every item ties on every key above). Ascending index last, since
        // sortText is very often duplicated across a server's items and the
        // triple alone wouldn't be a unique key.
        let items = &self.items;
        let sources = &self.sources;
        let priority_of = |item: &CompletionItem| {
            sources
                .get(item.source.as_ref())
                .expect("every item's source has a live entry, stamped by add_items")
                .priority
        };
        self.rank_scratch.sort_unstable_by(|a, b| {
            let item_a = &items[a.1 as usize];
            let item_b = &items[b.1 as usize];
            b.0.cmp(&a.0)
                .then_with(|| priority_of(item_b).cmp(&priority_of(item_a)))
                .then_with(|| item_a.sort_text.cmp(&item_b.sort_text))
                .then(a.1.cmp(&b.1))
        });
        self.filtered.clear();
        self.filtered
            .extend(self.rank_scratch.iter().map(|&(_, i)| i));
    }

    pub(in crate::editor) fn top(&self, n: usize) -> Vec<serde_json::Value> {
        self.filtered
            .iter()
            .take(n)
            .map(|&i| self.items[i as usize].to_json())
            .collect()
    }

    /// Row labels for every candidate in `filtered` (not just the visible
    /// window — the menu box's width has to stay stable as the user scrolls
    /// past wider or narrower rows), pre-measured to their menu box width.
    /// Built once per `filtered` set — see [`Self::menu_cache`]'s doc — so a
    /// caller redrawing the same unchanged menu every frame reads the cache
    /// instead of reformatting and re-measuring every candidate again.
    pub(in crate::editor) fn menu_rows(&mut self) -> hume_ui::popup::MenuRows {
        if self.menu_cache.is_none() {
            let labels: Vec<String> = self
                .filtered
                .iter()
                .map(|&i| self.items[i as usize].menu_row_label())
                .collect();
            self.menu_cache = Some(hume_ui::popup::MenuRows::measure(std::sync::Arc::new(
                labels,
            )));
        }
        self.menu_cache
            .clone()
            .expect("populated by the check above")
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
mod tests {
    use super::{CompletionItem, CompletionSession, MatchKind, prefix_matches};

    /// Regression for the cache the `menu_cache` field doc describes:
    /// `update_filter` must invalidate it, not just populate it once. A
    /// deleted `self.menu_cache = None` (session.rs's own `update_filter`)
    /// would leave `after` below still showing both items.
    #[test]
    fn menu_rows_cache_is_invalidated_by_update_filter() {
        let mut session = CompletionSession::begin_minibuf(
            0,
            "test".into(),
            MatchKind::String {
                case_sensitive: false,
            },
            vec![
                CompletionItem::plain("apple".into(), "apple".into(), "apple".into()),
                CompletionItem::plain("banana".into(), "banana".into(), "banana".into()),
            ],
        );
        let before_rows = session.menu_rows();
        let before: Vec<&str> = before_rows.labels().iter().map(String::as_str).collect();
        assert_eq!(
            before,
            vec!["apple", "banana"],
            "sanity: both items visible unfiltered"
        );

        session.update_filter(0, "ban".into());
        let after_rows = session.menu_rows();
        let after: Vec<&str> = after_rows.labels().iter().map(String::as_str).collect();
        assert_eq!(
            after,
            vec!["banana"],
            "menu_rows' cache must be invalidated by update_filter"
        );
    }

    #[test]
    fn prefix_matches_case_sensitive() {
        assert!(prefix_matches("write-quit", "write", true));
        assert!(!prefix_matches("write-quit", "Write", true));
    }

    #[test]
    fn prefix_matches_case_insensitive() {
        assert!(prefix_matches("write-quit", "Write", false));
        assert!(!prefix_matches("write-quit", "quit", false));
    }

    /// `haystack.get(..prefix.len())` (the case-insensitive branch) must not
    /// panic when `prefix.len()` lands mid-codepoint in a non-ASCII
    /// haystack — a byte-slice `haystack[..prefix.len()]` would. "ï"
    /// (U+00EF) occupies bytes 2-3 of "naïve-cmd", so a 3-byte prefix lands
    /// inside it; `.get()` returns `None` there instead of panicking.
    #[test]
    fn prefix_matches_non_ascii_boundary_does_not_panic() {
        assert!(!prefix_matches("naïve-cmd", "xyz", false));
        assert!(prefix_matches("naïve-cmd", "na", false));
    }

    #[test]
    fn prefix_matches_prefix_longer_than_haystack_does_not_panic() {
        assert!(!prefix_matches("q", "quit", false));
        assert!(!prefix_matches("q", "quit", true));
    }
}
