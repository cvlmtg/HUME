//! A [`History`] as plain data, saved with its buffer's file and restored the
//! next time the file is opened.
//!
//! A snapshot holds no text versions, which are unique only within one run.
//! Selections are char offsets, and each revision stores the one changeset
//! the restoring walk needs: its inverse when it lies on the path from the
//! current revision to the root, its forward change otherwise.
//! [`History::restore`] walks the tree outward from the current revision's
//! text, applying each stored change, deriving the opposite one with
//! [`ChangeSet::invert`] and rebuilding each selection on the text it belongs
//! to, so every check it makes is against a text it holds.

use std::borrow::Cow;
use std::time::{Duration, UNIX_EPOCH};

use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use super::{Children, History, Link, Owner, Revision, RevisionId};
use crate::changeset::{ChangeSet, Operation};
use crate::selection::SelectionsSnapshot;
use crate::text::BufferText;
use crate::transaction::Transaction;

/// A whole undo tree. A snapshot taken from a [`History`] borrows its
/// changes; a decoded one owns them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySnapshot<'a> {
    /// The revision whose text the buffer holds.
    current: usize,
    revisions: Vec<RevisionSnapshot<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RevisionSnapshot<'a> {
    id: usize,
    /// `None` only for the root.
    link: Option<LinkSnapshot<'a>>,
    /// When the revision was made, as time since the Unix epoch.
    since_epoch: Duration,
    /// Whether the pane that made the newest pane-made revision made this
    /// one.
    owned: bool,
    /// The child redo continues along, `None` for a leaf.
    redo: Option<usize>,
}

/// A revision's edge to its parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LinkSnapshot<'a> {
    parent: usize,
    /// The revision's text to the parent's for the current revision and its
    /// ancestors, the parent's text to the revision's for any other.
    change: Cow<'a, [Operation]>,
    /// The selections the edit was made with, on the parent's text.
    before: SelectionsSnapshot,
    /// The selections the edit left, on the revision's own text.
    after: SelectionsSnapshot,
}

/// Why a snapshot cannot be restored. The number names the revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    /// Two revisions share the id.
    Duplicate(usize),
    /// The root, id 0, is missing or has a parent.
    Root,
    /// The revision has no parent, or its parent is missing or not older.
    Parent(usize),
    /// The current revision is not in the tree.
    Current,
    /// The revision's redo target is not one of its children, it has
    /// children but no redo target, or it lies on the current revision's
    /// path and its redo leads elsewhere.
    Redo(usize),
    /// The revision's time is not a representable time.
    Timestamp(usize),
    /// The revision's change does not apply to its text, or changes nothing.
    Change(usize),
    /// A selection of the revision is out of range or splits a cluster.
    Selections(usize),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(id) => write!(f, "revision {id} appears twice"),
            Self::Root => write!(f, "the root revision is missing"),
            Self::Parent(id) => write!(f, "revision {id} has no valid parent"),
            Self::Current => write!(f, "the current revision is missing"),
            Self::Redo(id) => write!(f, "revision {id} has an invalid redo target"),
            Self::Timestamp(id) => write!(f, "revision {id} has an invalid time"),
            Self::Change(id) => write!(f, "revision {id} does not match the text"),
            Self::Selections(id) => write!(f, "revision {id} has invalid selections"),
        }
    }
}

impl std::error::Error for RestoreError {}

impl History {
    /// This history as plain data. Only the revisions made by the pane that
    /// made the newest pane-made revision keep an owner, as `owned`.
    pub fn snapshot(&self) -> HistorySnapshot<'_> {
        let (on_path, _) = self.path(Self::ROOT).expect("the root is in the tree");
        let on_path: FxHashSet<RevisionId> = on_path.into_iter().collect();
        let kept = self
            .revisions
            .iter()
            .map(|(&id, revision)| (id.0, self.owner_of(revision)))
            .filter(|&(_, owner)| owner != Owner::Nobody)
            .max_by_key(|&(id, _)| id)
            .map(|(_, owner)| owner);
        let mut revisions: Vec<_> = self
            .revisions
            .iter()
            .map(|(&id, revision)| RevisionSnapshot {
                id: id.0,
                link: revision.link.as_ref().map(|link| LinkSnapshot {
                    parent: link.parent.0,
                    change: Cow::Borrowed(if on_path.contains(&id) {
                        link.inverse.changes().ops()
                    } else {
                        link.forward.changes().ops()
                    }),
                    before: SelectionsSnapshot::of(link.inverse.selection()),
                    after: SelectionsSnapshot::of(link.forward.selection()),
                }),
                // A clock set before 1970 saves the epoch.
                since_epoch: revision
                    .timestamp
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default(),
                owned: Some(self.owner_of(revision)) == kept,
                redo: revision.children.redo().map(|child| child.0),
            })
            .collect();
        revisions.sort_by_key(|revision| revision.id);
        HistorySnapshot {
            current: self.current.0,
            revisions,
        }
    }

    /// Replace this history with `snapshot`'s tree, `text` being its current
    /// revision's text, keeping `undo_levels` and the pane that claimed it
    /// ([`Self::claim_restored`]), which owns the owned revisions. On an
    /// error this history is unchanged.
    pub fn restore(
        &mut self,
        snapshot: HistorySnapshot<'_>,
        text: &BufferText,
    ) -> Result<(), RestoreError> {
        let tree = Tree::index(snapshot)?;
        let next_id = tree.revisions.keys().max().map_or(1, |&max| max + 1);
        let current = RevisionId(tree.current);
        let revisions = tree.rebuild(text)?;
        *self = Self {
            revisions,
            current,
            next_id,
            undo_levels: self.undo_levels,
            change_seq: self.change_seq + 1,
            claimed_by: self.claimed_by,
        };
        Ok(())
    }
}

