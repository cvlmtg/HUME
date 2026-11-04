//! Selection undo: the selections `undo-selection-change` and
//! `redo-selection-change` walk for one pane and one buffer, newest first.
//!
//! The walk has two sources. The live list holds the selections the pane
//! built since its last edit of the buffer, recorded where a command is
//! dispatched, from the command's [`SelectionEffect`] and whether it ran in
//! Extend mode ([`Record::of`]), and where a mouse drag selects text. A
//! selection command inserts an entry and extending continues the entry it
//! starts from. The pane's own edit clears the list,
//! and another pane's edit carries it. Past the live list, the walk reads the
//! buffer's undo tree: the selections each edit this pane made was made with
//! and left ([`SavedSelections`]). A single cursor is never a step.
//!
//! The native commands one Steel session runs record one entry per pane and
//! buffer, when the session ends ([`EditorState::close_selection_group`]).

use std::collections::VecDeque;

use hume_editing::edit::TextChange;
use hume_editing::history::SelectionPoint;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_editing::tracked::Tracked;
use hume_engine::pipeline::EngineView;

use crate::editor::EditorState;
use crate::editor::buffer::SavedSelections;
use crate::editor::commands::CommandPane;
use crate::editor::registry::SelectionEffect;

/// The most entries the live list keeps; the oldest is dropped first.
const CAPACITY: usize = 20;

/// How a change to the selections enters the live list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum Record {
    /// A new entry.
    Insert,
    /// Replaces the entry the selections were before the change, and
    /// inserts when they were none.
    Continue,
}

/// Where a change's selections go in the live list, decided by
/// [`PaneSelections::placement`] from the selections the change starts
/// from, before it runs. A replace holds the list's generation at that
/// point, and inserts instead once the list has changed since.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) struct Placement {
    replaces: Option<u64>,
}

impl Record {
    /// What a command with `effect` records, run in Extend mode when
    /// `extend`; `None` when it records nothing. A transform of the staged
    /// selections inserts in either mode, so the set it reduced stays
    /// reachable.
    pub(in crate::editor) fn of(effect: SelectionEffect, extend: bool) -> Option<Self> {
        match effect {
            SelectionEffect::Extends => extend.then_some(Self::Continue),
            SelectionEffect::Establishes if extend => Some(Self::Continue),
            SelectionEffect::Establishes | SelectionEffect::Composes => Some(Self::Insert),
        }
    }
}

/// Which way `walk` steps.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) enum Walk {
    Back,
    Forward,
}

/// A walk stopped where it would have read the undo tree, which it may not
/// while an edit session holds the buffer.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::editor) struct TreeRefused;

/// A step a walk can land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Point {
    /// An entry of the live list, by its index there.
    Live(usize),
    Tree(SelectionPoint),
}

/// A pane's selections for one buffer and the live list of the ones it
/// built since its last edit of it.
#[derive(Debug, Clone)]
pub(in crate::editor) struct PaneSelections {
    current: SelectionSet,
    /// Oldest first, every one for the buffer's current text.
    live: VecDeque<SelectionSet>,
    /// Bumped by every record, so a [`Placement`] decided before one
    /// inserts instead of replacing.
    generation: u64,
    /// Where the last walk landed, until a record or the pane's own edit.
    walked: Option<Point>,
    /// The running Steel session's change, until the session ends, the
    /// pane edits the buffer or a walk lands.
    deferred: Option<Deferred>,
}

/// The first change a Steel session made to a pane's selections: where its
/// entry goes and the selections it started from.
#[derive(Debug, Clone)]
struct Deferred {
    placement: Placement,
    before: Tracked<SelectionSet>,
}

impl PaneSelections {
    /// `current`, with an empty live list.
    pub(in crate::editor) fn new(current: SelectionSet) -> Self {
        Self {
            current,
            live: VecDeque::new(),
            generation: 0,
            walked: None,
            deferred: None,
        }
    }

    pub(in crate::editor) fn current(&self) -> &SelectionSet {
        &self.current
    }

    /// Replace the selections with `new`, a set for the buffer's current
    /// text.
    pub(in crate::editor) fn set(&mut self, new: SelectionSet) {
        self.current = new;
    }

    /// Carry the selections and the live list through `change`, an edit
    /// another pane made.
    pub(in crate::editor) fn carry(&mut self, change: &TextChange<'_>) {
        self.current.translate(change);
        for sels in &mut self.live {
            sels.translate(change);
        }
        if let Some(deferred) = &mut self.deferred {
            deferred.before.translate(change, |sels| {
                sels.translate(change);
            });
        }
    }

