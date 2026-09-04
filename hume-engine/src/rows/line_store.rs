//! What a [`RowMap`](super::RowMap) knows about the lines it is walking.
//!
//! One store holds one entry per line visited: the line's virtual rows and
//! block shape, plus its formatted content rows once anything has needed
//! them. A `RowMap` borrows a store for its lifetime and reads everything
//! through it — there is no second copy of a line anywhere.
//!
//! # One store per pane, shared by everything that walks it
//!
//! A [`PaneLineStore`] lives on the [`Pane`](crate::pane::Pane) it describes,
//! so every walker of that pane reads and writes the same entries:
//!
//! - The frame's two passes — the editor's scroll step, deciding where the
//!   viewport lands, and the render pass drawing it. Whichever reaches a line
//!   first formats it; the other finds it already done. Under a wrapping mode
//!   they walk nearly the same range, so this is most of the work either does.
//! - The between-frame consumers (mouse mapping, visual movement, the
//!   `z`-scroll commands), which run after a frame has drawn and so find the
//!   visible range already formatted.
//!
//! Ownership by the pane is what makes that sharing safe to state: two panes
//! can show the same buffer at the same width and resolve a bit-identical
//! [`StoreKey`], which names no pane — so a store reachable from both would
//! serve one pane's block shapes for the other.
//!
//! # Two-phase entries
//!
//! [`LineEntry`] carries its block shape from the moment it exists, and its
//! [`LineFormat`] only once something formats the line. The gap is real
//! rather than an artefact: under `WrapMode::None` a line's block shape is
//! known without formatting anything (one content row, whatever its length),
//! so an entry sits metadata-only until a render walk reaches it. Under a
//! wrapping mode the row *count* is the formatter's output, so the format
//! arrives with the shape.
//!
//! # Scope key
//!
//! Entries describe a line's *block shape* under one set of formatting
//! inputs. [`StoreKey`] carries all of them, and a store whose key changes
//! drops what it holds. That is what keeps the two passes honest about
//! sharing:
//!
//! - Wrapping: both passes resolve the same key, so they share.
//! - `WrapMode::None`: the render pass clips to an `h_window` and the scroll
//!   pass does not. A windowed format *drops* leading graphemes rather than
//!   truncating, so a windowed and an unwindowed format are not
//!   interchangeable — but block shape (virtual rows, `before`/`after`) does
//!   not depend on the window, so both passes still share *that*. `h_window`
//!   is recorded on [`LineFormat`] instead of on `StoreKey`, and
//!   [`LineFormat::covers`](crate::format::LineFormat::covers) is what keeps
//!   a windowed and an unwindowed format from answering for each other,
//!   without evicting the shape they agree on.
//!
//! `buffer_tag` is opaque here on purpose: identifying a buffer's content
//! means reading its identity, its content generation and its decoration
//! store's generation, and this crate depends on neither the editing nor the
//! editor crate. The caller supplies those as a [`BufferTag`], which this
//! module only ever compares.
//!
//! # Lifetime: never across a frame
//!
//! [`EngineView::begin_frame`](crate::pipeline::EngineView::begin_frame)
//! rewinds every pane's store, and that is a correctness requirement rather
//! than hygiene. The per-pane inlay-hint and
//! EOL-text mirrors are rebuilt each frame *filtered to the viewport that
//! frame shows*, without bumping the decoration store's generation — so a
//! line scrolling into view can gain inline inserts that change its wrap-row
//! count while both the buffer's content generation and the decoration
//! generation stay put. Nothing in [`StoreKey`] can see that, and nothing
//! needs to while entries never outlive the frame that made them. Reusing
//! them across frames would need the visible window in the key.

use rustc_hash::FxHashMap;

use crate::format::{LineFormat, VirtualRowScratch};
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::VirtualLine;

/// The caller's identification of a buffer *state* — which buffer, at which
/// content generation, with which decorations — as the three numbers naming
/// it.
///
/// Compared as a unit and never interpreted, so which component is which is
/// this crate's business only insofar as the caller stays consistent about
/// it. Three numbers rather than one the caller hashed them into: the
/// comparison here is exact, and a fold would trade that for a probabilistic
/// one to buy nothing — a key this small is copied, not stored at scale.
pub type BufferTag = [u64; 3];

/// Everything a line's block shape depends on besides the line's own text.
///
/// Deliberately excludes the horizontal clip (`h_window`) a `WrapMode::None`
/// render applies: block shape doesn't depend on it, only the formatted rows
/// do, so that lives on `LineFormat` instead — see the module doc's "Scope
/// key" section.
///
/// Also excludes the pane's content width, which reaches formatting through
/// `wrap_mode` and nowhere else: the mode is stored already resolved, so a
/// width that matters is already in this key, and one that doesn't (a resize
/// under `WrapMode::None`, or under a wrap mode with an explicit column)
/// would only rewind a store whose entries stayed valid.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct StoreKey {
    pub buffer_tag: BufferTag,
    pub wrap_mode: WrapMode,
    pub tab_width: u8,
    pub whitespace: WhitespaceConfig,
}

/// A pane's store together with the buffer tag its entries are keyed under.
///
/// One argument rather than two because the pairing is not a caller's to
/// vary: a store handed a tag other than the one its entries were built for
/// would serve one buffer state's block shapes for another, and every caller
/// reads both out of the same editor state anyway.
pub struct StoreScope<'a> {
    pub store: &'a mut PaneLineStore,
    pub buffer_tag: BufferTag,
}