/// A snapshot's revisions by id, checked to form a tree with sound redo
/// targets and times.
struct Tree<'a> {
    current: usize,
    revisions: FxHashMap<usize, RevisionSnapshot<'a>>,
    /// Each revision's children, oldest first.
    children: FxHashMap<usize, Vec<usize>>,
    /// The current revision and its ancestors, the root left out.
    on_path: FxHashSet<usize>,
}

impl<'a> Tree<'a> {
    fn index(snapshot: HistorySnapshot<'a>) -> Result<Self, RestoreError> {
        let mut revisions = FxHashMap::default();
        for revision in snapshot.revisions {
            let id = revision.id;
            if revisions.insert(id, revision).is_some() {
                return Err(RestoreError::Duplicate(id));
            }
        }
        if revisions.get(&0).is_none_or(|root| root.link.is_some()) {
            return Err(RestoreError::Root);
        }
        let mut children: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
        for (&id, revision) in &revisions {
            let parent = match &revision.link {
                Some(link) => link.parent,
                None if id == 0 => continue,
                None => return Err(RestoreError::Parent(id)),
            };
            if parent >= id || !revisions.contains_key(&parent) {
                return Err(RestoreError::Parent(id));
            }
            children.entry(parent).or_default().push(id);
        }
        children.values_mut().for_each(|ids| ids.sort_unstable());

        let current = snapshot.current;
        if !revisions.contains_key(&current) {
            return Err(RestoreError::Current);
        }
        let parent_of = |id: usize| revisions[&id].link.as_ref().map(|link| link.parent);
        let on_path: FxHashSet<usize> = std::iter::successors(Some(current), |&id| parent_of(id))
            .filter(|&id| id != 0)
            .collect();

        for (&id, revision) in &revisions {
            let kids = children.get(&id).map_or(&[][..], Vec::as_slice);
            let redo_sound = match revision.redo {
                None => kids.is_empty(),
                Some(redo) => {
                    kids.contains(&redo)
                        && kids
                            .iter()
                            .all(|kid| !on_path.contains(kid) || *kid == redo)
                }
            };
            if !redo_sound {
                return Err(RestoreError::Redo(id));
            }
            if UNIX_EPOCH.checked_add(revision.since_epoch).is_none() {
                return Err(RestoreError::Timestamp(id));
            }
        }
        Ok(Self {
            current,
            revisions,
            children,
            on_path,
        })
    }

    /// Every revision with its link rebuilt by walking out from `text`, the
    /// current revision's text: up through its ancestors with their
    /// inverses, down every other branch with forward changes.
    fn rebuild(
        mut self,
        text: &BufferText,
    ) -> Result<FxHashMap<RevisionId, Revision>, RestoreError> {
        let mut links = FxHashMap::default();
        let mut stack = vec![(self.current, text.clone())];
        while let Some((id, here)) = stack.pop() {
            if self.on_path.contains(&id) {
                let (parent_text, link) = self.link(id, &here)?;
                stack.push((link.parent.0, parent_text));
                links.insert(id, link);
            }
            let off_path: Vec<usize> = (self.children.get(&id).into_iter().flatten())
                .filter(|kid| !self.on_path.contains(kid))
                .copied()
                .collect();
            for kid in off_path {
                let (kid_text, link) = self.link(kid, &here)?;
                links.insert(kid, link);
                stack.push((kid, kid_text));
            }
        }

        let mut revisions = FxHashMap::default();
        for (id, snapshot) in self.revisions {
            let mut children = Children::default();
            for &kid in self.children.get(&id).into_iter().flatten() {
                children.push(RevisionId(kid));
            }
            if let Some(redo) = snapshot.redo {
                children.set_redo(RevisionId(redo));
            }
            let revision = Revision {
                link: links.remove(&id),
                owner: if snapshot.owned {
                    Owner::Restored
                } else {
                    Owner::Nobody
                },
                children,
                timestamp: UNIX_EPOCH + snapshot.since_epoch,
            };
            revisions.insert(RevisionId(id), revision);
        }
        Ok(revisions)
    }

    /// `id`'s link, rebuilt from its stored change and `here`, the text that
    /// change applies to, with the text it makes. The change must fit
    /// `here`, change something and keep the final line break.
    fn link(&mut self, id: usize, here: &BufferText) -> Result<(BufferText, Link), RestoreError> {
        let stored = self
            .revisions
            .get_mut(&id)
            .and_then(|revision| revision.link.take())
            .expect("the walk crosses each edge once");
        let changes = ChangeSet::from_ops(stored.change.into_owned(), here.len_chars())
            .filter(|changes| !changes.is_identity())
            .ok_or(RestoreError::Change(id))?;
        let there = changes.apply(here).map_err(|_| RestoreError::Change(id))?;
        let opposite = changes.invert(here);
        let up = self.on_path.contains(&id);
        let (parent_text, own_text) = if up { (&there, here) } else { (here, &there) };
        let before = stored
            .before
            .on(parent_text)
            .ok_or(RestoreError::Selections(id))?;
        let after = stored
            .after
            .on(own_text)
            .ok_or(RestoreError::Selections(id))?;
        let (inverse, forward) = if up {
            (changes, opposite)
        } else {
            (opposite, changes)
        };
        let link = Link {
            parent: RevisionId(stored.parent),
            inverse: Transaction::new(inverse, before),
            forward: Transaction::new(forward, after),
        };
        Ok((there, link))
    }
}

#[cfg(test)]
mod tests;