    /// Forget the live list and the walk: this pane edited the buffer.
    pub(in crate::editor) fn edited(&mut self) {
        self.live.clear();
        self.walked = None;
        self.deferred = None;
    }

    /// A copy for a pane split off this one, which walks from the present.
    pub(in crate::editor) fn split(&self) -> Self {
        Self {
            walked: None,
            deferred: None,
            ..self.clone()
        }
    }

    /// Where a change of `record` starting from the current selections puts
    /// the selections it leaves. Decided before the change and handed to
    /// [`Self::record`] after it.
    pub(in crate::editor) fn placement(&self, record: Record) -> Placement {
        let entry = match self.walked {
            None => self.live.back(),
            Some(Point::Live(index)) => self.live.get(index),
            Some(Point::Tree(_)) => None,
        };
        let replaces = match record {
            Record::Continue if entry.is_some_and(|e| e.eq_ignoring_sticky(&self.current)) => {
                Some(self.generation)
            }
            Record::Continue | Record::Insert => None,
        };
        Placement { replaces }
    }

    /// Store the current selections in the live list at `placement`. The
    /// entries newer than where a walk landed go. A single cursor is not
    /// stored.
    pub(in crate::editor) fn record(&mut self, placement: Placement) {
        if self.current.is_single_cursor() {
            return;
        }
        match self.walked.take() {
            Some(Point::Live(index)) => self.live.truncate(index + 1),
            Some(Point::Tree(_)) => self.live.clear(),
            None => {}
        }
        let stamp = Some(self.generation);
        self.generation += 1;
        if placement.replaces == stamp
            && let Some(last) = self.live.back_mut()
        {
            *last = self.current.clone();
            return;
        }
        self.live.push_back(self.current.clone());
        if self.live.len() > CAPACITY {
            self.live.pop_front();
        }
    }

    /// Note a change of `record` a Steel session made, `placement` and
    /// `before` taken before it ran. The session's first change stands for
    /// all of them, and inserts when any of them does.
    pub(in crate::editor) fn defer(
        &mut self,
        placement: Placement,
        before: Tracked<SelectionSet>,
        record: Record,
    ) {
        let deferred = self.deferred.get_or_insert(Deferred { placement, before });
        if record == Record::Insert {
            deferred.placement.replaces = None;
        }
    }

    /// Store the selections at the deferred change's placement if they
    /// differ from the ones it started from, `text` being the buffer's
    /// current text.
    pub(in crate::editor) fn record_deferred(&mut self, text: &BufferText) {
        let Some(Deferred { placement, before }) = self.deferred.take() else {
            return;
        };
        if before.get(text) != Some(&self.current) {
            self.record(placement);
        }
    }

