//! The one Insert or paste session live in the editor, if any.
//!
//! [`EditorState::active_session`] replaces a pair of per-(pane, buffer)
//! `edit_group`/`paste_group` fields that used to live on `PaneBufferState`,
//! relying on the convention that at most one pane is ever in Insert or has
//! a paste session open — every focus change and buffer switch had to
//! remember to call `focus::end_focus_sessions` first, in the right order,
//! with nothing enforcing that structurally. Recording the owner (which
//! pane, which buffer) as one value instead makes "is there a conflicting
//! session elsewhere" a single field comparison rather than a scan over
//! every pane, and makes a stale session left open by a caller that forgot
//! to tear it down impossible to represent silently — there is exactly one
//! `Option` to get right, not one per (pane, buffer) pair. Mirrors the
//! pattern completion sessions already use (`BufferSession` stores its
//! `pane_id` directly on the session, not keyed by a per-pane map).

use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, PaneId};

/// The editor's one live Insert or paste session, if any.
pub(in crate::editor) struct EditSession {
    /// The pane that opened this session — always the focused pane, since
    /// Insert/paste sessions only ever open there.
    pub(in crate::editor) pane: PaneId,
    /// The buffer this session is editing.
    pub(in crate::editor) buffer: BufferId,
    pub(in crate::editor) kind: EditSessionKind,
    pub(in crate::editor) group: EditGroup,
}

impl EditSession {
    /// `true` if this is an Insert-kind session on `(pane, buffer)` — the
    /// check every caller composing an edit into an *Insert* session (as
    /// opposed to a Paste one, which has its own `group` but a different
    /// commit lifecycle) must make before touching it. A caller that only
    /// checked `pane`/`buffer` and skipped `kind` could silently compose an
    /// Insert-mode edit into an open Paste session's group instead of
    /// panicking on the mismatch — this is the one place that three-way
    /// check is spelled out, so it can't be forgotten at a new call site.
    pub(in crate::editor) fn is_insert_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer && matches!(self.kind, EditSessionKind::Insert)
    }
}

/// Which kind of session [`EditSession`] is — the two ways HUME opens an
/// undo group that spans more than one edit.
#[derive(Clone, Copy)]
pub(in crate::editor) enum EditSessionKind {
    /// An Insert-mode session: every keystroke since entry composes into
    /// [`EditSession::group`] until Esc/Ctrl-c/a mode change commits it as
    /// one undo step.
    Insert,
    /// An open `p`/`P` + `[`/`]` ring-cycle session. `before` is the
    /// direction the session was opened with (`true` = `P`/paste-before) —
    /// meaningful only while the session is open; read by `[`/`]` so
    /// cycling re-pastes in the same direction as the opening `p`/`P`.
    Paste { before: bool },
}

/// Accumulated state for an in-progress insert or paste session.
pub(in crate::editor) struct EditGroup {
    /// Buffer text snapshot taken at `begin_edit_group`. Used by
    /// `commit_edit_group` to invert the composed CS and record a single
    /// history revision.
    pub(in crate::editor) text_snapshot: BufferText,
    /// Selection state at group open — stored in the history revision so
    /// undo restores the cursor to its pre-insert position.
    pub(in crate::editor) pre_sels: SelectionSet,
    /// Running composition of all forward ChangeSets applied since the group
    /// opened. `None` until the first keystroke (empty session = no revision
    /// recorded on commit).
    pub(in crate::editor) cs: Option<ChangeSet>,
}