/// One line's block shape, and its formatted rows once it has them.
pub struct LineEntry {
    /// The line this describes. Held so an entry index alone addresses a
    /// line: every caller working from one reads the line back here rather
    /// than carrying it alongside and risking the two disagreeing.
    pub line: usize,
    /// This line's virtual rows, `Before` ones first — the order
    /// [`crate::providers::VirtualLineAnchor::sort_key`] imposes, so the
    /// `i`th `After` row is at index `before + i`.
    pub virtual_lines: Vec<VirtualLine>,
    pub before: usize,
    /// The line's content rows. `format.extent` is `None` until something
    /// formats them.
    pub format: LineFormat,
}

impl LineEntry {
    fn new(line: usize) -> Self {
        Self {
            line,
            virtual_lines: Vec::new(),
            before: 0,
            format: LineFormat::new(),
        }
    }

    /// Virtual rows anchored `After` this line — whatever `before` doesn't
    /// claim, since `virtual_lines` holds the two groups back to back.
    pub fn after(&self) -> usize {
        self.virtual_lines.len() - self.before
    }

    /// Reuse this entry for `line`, keeping the allocations behind it.
    ///
    /// Only re-points an entry at a new line — reclaiming an oversized buffer
    /// is [`PaneLineStore::rewind`]'s job, since a spare slot is reached again
    /// only if some later frame walks at least that many lines, which is not
    /// something any frame can be relied on to do.
    fn rebind(&mut self, line: usize) {
        self.line = line;
        self.virtual_lines.clear();
        self.before = 0;
        self.format.reset();
    }
}

/// The lines one [`RowMap`](super::RowMap) is working with.
///
/// `entries` is a free list: [`PaneLineStore::rewind`] drops what is live but
/// keeps each entry's allocation — bar one grown past
/// [`crate::format::LineFormat::reset_and_shrink`]'s ceiling, so a store
/// settles into reusing what it already has without one pathologically wide
/// line pinning its capacity for the pane's whole life. An entry only ever
/// holds buffers for a line something actually formatted; one walked for its
/// block shape alone carries none.
#[derive(Default)]
pub struct PaneLineStore {
    key: Option<StoreKey>,
    entries: Vec<LineEntry>,
    /// `entries[..live]` describe lines; the rest are spares.
    live: usize,
    /// Buffer line -> index into `entries`.
    index: FxHashMap<usize, usize>,
    /// The virtual row currently being laid out. Separate from any entry's
    /// `format`: a `Before` row renders ahead of its line's content rows, so
    /// laying it out must not disturb them.
    virtual_row: VirtualRowScratch,
}

impl PaneLineStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Point this store at `key`, dropping what it holds if that differs from
    /// what its entries were built under.
    pub(super) fn scope(&mut self, key: StoreKey) {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.rewind();
        }
    }

    /// Drop what this store holds, keeping the allocations behind it — bar
    /// one grown past its ceiling, which is handed back here.
    ///
    /// Called once per frame via [`EngineView::begin_frame`](crate::pipeline::EngineView::begin_frame),
    /// and a correctness requirement rather than hygiene — see the module doc.
    ///
    /// The shrink belongs at this boundary rather than at
    /// [`LineEntry::rebind`], the other point an entry changes hands: a slot
    /// is rebound only by a later frame that walks at least as many lines as
    /// the frame that grew it, so a pane that shows one pathologically wide
    /// line and then a handful of short ones would otherwise pin that
    /// capacity for as long as the pane lives.
    pub(crate) fn rewind(&mut self) {
        for entry in &mut self.entries[..self.live] {
            entry.format.reset_and_shrink();
        }
        self.live = 0;
        self.index.clear();
        self.virtual_row.clear_and_shrink();
    }

    /// The entry for `line`, if this store has one.
    pub(super) fn find(&self, line: usize) -> Option<usize> {
        self.index.get(&line).copied()
    }

    /// Start an entry for `line`, reusing a spare's allocations. The caller
    /// fills in the block shape; the format arrives later, if at all.
    pub(super) fn insert(&mut self, line: usize) -> usize {
        debug_assert!(
            self.find(line).is_none(),
            "line {line} already has an entry; the caller must reuse it"
        );
        let idx = self.live;
        match self.entries.get_mut(idx) {
            Some(spare) => spare.rebind(line),
            None => self.entries.push(LineEntry::new(line)),
        }
        self.index.insert(line, idx);
        self.live += 1;
        idx
    }

    pub(super) fn entry(&self, idx: usize) -> &LineEntry {
        &self.entries[idx]
    }

    pub(super) fn entry_mut(&mut self, idx: usize) -> &mut LineEntry {
        &mut self.entries[idx]
    }

    /// An entry alongside the virtual-row scratch, which laying one of its
    /// virtual rows out needs at the same time. Split here because they are
    /// disjoint fields of this struct and only this struct can say so.
    pub(super) fn entry_and_virtual_row(
        &mut self,
        idx: usize,
    ) -> (&LineEntry, &mut VirtualRowScratch) {
        (&self.entries[idx], &mut self.virtual_row)
    }
}
