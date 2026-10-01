//! Building an edit against the text it changes, applied in position order.
//!
//! Every operation names a position or range of the old text; none depends on
//! what was recorded before it, except that operations at one position take
//! effect in the order they were recorded. [`edit`] sorts the operations by
//! position, merges overlapping deletions into their union, and drives
//! [`ChangeSetBuilder`] front to back. A position strictly inside a deletion
//! resolves to the deletion point, the same rule
//! [`crate::changeset::PosMapCursor`] applies to any position.
//!
//! The text keeps ending with a `\n`, by
//! [`apply_keeping_final_break`](super::apply_keeping_final_break)'s rule.
//! Text lands after the structural `\n` only when that text ends with a `\n`
//! of its own, so under the ceiling only the deletions change.
//!
//! Positions of the text being produced come back as [`NewPos`] and [`Mark`]
//! handles. Only a [`Landing`] consumes one, and the `'id` brand ties each
//! handle to the builder that made it, so a position of one edit can never be
//! resolved against another.

use std::marker::PhantomData;

use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::column::CharCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::Edited;
use crate::changeset::{Assoc, ChangeSet, ChangeSetBuilder};
use crate::edit::TextChange;
use crate::selection::{Facing, Resolver, Selection, SelectionSet, SelectionView};
use crate::state::EditState;
use crate::text::{BufferText, LfText};

/// Ties a handle to the one builder that made it: the lifetime is invariant, and
/// [`edit`] instantiates it afresh for each call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Brand<'id>(PhantomData<fn(&'id ()) -> &'id ()>);

/// A position in the text an edit is producing: where item `index` starts
/// in the new text, or where it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewPos<'id> {
    index: usize,
    after: bool,
    brand: Brand<'id>,
}

/// The new text between two builder positions: what an insertion produced.
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
    /// Removes `key..end`.
    Delete {
        end: CharOffset,
    },
    Insert(LfText),
    /// Marks a place in the new text without changing it.
    Anchor,
}

/// The operations of one edit, recorded against the old text.
pub struct EditBuilder<'a, 'id> {
    text: &'a BufferText,
    items: Vec<Item>,
    brand: Brand<'id>,
}

/// Records the edit `build` describes for `state`'s text and applies it.
///
/// A position one edit hands out cannot land a selection of another:
///
/// ```compile_fail
/// use hume_editing::edit::{Landing, Landings, edit};
/// use hume_editing::state::EditState;
/// use hume_editing::text::BufferText;
/// use hume_rope::cluster::ClusterBound;
///
/// let state = EditState::at_text_start(BufferText::from("ab\n"));
/// edit(&state, |outer| {
///     let pos = outer.at(ClusterBound::TEXT_START);
///     edit(&state, |_inner| Landings::new(vec![Landing::cursor(pos)], 0));
///     Landings::new(vec![Landing::cursor(pos)], 0)
/// });
/// ```
pub fn edit<'a>(
    state: &'a EditState,
    build: impl for<'id> FnOnce(&mut EditBuilder<'a, 'id>) -> Landings<'id>,
) -> Edited {
    let mut builder = EditBuilder {
        text: state.text(),
        items: Vec::new(),
        brand: Brand(PhantomData),
    };
    let landings = build(&mut builder);
    builder.finish(landings)
}

