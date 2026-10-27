//! What fires on which typed characters, within one scope.
//!
//! A [`TriggerTable`] maps each [`TriggerListener`] to the [`CharSet`] it
//! fires on.
//! The map is the whole state: the question the editor asks of a table, "which
//! listeners fire on this character?", is answered by reading it, and there
//! is no second structure to keep in step with it.
//!
//! The invariants are carried by the types rather than by callers:
//!
//! - a [`CharSet`] is non-empty, sorted and duplicate-free, because
//!   [`CharSet::new`] is the only way to build one and returns `None` for no
//!   characters;
//! - so a table never holds a listener with nothing to fire on: setting an
//!   empty set removes the listener;
//! - a table iterates in [`TriggerListener`] order, because it is a
//!   `BTreeMap`.
//!
//! This module knows nothing of buffers, languages or servers; the scopes
//! that own tables, and the union a keystroke reads, are in the parent
//! module.

use std::collections::BTreeMap;

use hume_core::TriggerListener;

/// A non-empty set of characters, sorted and without duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::editor) struct CharSet(Box<[char]>);

impl CharSet {
    /// The set of `chars`, or `None` when there are none.
    pub(in crate::editor) fn new(chars: impl IntoIterator<Item = char>) -> Option<Self> {
        let mut chars: Vec<char> = chars.into_iter().collect();
        chars.sort_unstable();
        chars.dedup();
        (!chars.is_empty()).then(|| Self(chars.into_boxed_slice()))
    }

    pub(in crate::editor) fn contains(&self, ch: char) -> bool {
        self.0.binary_search(&ch).is_ok()
    }

    #[cfg(test)]
    fn as_slice(&self) -> &[char] {
        &self.0
    }
}

/// The characters each listener fires on, within one scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::editor) struct TriggerTable {
    sets: BTreeMap<TriggerListener, CharSet>,
}

impl TriggerTable {
    /// Makes `chars` the characters `listener` fires on, replacing its
    /// previous set. No characters removes the listener.
    pub(in crate::editor) fn set(
        &mut self,
        listener: TriggerListener,
        chars: impl IntoIterator<Item = char>,
    ) {
        match CharSet::new(chars) {
            Some(set) => {
                self.sets.insert(listener, set);
            }
            None => {
                self.sets.remove(&listener);
            }
        }
    }

    /// The listeners that fire on `ch`, in [`TriggerListener`] order.
    pub(in crate::editor) fn fires_on(&self, ch: char) -> impl Iterator<Item = &TriggerListener> {
        self.sets
            .iter()
            .filter(move |(_, set)| set.contains(ch))
            .map(|(listener, _)| listener)
    }

    pub(in crate::editor) fn clear(&mut self) {
        self.sets.clear();
    }

    /// How many listeners the table holds.
    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.sets.len()
    }

    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

#[cfg(test)]
mod tests;
