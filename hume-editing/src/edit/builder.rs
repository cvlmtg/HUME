//! Building an edit against the text it changes, applied in position order.
//!
//! Every operation names a position or range of the old text; none depends on
//! what was recorded before it, except that operations at one position take
//! effect in the order they were recorded. [`edit`] sorts the operations by
//! position, merges overlapping deletions into their union, and drives
//! [`ChangeSetBuilder`] front to back. A position deleted by another
//! operation resolves to the deletion point, the same rule
//! [`crate::changeset::PosMapCursor`] applies to any position.
//!
//! The text keeps ending with its structural `\n`: no deletion reaches it,
//! and text lands after it only when that text ends with a `\n` of its own.
//!
//! Positions of the text being produced come back as [`NewPos`] and [`Mark`]
//! handles. Only a [`Landing`] consumes one, and the `'id` brand ties each
//! handle to the plan that made it, so a position of one edit can never be
//! resolved against another.

use std::marker::PhantomData;

use hume_rope::cluster::ClusterRange;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use ropey::RopeSlice;

use super::Edited;
use crate::changeset::{Assoc, ChangeSet, ChangeSetBuilder};
use crate::edit::TextChange;
use crate::selection::{
    Facing, Resolver, Selection, SelectionSet, SelectionView, UnboundSelection,
};
use crate::state::EditState;
use crate::text::{BufferText, normalize_line_endings};

/// Ties a handle to the one plan that made it: the lifetime is invariant, and
/// [`edit`] instantiates it afresh for each call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Brand<'id>(PhantomData<fn(&'id ()) -> &'id ()>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    /// Where item `index` starts in the new text, or where it ends.
    Item { index: usize, after: bool },
    /// A position of the old text, carried through the finished edit.
    Old(CharOffset, Assoc),
}

/// A position in the text an edit is producing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewPos<'id> {
    handle: Handle,
    brand: Brand<'id>,
}

/// The new text between two plan positions: what an insertion produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark<'id> {
    start: NewPos<'id>,
    end: NewPos<'id>,
}

impl<'id> Mark<'id> {
    pub fn start(self) -> NewPos<'id> {
        self.start
    }

    pub fn end(self) -> NewPos<'id> {
        self.end
    }
}

#[derive(Debug)]
struct Item {
    key: CharOffset,
    kind: ItemKind,
}

#[derive(Debug)]
enum ItemKind {
    /// Removes `key..end`. `lines` is set for a whole-line removal.
    Delete {
        end: CharOffset,
        lines: Option<(ContentLine, ContentLine)>,
    },
    Insert(String),
    /// Marks a place in the new text without changing it.
    Anchor,
}

/// The operations of one edit, recorded against the old text.
pub struct EditBuilder<'a, 'id> {
    text: &'a BufferText,
    items: Vec<Item>,
    brand: Brand<'id>,
}

/// What [`EditBuilder::remove`] took out of the text: the text to put in a
/// register, and where the selection lands.
pub struct Removed<'a, 'id> {
    pub text: RopeSlice<'a>,
    pub cursor: Landing<'id>,
}

/// Records the edit `build` describes for `state`'s text and applies it.
pub fn edit<'a>(
    state: &'a EditState,
    build: impl for<'id> FnOnce(&mut EditBuilder<'a, 'id>) -> Landings<'id>,
) -> Edited {
    let mut plan = EditBuilder {
        text: state.text(),
        items: Vec::new(),
        brand: Brand(PhantomData),
    };
    let landings = build(&mut plan);
    plan.finish(landings)
}

