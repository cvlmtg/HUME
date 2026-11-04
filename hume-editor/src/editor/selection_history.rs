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

use std::collections::VecDeque;

use hume_editing::edit::TextChange;
use hume_editing::history::SelectionPoint;
use hume_editing::selection::SelectionSet;
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
}

impl PaneSelections {
    /// `current`, with an empty live list.
    pub(in crate::editor) fn new(current: SelectionSet) -> Self {
        Self {
            current,
            live: VecDeque::new(),
            generation: 0,
            walked: None,
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
    }

    /// Forget the live list and the walk: this pane edited the buffer.
    pub(in crate::editor) fn edited(&mut self) {
        self.live.clear();
        self.walked = None;
    }

    /// A copy for a pane split off this one, which walks from the present.
    pub(in crate::editor) fn split(&self) -> Self {
        Self {
            walked: None,
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

    /// Take `count` steps (at least one) in direction `walk`, as far as the
    /// steps go, and make the selections the one it lands on. `saved` is
    /// the undo tree's part, `None` while the walk may not read it. A step
    /// is a set that is not a single cursor and differs from the selections
    /// it leaves, ignoring sticky columns. Back from selections that moved
    /// off where the last walk landed lands there first.
    ///
    /// # Errors
    /// [`TreeRefused`] when a step back needed the undo tree and `saved` is
    /// `None`. The steps taken before it stay.
    pub(in crate::editor) fn walk(
        &mut self,
        walk: Walk,
        count: usize,
        saved: Option<&SavedSelections<'_>>,
    ) -> Result<(), TreeRefused> {
        let mut steps = Steps::new(&self.live, saved);
        let mut at = self.walked.map(|point| steps.index_of(point));
        let mut retry_landed = matches!(walk, Walk::Back)
            && at.is_some_and(|index| {
                steps.get(index).is_ok_and(|landed| {
                    landed.is_some_and(|l| !l.eq_ignoring_sticky(&self.current))
                })
            });
        for _ in 0..count {
            loop {
                let next = match walk {
                    Walk::Back if retry_landed => at.expect("a landed walk has a point"),
                    Walk::Back => at.map_or(0, |index| index + 1),
                    Walk::Forward => match at {
                        Some(index) if index > 0 => index - 1,
                        _ => return Ok(()),
                    },
                };
                retry_landed = false;
                at = Some(next);
                let Some(sels) = steps.get(next)? else {
                    return Ok(());
                };
                if !sels.is_single_cursor() && !sels.eq_ignoring_sticky(&self.current) {
                    self.current = sels;
                    self.walked = Some(steps.point(next));
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

    /// The step at `index`, `None` past the oldest.
    fn get(&mut self, index: usize) -> Result<Option<SelectionSet>, TreeRefused> {
        let live = self.live.len();
        if let Some(live_index) = live.checked_sub(index + 1) {
            return Ok(Some(self.live[live_index].clone()));
        }
        let point = self.tree()?.get(index - live).copied();
        Ok(point.and_then(|point| {
            self.saved
                .expect("the tree's points came from it")
                .at(point)
        }))
    }

    fn point(&self, index: usize) -> Point {
        match self.live.len().checked_sub(index + 1) {
            Some(live_index) => Point::Live(live_index),
            None => Point::Tree(
                self.tree.as_ref().expect("a tree step was read")[index - self.live.len()],
            ),
        }
    }

    /// The index of `point`. A tree point whose revision is gone (trimmed by
    /// `undo-levels`) resumes from the newest tree step.
    fn index_of(&mut self, point: Point) -> usize {
        match point {
            Point::Live(live_index) => self.live.len() - 1 - live_index,
            Point::Tree(point) => {
                let offset = self
                    .tree()
                    .ok()
                    .and_then(|tree| tree.iter().position(|&p| p == point))
                    .unwrap_or(0);
                self.live.len() + offset
            }
        }
    }
}

/// A change to a pane's selections that the live list will record: where it
/// goes, and the selections it starts from.
pub(in crate::editor) struct PendingRecord {
    placement: Placement,
    before: Tracked<SelectionSet>,
}

impl PendingRecord {
    /// Store `pane`'s selections at the placement if the change moved them,
    /// and return whether it did.
    pub(in crate::editor) fn finish(
        self,
        state: &mut EditorState,
        view: &EngineView,
        pane: CommandPane,
    ) -> bool {
        let text = state.buffers.get(pane.bid(view)).text();
        let pane_state = pane.state_mut(&mut state.panes.entries, view);
        let changed = self.before.get(text) != Some(pane_state.selections());
        if changed {
            pane_state.record_selections(self.placement);
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
            placement: state.selection_placement(record),
            before: Tracked::new(state.selections().clone(), text),
        }
    }
}

#[cfg(test)]
mod tests;