    /// Take `count` steps (at least one) in direction `walk`, as far as the
    /// steps go, and make the selections the one it lands on. `saved` is
    /// the undo tree's part, `None` while the walk may not read it. A step
    /// is a set that is not a single cursor and differs from the selections
    /// it leaves, ignoring sticky columns. Back from selections that moved
    /// off where the last walk landed lands there first.
    ///
    /// # Errors
    /// [`TreeRefused`] when the walk starts from an undo-tree step or a
    /// step back needed the undo tree, and `saved` is `None`. The steps
    /// taken before it stay.
    pub(in crate::editor) fn walk(
        &mut self,
        walk: Walk,
        count: usize,
        saved: Option<&SavedSelections<'_>>,
    ) -> Result<(), TreeRefused> {
        let mut steps = Steps::new(&self.live, saved);
        let landed = self.walked.map(|point| steps.index_of(point)).transpose()?;
        // The index the walk tries next. Back tries the landed step first,
        // which it skips unless the selections moved off it.
        let mut next = match walk {
            Walk::Back => Some(landed.unwrap_or(0)),
            Walk::Forward => landed.and_then(|index| index.checked_sub(1)),
        };
        for _ in 0..count {
            loop {
                let Some(index) = next else {
                    return Ok(());
                };
                next = match walk {
                    Walk::Back => Some(index + 1),
                    Walk::Forward => index.checked_sub(1),
                };
                let Some((point, sels)) = steps.get(index)? else {
                    return Ok(());
                };
                if !sels.is_single_cursor() && !sels.eq_ignoring_sticky(&self.current) {
                    self.current = sels;
                    self.walked = Some(point);
                    self.deferred = None;
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Every step a walk can take, newest first: the live list's entries, then
/// the undo tree's points.
struct Steps<'a> {
    live: &'a VecDeque<SelectionSet>,
    saved: Option<&'a SavedSelections<'a>>,
    /// The tree's points, read the first time a step reaches them.
    tree: Option<Vec<SelectionPoint>>,
}

impl<'a> Steps<'a> {
    fn new(live: &'a VecDeque<SelectionSet>, saved: Option<&'a SavedSelections<'a>>) -> Self {
        Self {
            live,
            saved,
            tree: None,
        }
    }

    fn tree(&mut self) -> Result<&[SelectionPoint], TreeRefused> {
        let saved = self.saved.ok_or(TreeRefused)?;
        Ok(self.tree.get_or_insert_with(|| saved.points()))
    }

    /// The step at `index` and its selections, `None` past the oldest.
    fn get(&mut self, index: usize) -> Result<Option<(Point, SelectionSet)>, TreeRefused> {
        let Some(point) = self.point_at(index)? else {
            return Ok(None);
        };
        let sels = match point {
            Point::Live(live_index) => self.live[live_index].clone(),
            Point::Tree(point) => self
                .saved
                .expect("the tree's points came from it")
                .at(point),
        };
        Ok(Some((point, sels)))
    }

    /// The point at `index`, `None` past the oldest. The inverse of
    /// [`Self::index_of`].
    fn point_at(&mut self, index: usize) -> Result<Option<Point>, TreeRefused> {
        let live = self.live.len();
        if index < live {
            return Ok(Some(Point::Live(live - 1 - index)));
        }
        Ok(self.tree()?.get(index - live).copied().map(Point::Tree))
    }

    /// The index of `point`. A tree point whose revision is gone (trimmed by
    /// `undo-levels`) resumes from the newest tree step.
    ///
    /// # Errors
    /// [`TreeRefused`] for a tree point while the walk may not read the tree.
    fn index_of(&mut self, point: Point) -> Result<usize, TreeRefused> {
        match point {
            Point::Live(live_index) => Ok(self.live.len() - 1 - live_index),
            Point::Tree(point) => {
                let offset = self.tree()?.iter().position(|&p| p == point);
                Ok(self.live.len() + offset.unwrap_or(0))
            }
        }
    }
}

/// A change to a pane's selections that the live list will record: how it
/// enters the list, where it goes, and the selections it starts from.
#[must_use = "a change records nothing until `finish` runs"]
pub(in crate::editor) struct PendingRecord {
    record: Record,
    placement: Placement,
    before: Tracked<SelectionSet>,
}

impl PendingRecord {
    /// Store `pane`'s selections at the placement if the change moved them,
    /// or defer them to the end of the running Steel session, and return
    /// whether it did.
    pub(in crate::editor) fn finish(
        self,
        state: &mut EditorState,
        view: &EngineView,
        pane: CommandPane,
    ) -> bool {
        let pair @ (_, bid) = (pane.pid(), pane.bid(view));
        let text = state.buffers.get(bid).text();
        let pane_state = pane.state_mut(&mut state.panes.entries, view);
        let changed = self.before.get(text) != Some(pane_state.selections());
        if changed {
            match &mut state.selection_group {
                None => pane_state.record_selections(self.placement),
                Some(group) => {
                    pane_state.defer_selections(self.placement, self.before, self.record);
                    if !group.contains(&pair) {
                        group.push(pair);
                    }
                }
            }
        }
        changed
    }
}

impl EditorState {
    /// Start recording a change of `record` to `pane`'s selections, which
    /// [`PendingRecord::finish`] stores once the change has run.
    pub(in crate::editor) fn begin_selection_record(
        &self,
        view: &EngineView,
        pane: CommandPane,
        record: Record,
    ) -> PendingRecord {
        let text = self.buffers.get(pane.bid(view)).text();
        let state = pane.state(&self.panes.entries, view);
        PendingRecord {
            record,
            placement: state.selection_placement(record),
            before: Tracked::new(state.selections().clone(), text),
        }
    }

    /// End the running Steel session's group: store the selections each
    /// pane it changed through native commands deferred, for the panes and
    /// buffers still open.
    ///
    /// # Panics
    /// Panics if no group is open.
    pub(in crate::editor) fn close_selection_group(&mut self) {
        let group = self
            .selection_group
            .take()
            .expect("a Steel session's group is open");
        for (pid, bid) in group {
            if let Some(pane_state) = self.panes.entries.pair_mut(pid, bid) {
                pane_state.record_deferred_selections(self.buffers.get(bid).text());
            }
        }
    }
}

#[cfg(test)]
mod tests;