impl<'a, 'id> EditBuilder<'a, 'id> {
    /// The text being edited.
    pub fn text(&self) -> &'a BufferText {
        self.text
    }

    fn push(&mut self, key: CharOffset, kind: ItemKind) -> usize {
        self.items.push(Item { key, kind });
        self.items.len() - 1
    }

    fn pos(&self, index: usize, after: bool) -> NewPos<'id> {
        NewPos {
            handle: Handle::Item { index, after },
            brand: self.brand,
        }
    }

    /// The deletion of `range`, which stops short of the structural `\n`.
    fn record_delete(
        &mut self,
        range: ExclusiveRange<CharOffset>,
        lines: Option<(ContentLine, ContentLine)>,
    ) -> usize {
        let last = self.text.last_char();
        let key = range.start.min(last);
        let end = range.end.min(last).max(key);
        self.push(key, ItemKind::Delete { end, lines })
    }

    /// Insert `text` at `at`; returns what it became. At the text's end,
    /// after the structural `\n`, only text ending with a `\n` goes there;
    /// any other lands before the structural `\n`.
    ///
    /// # Panics
    /// Panics if `at` is past the end of the text.
    pub fn insert(&mut self, at: impl Into<CharOffset>, text: &str) -> Mark<'id> {
        let at = at.into();
        let end = self.text.end();
        assert!(
            at <= end,
            "EditBuilder::insert: position {at:?} is past the text end {end:?}"
        );
        let text = normalize_line_endings(text);
        let key = if at < end {
            at
        } else if text.ends_with('\n') {
            end
        } else {
            self.text.last_char()
        };
        let index = self.push(key, ItemKind::Insert(text.into_owned()));
        Mark {
            start: self.pos(index, false),
            end: self.pos(index, true),
        }
    }

    /// Delete `range`; returns where it was.
    pub fn delete(&mut self, range: impl Into<ExclusiveRange<CharOffset>>) -> NewPos<'id> {
        let index = self.record_delete(range.into(), None);
        self.pos(index, false)
    }

    /// Replace `range` with `text`; returns what the replacement became.
    ///
    /// A range reaching the structural `\n` keeps it. When `text` ends in a
    /// `\n`, that one stands for the kept structural `\n`, so the result has
    /// no extra line and the returned mark covers the structural `\n`.
    pub fn replace(
        &mut self,
        range: impl Into<ExclusiveRange<CharOffset>>,
        text: &str,
    ) -> Mark<'id> {
        let range = range.into();
        let text = normalize_line_endings(text);
        let reaches_break = range.end > self.text.last_char();
        let deleted = self.record_delete(range, None);
        let key = self.items[deleted].key;
        let start = self.pos(deleted, false);
        match text.strip_suffix('\n') {
            Some(body) if reaches_break => {
                self.push(key, ItemKind::Insert(body.to_owned()));
                let end = NewPos {
                    handle: Handle::Old(self.text.end(), Assoc::Before),
                    brand: self.brand,
                };
                Mark { start, end }
            }
            _ => {
                let inserted = self.push(key, ItemKind::Insert(text.into_owned()));
                Mark {
                    start,
                    end: self.pos(inserted, true),
                }
            }
        }
    }

    /// Keep `range` unchanged; returns what it became.
    pub fn keep(&mut self, range: ClusterRange) -> Mark<'id> {
        let start = self.push(range.start().offset(), ItemKind::Anchor);
        let end = self.push(range.end().offset(), ItemKind::Anchor);
        Mark {
            start: self.pos(start, false),
            end: self.pos(end, false),
        }
    }

    /// Take `sel` out of the text as `d` does. A selection of whole lines
    /// removes them; when they run to the last line, the break before the
    /// first of them goes too, so no empty line is left behind, and the
    /// lines go in the register. Any other selection removes what it covers
    /// except the structural `\n`, and puts what it removed in the register.
    /// `None` when nothing can be removed: a cursor on the structural `\n` of
    /// a line with text, or of the only line.
    pub fn remove(&mut self, sel: SelectionView<'a>) -> Option<Removed<'a, 'id>> {
        let (removed, text) = sel.removal()?;
        if sel.is_linewise() {
            let lines = sel.lines();
            let index = self.record_delete(removed, Some((lines.start, lines.end)));
            return Some(Removed {
                text,
                cursor: Landing::line_start_of(self.pos(index, false)),
            });
        }
        let index = self.record_delete(removed, None);
        Some(Removed {
            text,
            cursor: Landing::cursor(self.pos(index, false)),
        })
    }

    /// Take what `sel` covers out of the text as `c` does: everything but
    /// the `\n` it ends on. `None` when that `\n` is all it covers.
    pub fn remove_content(&mut self, sel: SelectionView<'a>) -> Option<Removed<'a, 'id>> {
        let content = sel.content()?;
        let index = self.record_delete(content.chars(), None);
        Some(Removed {
            text: self.text.slice(content.chars()),
            cursor: Landing::cursor(self.pos(index, false)),
        })
    }

    /// Whole-line removals that run to the last line take the break before
    /// the first of them, so the line left last has none of its own trailing
    /// break doubled.
    fn extend_removed_run(&mut self) {
        let last_line = self.text.last_content_line();
        let mut runs: Vec<(ContentLine, ContentLine, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item.kind {
                ItemKind::Delete {
                    lines: Some((first, last)),
                    ..
                } => Some((first, last, index)),
                _ => None,
            })
            .collect();
        runs.sort_by_key(|&(first, _, _)| first);
        let Some(mut at) = runs.iter().rposition(|&(_, last, _)| last >= last_line) else {
            return;
        };
        while at > 0 && runs[at - 1].1.advance(1) >= runs[at].0 {
            at -= 1;
        }
        let (first, _, owner) = runs[at];
        if first.index() > 0 {
            let before = crate::lines::line_break(self.text, first.retreat_saturating(1));
            self.items[owner].key = before.offset();
        }
    }

    /// The `ChangeSet` of the recorded operations, and where each item
    /// starts and ends in the new text.
    fn drive(&self) -> (ChangeSet, Vec<(CharOffset, CharOffset)>) {
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by_key(|&index| (self.items[index].key, index));
        let mut changes = ChangeSetBuilder::new(self.text.end());
        let mut placed = vec![(CharOffset::default(), CharOffset::default()); self.items.len()];
        for index in order {
            let item = &self.items[index];
            if item.key > changes.old_pos() {
                changes.retain_to(item.key);
            }
            let before = changes.new_pos();
            match &item.kind {
                ItemKind::Delete { end, .. } => {
                    if *end > changes.old_pos() {
                        changes.delete_to(*end);
                    }
                }
                ItemKind::Insert(text) => {
                    changes.insert(text);
                }
                ItemKind::Anchor => {}
            }
            placed[index] = (before, changes.new_pos());
        }
        (changes.finish(), placed)
    }

    fn finish(mut self, results: Landings<'id>) -> Edited {
        self.extend_removed_run();
        let (changes, placed) = self.drive();
        let text = changes
            .apply(self.text)
            .expect("an edit plan produces a changeset for its own text");
        let selections = {
            let change = TextChange::new(self.text, &text, &changes);
            let mut resolver = Resolver::new(Some(&change), &text);
            let selections = results
                .items
                .into_iter()
                .map(|landing| landing.resolve(&placed, &mut resolver))
                .collect();
            SelectionSet::from_parts(selections, results.primary, text.version())
        };
        Edited {
            state: EditState::from_set(text, selections),
            base: self.text.version(),
            changes,
        }
    }
}

