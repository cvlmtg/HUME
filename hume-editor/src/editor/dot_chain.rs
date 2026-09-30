//! The edits a dot capture records, and the cursor position they are
//! measured from.

use hume_editing::changeset::ChangeSet;
use hume_editing::edit::TextChange;
use hume_editing::text::{BufferText, TextVersion};
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;

use super::replay::CursorReplacement;

/// A run of edits that compose into one, plus the primary head in the text
/// before the first of them.
///
/// `origin` is a position in the first edit's old text, not in the
/// buffer's current text, which is why it is not `Tracked`: it is read
/// against the composed edit at `finish`. The chain holds only while every
/// edit starts from the text the previous one produced. `version` is that
/// text's version, and an edit starting anywhere else (one the chain never
/// saw landed in between) breaks the chain for good.
#[derive(Debug)]
pub(in crate::editor) struct DotChain {
    links: Option<Links>,
}

#[derive(Debug)]
struct Links {
    origin: ClusterStart,
    edits: Vec<ChangeSet>,
    version: TextVersion,
}

/// A chain an unrecorded edit broke: its edits no longer compose.
#[derive(Debug)]
pub(in crate::editor) struct ChainBroken;

impl DotChain {
    /// An empty chain measured from `head` in `text`.
    pub(in crate::editor) fn new(head: ClusterStart, text: &BufferText) -> Self {
        Self {
            links: Some(Links {
                origin: head,
                edits: Vec::new(),
                version: text.version(),
            }),
        }
    }

    /// Record `change`. Breaks the chain unless `change` starts from the
    /// text the chain last saw.
    pub(in crate::editor) fn push(&mut self, change: &TextChange<'_>) {
        self.links = self.links.take().and_then(|mut links| {
            (links.version == change.before().version()).then(|| {
                links.edits.push(change.changes().clone());
                links.version = change.after().version();
                links
            })
        });
    }

    /// The chain to continue recording on `text`, whose primary head is
    /// `head`. An empty chain starts over from `head`, whatever happened to
    /// the text since it was armed. A non-empty one continues only if
    /// `text` is the one its last edit produced; `None` otherwise, or if it
    /// is broken.
    pub(in crate::editor) fn rearm(self, head: ClusterStart, text: &BufferText) -> Option<Self> {
        let links = self.links?;
        if links.edits.is_empty() {
            Some(Self::new(head, text))
        } else {
            (links.version == text.version()).then_some(Self { links: Some(links) })
        }
    }

    /// The composed edit's replacement at `origin`, or `None` when it did
    /// not touch it.
    pub(in crate::editor) fn finish(self) -> Result<Option<CursorReplacement>, ChainBroken> {
        let links = self.links.ok_or(ChainBroken)?;
        Ok(ChangeSet::compose_all(links.edits)
            .and_then(|delta| cursor_replacement_at(&delta, links.origin.offset())))
    }
}

/// Extracts the edited region of `delta` that contains `head` (a position
/// in `delta`'s *old* document), as a cursor-relative replacement, or `None`
/// if `delta` is identity, or every edited region lands away from `head`.
/// `delta` can hold more than one region (a completion's own
/// `additionalTextEdits`, or a second cursor's own edit under a
/// multi-cursor accept), and any region other than the one at `head` is
/// simply skipped: it's document-absolute, or belongs to a different
/// cursor, the same reasoning [`CursorReplacement`]'s own doc gives for
/// excluding `additionalTextEdits` from the replacement it records.
///
/// [`ChangeSet::edited_regions`] already pairs a region's `Delete`/`Insert`
/// ops in either order, so this only has to pick the one at `head`.
fn cursor_replacement_at(delta: &ChangeSet, head: CharOffset) -> Option<CursorReplacement> {
    delta
        .edited_regions()
        .into_iter()
        .find(|r| head >= r.old.start && head <= r.old.end)
        .map(|r| CursorReplacement {
            back: head.chars_since(r.old.start),
            forward: r.old.end.chars_since(head),
            text: r.inserted.into_owned(),
        })
}

#[cfg(test)]
mod tests;