impl<'a, 'id> EditBuilder<'a, 'id> {
    /// The text being edited.
    pub fn text(&self) -> &'a BufferText {
        self.text
    }

    /// # Panics
    /// Panics if the operation reaches past the text end. Debug builds also
    /// check that it starts and ends on cluster boundaries of this text.
    fn push(&mut self, key: CharOffset, kind: ItemKind) -> usize {
        let text_end = self.text.end();
        let end = match kind {
            ItemKind::Delete { end } => end,
            ItemKind::Insert(_) | ItemKind::Anchor => key,
        };
        assert!(
            end <= text_end,
            "EditBuilder: an operation reaches {end:?}, past the text end {text_end:?}"
        );
        debug_assert!(
            [key, end]
                .iter()
                .all(|&at| at == text_end || self.text.snap(at).offset() == at),
            "EditBuilder: an operation splits a cluster of this text"
        );
        self.items.push(Item { key, kind });
        self.items.len() - 1
    }

    fn pos(&self, index: usize, after: bool) -> NewPos<'id> {
        NewPos {
            index,
            after,
            brand: self.brand,
        }
    }

    /// Insert `text` at `at`; returns what it became. At the text's end,
    /// after the structural `\n`, only text ending with a `\n` goes there;
    /// any other lands before the structural `\n`.
    ///
    /// # Panics
    /// Panics if `at` is past the end of the text.
    pub fn insert(&mut self, at: impl Into<ClusterBound>, text: &str) -> Mark<'id> {
        let at = at.into().offset();
        let end = self.text.end();
        assert!(
            at <= end,
            "EditBuilder::insert: position {at:?} is past the text end {end:?}"
        );
        let text = LfText::new(text);
        let key = if at < end {
            at
        } else if text.as_str().ends_with('\n') {
            end
        } else {
            self.text.last_char()
        };
        let index = self.push(key, ItemKind::Insert(text));
        Mark {
            start: self.pos(index, false),
            end: self.pos(index, true),
        }
    }

    /// Delete `range`; returns where it was.
    ///
    /// The range is whole clusters, so a deletion cannot leave part of one
    /// behind. A char range enters through [`BufferText::within`] or
    /// [`BufferText::covering`]:
    ///
    /// ```
    /// use hume_editing::edit::{Landing, Landings, edit};
    /// use hume_editing::state::EditState;
    /// use hume_editing::text::BufferText;
    /// use hume_rope::offset::{CharOffset, ExclusiveRange};
    ///
    /// let state = EditState::at_text_start(BufferText::from("xe\u{301}y\n"));
    /// let edited = edit(&state, |b| {
    ///     let chars = ExclusiveRange::new(CharOffset::new(0), CharOffset::new(2));
    ///     let range = b.text().within(chars).expect("`x` lies inside the range");
    ///     Landings::new(vec![Landing::cursor(b.delete(range))], 0)
    /// });
    /// assert_eq!(edited.state().text().to_string(), "e\u{301}y\n");
    /// ```
    ///
    /// A char range as it stands does not compile:
    ///
    /// ```compile_fail,E0308
    /// use hume_editing::edit::{Landing, Landings, edit};
    /// use hume_editing::state::EditState;
    /// use hume_editing::text::BufferText;
    /// use hume_rope::offset::{CharOffset, ExclusiveRange};
    ///
    /// let state = EditState::at_text_start(BufferText::from("xe\u{301}y\n"));
    /// let edited = edit(&state, |b| {
    ///     let chars = ExclusiveRange::new(CharOffset::new(0), CharOffset::new(2));
    ///     Landings::new(vec![Landing::cursor(b.delete(chars))], 0)
    /// });
    /// ```
    pub fn delete(&mut self, range: ClusterRange) -> NewPos<'id> {
        let end = range.end().offset();
        let index = self.push(range.start().offset(), ItemKind::Delete { end });
        self.pos(index, false)
    }

    /// Where `pos` is in the new text, for an edit that changes nothing
    /// there: the place a deletion of nothing would name.
    pub fn at(&mut self, pos: impl Into<ClusterBound>) -> NewPos<'id> {
        let index = self.push(pos.into().offset(), ItemKind::Anchor);
        self.pos(index, false)
    }

    /// Replace `range` with `text`; returns what the replacement became.
    pub fn replace(&mut self, range: ClusterRange, text: &str) -> Mark<'id> {
        let start = self.delete(range);
        let inserted = self.push(range.start().offset(), ItemKind::Insert(LfText::new(text)));
        Mark {
            start,
            end: self.pos(inserted, true),
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

    /// The `ChangeSet` of the recorded operations, with every deletion
    /// stopping at `ceiling`, and where each item starts and ends in the new
    /// text.
    ///
    /// Inserted text moves into the changeset, so a call that another may
    /// follow clones it instead. Only a deletion reaching the text end can
    /// drop the final `\n` (an insertion there ends with its own), so only
    /// then is a second call possible.
    fn drive(&mut self, ceiling: Option<CharOffset>) -> (ChangeSet, Vec<(CharOffset, CharOffset)>) {
        let text_end = self.text.end();
        let rerun_possible = ceiling.is_none()
            && self
                .items
                .iter()
                .any(|item| matches!(item.kind, ItemKind::Delete { end } if end == text_end));
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by_key(|&index| (self.items[index].key, index));
        let mut changes = ChangeSetBuilder::new(self.text.end());
        let mut placed = vec![(CharOffset::default(), CharOffset::default()); self.items.len()];
        // The old start and new position of the deletion ending at `old_pos`.
        let mut deletion: Option<(CharOffset, CharOffset)> = None;
        for index in order {
            let item = &mut self.items[index];
            if item.key > changes.old_pos() {
                changes.retain_to(item.key);
                deletion = None;
            }
            let before = changes.new_pos();
            match &mut item.kind {
                ItemKind::Delete { end } => {
                    let end = ceiling.map_or(*end, |ceiling| (*end).min(ceiling));
                    if end > changes.old_pos() {
                        deletion.get_or_insert((item.key, changes.new_pos()));
                        changes.delete_to(end);
                    }
                }
                ItemKind::Insert(text) => {
                    let text = if rerun_possible {
                        text.clone()
                    } else {
                        std::mem::take(text)
                    };
                    changes.insert_normalized(text);
                }
                ItemKind::Anchor => {}
            }
            placed[index] = match (&item.kind, deletion) {
                (ItemKind::Anchor, Some((start, at)))
                    if item.key > start && item.key < changes.old_pos() =>
                {
                    (at, at)
                }
                _ => (before, changes.new_pos()),
            };
        }
        (changes.finish(), placed)
    }

    fn finish(mut self, results: Landings<'id>) -> Edited {
        let (text, changes, placed) =
            super::apply_keeping_final_break(self.text, |ceiling| self.drive(ceiling));
        let selections = {
            let change = TextChange::new(self.text, &text, &changes);
            let mut resolver = Resolver::new(&change);
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

/// One selection an edit leaves behind, described by positions its builder
/// handed out, by a selection of the old text to carry through the edit, or
/// by one moved to other lines. Resolving it lands every end on a cluster
/// start of the new text.
pub struct Landing<'id>(Kind<'id>);

enum Kind<'id> {
    Cursor(NewPos<'id>),
    CursorEndingAt(NewPos<'id>),
    Covering(Mark<'id>, Facing),
    LineStartOf(NewPos<'id>),
    Kept {
        sel: Selection,
        assoc: Assoc,
    },
    AtLines {
        sel: Selection,
        first: LineEnd,
        last: LineEnd,
    },
}

impl<'id> Landing<'id> {
    /// A cursor on the cluster holding `at`, or on the structural `\n` when
    /// `at` is the text end.
    pub fn cursor(at: NewPos<'id>) -> Self {
        Self(Kind::Cursor(at))
    }

    /// A cursor on the last cluster before `end` on `end`'s own line. When
    /// `end` starts a line, that line has no cluster before it and the cursor
    /// sits at `end`; at the text's end that is the structural `\n` before it.
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
        Self::kept_with(sel, Assoc::After)
    }

    /// `sel` carried through the edit, each end on `assoc`'s side of text
    /// inserted at it. One side for both ends keeps the selection's facing.
    pub fn kept_with(sel: Selection, assoc: Assoc) -> Self {
        Self(Kind::Kept {
            sel: sel.without_sticky(),
            assoc,
        })
    }

    /// `sel` with its first end moved to line `first` of the new text and its
    /// last end to line `last`. Each end keeps its column, clamped to its new
    /// line's last content cluster; an end on its line's `\n` stays on the
    /// new line's `\n`. For an edit that moves whole lines.
    ///
    /// # Panics
    /// Panics if `first` is after `last`.
    pub fn at_lines(sel: SelectionView<'_>, first: ContentLine, last: ContentLine) -> Self {
        assert!(
            first <= last,
            "at_lines: the first end's line is after the last's"
        );
        let text = sel.text();
        Self(Kind::AtLines {
            sel: sel.selection().without_sticky(),
            first: LineEnd::of(text, sel.start(), first),
            last: LineEnd::of(text, sel.last(), last),
        })
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
        let place = |pos: NewPos<'id>| {
            let (start, end) = placed[pos.index];
            if pos.after { end } else { start }
        };
        match self.0 {
            Kind::Cursor(at) => {
                let at = place(at);
                debug_assert!(at <= text.end(), "a cursor landed past the text end");
                Selection::cursor(text.snap(at))
            }
            Kind::CursorEndingAt(end) => {
                let end = place(end);
                let starts_line = end < text.end()
                    && crate::lines::line_start(text, text.char_to_line(end)).offset() == end;
                if starts_line {
                    Selection::cursor(text.snap(end))
                } else {
                    Selection::cursor(text.snap(end.retreat(1)))
                }
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
                Selection::cursor(crate::lines::line_start(text, line))
            }
            Kind::Kept { sel, assoc } => resolver.carry(sel, assoc),
            Kind::AtLines { sel, first, last } => {
                sel.with_ends(first.resolve(text), last.resolve(text))
            }
        }
    }
}

/// Where a selection end sits on a line of the new text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineEnd {
    line: ContentLine,
    place: LinePlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinePlace {
    Break,
    Col(CharCol),
}

impl LineEnd {
    /// `end`, a cluster of `text`, moved to `line` of the new text.
    fn of(text: &BufferText, end: ClusterStart, line: ContentLine) -> Self {
        let old = text.char_to_line(end.offset());
        let place = if end == crate::lines::line_break(text, old) {
            LinePlace::Break
        } else {
            LinePlace::Col(crate::lines::char_col_in_line(text, old, end.offset()))
        };
        Self { line, place }
    }

    /// The cluster of `text` this end names.
    ///
    /// # Panics
    /// Panics if the line is past the last line of `text`.
    fn resolve(self, text: &BufferText) -> ClusterStart {
        assert!(
            self.line <= text.last_content_line(),
            "an end moved to line {}, past the last line of the text",
            self.line.index()
        );
        match self.place {
            LinePlace::Break => crate::lines::line_break(text, self.line),
            LinePlace::Col(col) => crate::lines::place_char_column(text, self.line.into(), col),
        }
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
