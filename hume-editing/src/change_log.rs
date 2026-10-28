//! What changed in a text since an earlier generation of it, as line hunks.

use std::collections::VecDeque;

use crate::changeset::{ChangeSet, Operation};
use crate::edit::TextChange;
use crate::hunk::{ChangeHunk, change_hunks};
use crate::text::BufferText;

/// Steps the log keeps. A keystroke's step holds a few ops, so the full log
/// of a typing session is around a hundred kilobytes.
const MAX_ENTRIES: usize = 1024;

/// Deleted chars the kept steps may hold between them, so a few whole-text
/// rewrites of a large buffer do not keep megabytes alive.
const MAX_HELD_CHARS: usize = 1 << 20;

/// The recent steps of one text's lineage, each able to take the text it
/// produced back to the generation it started from. A step's start is the
/// previous step's result, so every generation the text has had since the
/// oldest kept step is a point [`ChangeLog::since`] can answer from.
#[derive(Debug, Default)]
pub struct ChangeLog {
    steps: VecDeque<Step>,
    held_chars: usize,
}

#[derive(Debug)]
struct Step {
    /// Generation of the text the step started from.
    from: u64,
    /// Maps the step's result back to its start, `None` for a replacement no
    /// change describes.
    undo: Option<ChangeSet>,
    /// Deleted chars `undo` holds to put back.
    chars: usize,
}

impl ChangeLog {
    /// Records `change` as the newest step. It must move to a later generation.
    pub fn record(&mut self, change: &TextChange<'_>) {
        debug_assert!(
            change.after().generation() > change.before().generation(),
            "ChangeLog::record: a step must move the text to a later generation"
        );
        let undo = change.changes().invert(change.before());
        let chars = undo
            .ops()
            .iter()
            .map(|op| match op {
                Operation::Insert(s) => s.chars().count(),
                Operation::Retain(_) | Operation::Delete(_) => 0,
            })
            .sum();
        self.push(Step {
            from: change.before().generation(),
            undo: Some(undo),
            chars,
        });
    }

    /// Records a wholesale replacement of `before`, which no change describes.
    pub fn record_replace(&mut self, before: &BufferText) {
        self.push(Step {
            from: before.generation(),
            undo: None,
            chars: 0,
        });
    }

    /// The hunks separating generation `generation` (old side) from `live`
    /// (new side), the text the newest step produced. `None` when no kept
    /// step starts at `generation` or a replacement lies between the two.
    pub fn since(&self, generation: u64, live: &BufferText) -> Option<Vec<ChangeHunk>> {
        if generation == live.generation() {
            return Some(Vec::new());
        }
        let first = self.steps.iter().position(|s| s.from == generation)?;
        let undos = self
            .steps
            .range(first..)
            .rev()
            .map(|s| s.undo.clone())
            .collect::<Option<Vec<_>>>()?;
        let net = ChangeSet::compose_all(undos).expect("a found step gives one undo at least");
        Some(change_hunks(&net, live))
    }

    fn push(&mut self, step: Step) {
        self.held_chars += step.chars;
        self.steps.push_back(step);
        while self.steps.len() > MAX_ENTRIES || self.held_chars > MAX_HELD_CHARS {
            let dropped = self
                .steps
                .pop_front()
                .expect("a log over a bound keeps a step");
            self.held_chars -= dropped.chars;
        }
    }
}

#[cfg(test)]
mod tests;
