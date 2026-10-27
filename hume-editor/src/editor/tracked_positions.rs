//! Positions a script asked the editor to remember, carried through every
//! text change like the rest of the positions the editor stores against a
//! buffer (see `position_stores`).

use std::rc::Rc;

use rustc_hash::FxHashMap;

use hume_core::BufferId;
use hume_editing::edit::TextChange;
use hume_editing::selection::{EditView, SelectionSet};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;
use hume_scripting::host::{HostToken, TrackedFate, TrackedHold};

use super::buffer::store::BufferStore;
use super::host_token;

/// The positions scripts are tracking, each a cursor in one buffer under a
/// token the script holds. A token nobody minted, or one already released,
/// answers absent, like every other widget token. A position a request's
/// hold released stays in the map until the next carry sweeps it, absent
/// to every query.
#[derive(Default)]
pub(in crate::editor) struct TrackedPositions {
    entries: FxHashMap<HostToken, Tracked>,
}

struct Tracked {
    bid: BufferId,
    cursor: SelectionSet,
    /// Shared with the holds of the `lsp-request!`s holding it.
    fate: Rc<TrackedFate>,
}

impl Tracked {
    fn live(&self) -> bool {
        !self.fate.is_released()
    }
}

impl TrackedPositions {
    /// Start tracking `at`, a cluster of `text`, the current text of `bid`.
    pub(in crate::editor) fn track(
        &mut self,
        bid: BufferId,
        text: &BufferText,
        at: ClusterStart,
    ) -> HostToken {
        let token = host_token::mint();
        let cursor = EditState::with_cursor(text.clone(), at).into_selections();
        self.entries.insert(
            token,
            Tracked {
                bid,
                cursor,
                fate: Rc::default(),
            },
        );
        token
    }

    /// Where `token`'s position is now, and in which buffer. `None` for a
    /// released token, a buffer that is closed, or one whose text was
    /// replaced.
    pub(in crate::editor) fn position(
        &self,
        token: HostToken,
        buffers: &BufferStore,
    ) -> Option<(BufferId, CharOffset)> {
        let tracked = self.entries.get(&token).filter(|t| t.live())?;
        let text = buffers.try_get(tracked.bid)?.text();
        Some((
            tracked.bid,
            EditView::bind(text, &tracked.cursor)
                .primary()
                .head()
                .offset(),
        ))
    }

    pub(in crate::editor) fn untrack(&mut self, token: HostToken) {
        self.entries.remove(&token);
    }

    /// Keep `token` past the callback of the request that holds it.
    pub(in crate::editor) fn keep(&mut self, token: HostToken) {
        if let Some(tracked) = self.entries.get(&token).filter(|t| t.live()) {
            tracked.fate.keep();
        }
    }

    /// A request's hold on `token`'s position, `None` for a token that names
    /// none.
    pub(in crate::editor) fn hold(&self, token: HostToken) -> Option<TrackedHold> {
        self.entries
            .get(&token)
            .filter(|t| t.live())
            .map(|t| TrackedHold::new(Rc::clone(&t.fate)))
    }

    /// Carry every position tracked in `buffer` through `change`, dropping
    /// the ones a hold released.
    pub(in crate::editor) fn carry(&mut self, buffer: BufferId, change: &TextChange<'_>) {
        self.entries.retain(|_, tracked| tracked.live());
        for tracked in self.entries.values_mut() {
            if tracked.bid == buffer {
                tracked.cursor.translate(change);
            }
        }
    }

    /// Drop the positions tracked in `buffer`, whose text was replaced or
    /// which closed.
    pub(in crate::editor) fn prune_buffer(&mut self, buffer: BufferId) {
        self.entries.retain(|_, tracked| tracked.bid != buffer);
    }

    /// How many positions are being tracked, for tests of a script's cleanup.
    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.entries.values().filter(|t| t.live()).count()
    }

    /// Drop every position: the scripts that held the tokens are gone.
    pub(in crate::editor) fn clear(&mut self) {
        self.entries.clear();
    }
}