/// One selection an edit leaves behind, described by positions its plan
/// handed out or by a selection of the old text to carry through the edit.
pub struct Landing<'id>(Kind<'id>);

enum Kind<'id> {
    Cursor(NewPos<'id>),
    After(NewPos<'id>),
    CursorEndingAt(NewPos<'id>),
    Covering(Mark<'id>, Facing),
    LineStartOf(NewPos<'id>),
    Old(UnboundSelection),
}

impl<'id> Landing<'id> {
    /// A cursor on the cluster holding `at`.
    pub fn cursor(at: NewPos<'id>) -> Self {
        Self(Kind::Cursor(at))
    }

    /// A cursor on the cluster after inserted text ending at `end`; the
    /// structural `\n` when the text ends there.
    pub fn after(end: NewPos<'id>) -> Self {
        Self(Kind::After(end))
    }

    /// A cursor on the cluster that ends at `end`: the last cluster before it.
    pub fn cursor_ending_at(end: NewPos<'id>) -> Self {
        Self(Kind::CursorEndingAt(end))
    }

    /// The clusters `mark` spans, facing `facing`; a cursor at its start when
    /// it spans nothing.
    pub fn covering(mark: Mark<'id>, facing: Facing) -> Self {
        Self(Kind::Covering(mark, facing))
    }

    /// `sel` carried through the edit, each end past text inserted at it.
    pub fn kept(sel: Selection) -> Self {
        UnboundSelection::kept(sel).into()
    }

    /// `sel` carried through the edit, each end on `assoc`'s side of text
    /// inserted at it.
    pub fn kept_with(sel: Selection, assoc: Assoc) -> Self {
        UnboundSelection::kept_with(sel, assoc).into()
    }

    /// A cursor on the first cluster of the line holding `at`.
    pub fn line_start_of(at: NewPos<'id>) -> Self {
        Self(Kind::LineStartOf(at))
    }

    fn resolve(
        self,
        placed: &[(CharOffset, CharOffset)],
        resolver: &mut Resolver<'_>,
    ) -> Selection {
        let text = resolver.text();
        let mut place = |pos: NewPos<'id>| match pos.handle {
            Handle::Item { index, after } => {
                let (start, end) = placed[index];
                if after { end } else { start }
            }
            Handle::Old(pos, assoc) => resolver.map_old(pos, assoc),
        };
        match self.0 {
            Kind::Cursor(at) => {
                let at = place(at);
                debug_assert!(at < text.end(), "a cursor landed past the last cluster");
                Selection::cursor(text.snap(at))
            }
            Kind::After(end) => Selection::cursor(text.snap(place(end))),
            Kind::CursorEndingAt(end) => {
                let end = place(end);
                assert!(
                    end > CharOffset::default(),
                    "no cluster ends at the text start"
                );
                Selection::cursor(text.snap(end.retreat(1)))
            }
            Kind::Covering(mark, facing) => {
                let start = place(mark.start);
                let end = place(mark.end);
                assert!(start <= end, "a mark ends before it starts");
                match text.covering(ExclusiveRange::new(start, end)) {
                    Some(range) => Selection::covering(range, facing),
                    None => Selection::cursor(text.snap(start)),
                }
            }
            Kind::LineStartOf(at) => {
                let line = text.char_to_line(place(at));
                Selection::cursor(text.snap(text.line_to_char(line.into())))
            }
            Kind::Old(unbound) => resolver.selection(unbound),
        }
    }
}

impl From<UnboundSelection> for Landing<'_> {
    fn from(unbound: UnboundSelection) -> Self {
        Self(Kind::Old(unbound))
    }
}

/// An edit's selections before its text exists, with the primary's index.
pub struct Landings<'id> {
    items: Vec<Landing<'id>>,
    primary: usize,
}

impl<'id> Landings<'id> {
    /// # Panics
    /// Panics if `items` is empty or `primary` is out of range.
    pub fn new(items: Vec<Landing<'id>>, primary: usize) -> Self {
        assert!(!items.is_empty(), "an edit leaves at least one selection");
        assert!(primary < items.len(), "primary index out of bounds");
        Self { items, primary }
    }
}

#[cfg(test)]
mod tests;
