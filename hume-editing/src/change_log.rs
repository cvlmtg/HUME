//! What changed in a text since an earlier generation of it, as line hunks.

use std::borrow::Cow;
use std::collections::VecDeque;

use crate::changeset::{ChangeSet, Operation};
use crate::edit::TextChange;
use crate::hunk::{ChangeHunk, change_hunks_within};
use crate::text::BufferText;

/// Steps the log keeps. A keystroke's step holds a few ops, so the full log
/// of a typing session is around a hundred kilobytes.
const MAX_ENTRIES: usize = 1024;

/// Deleted chars the kept steps may hold between them, so a few whole-text
/// rewrites of a large buffer do not keep megabytes alive.
const MAX_HELD_CHARS: usize = 1 << 20;

/// Chars the lines of one [`ChangeLog::since`] answer may span, both sides
/// counted, so a large paste or reload reaches handlers as unknown instead
/// of as copies of every line it touched.
const MAX_REPORTED_CHARS: usize = 1 << 20;

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
    /// step starts at `generation`, a replacement lies between the two, or
    /// the hunks' lines would span more than `MAX_REPORTED_CHARS`.
    pub fn since(&self, generation: u64, live: &BufferText) -> Option<Vec<ChangeHunk>> {
        if generation == live.generation() {
            return Some(Vec::new());
        }
        let first = self.steps.iter().rposition(|s| s.from == generation)?;
        let undos = self
            .steps
            .range(first..)
            .rev()
            .map(|s| s.undo.as_ref())
            .collect::<Option<Vec<_>>>()?;
        let net = match undos[..] {
            [only] => Cow::Borrowed(only),
            _ => Cow::Owned(
                ChangeSet::compose_all(undos.into_iter().cloned())
                    .expect("a found step gives one undo at least"),
            ),
        };
        change_hunks_within(&net, live, MAX_REPORTED_CHARS)
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
