//! The one Insert or paste session live in the editor, if any.
//!
//! [`EditorState::active_session`] relies on the invariant that at most one
//! pane is ever in Insert or has a paste session open — every focus change
//! and buffer switch must call `focus::end_focus_sessions` first, in the
//! right order. Recording the owner (which pane, which buffer) as one value
//! makes "is there a conflicting session elsewhere" a single field
//! comparison rather than a scan over every pane, and makes a stale session
//! left open by a caller that forgot to tear it down impossible to
//! represent silently — there is exactly one `Option` to get right, not one
//! per (pane, buffer) pair. Mirrors the pattern completion sessions already
//! use (`BufferSession` stores its `pane_id` directly on the session, not
//! keyed by a per-pane map).

use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::error::CommandError;

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

    /// `true` if this session belongs to `(pane, buffer)`, regardless of
    /// kind — the check [`open_or_retarget`] and every generic teardown
    /// (`focus::end_focus_sessions`'s own remaining-shape handling) needs,
    /// as opposed to `is_insert_at`'s kind-specific one.
    pub(in crate::editor) fn owned_by(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer
    }

    /// `true` if this session has composed no edits yet — the condition
    /// under which [`open_or_retarget`] may retarget its `kind` in place
    /// rather than refuse.
    fn is_empty(&self) -> bool {
        self.group.cs.is_none()
    }

    /// `true` if this session would block a fresh [`open_or_retarget`] call
    /// at `(pane, buffer)` — a real conflict, not the empty-retarget case.
    /// Exposed so a caller that needs to know the outcome *before* mutating
    /// anything else (`commands::paste`'s `do_normal_paste`/`do_smart_paste`,
    /// which must not take the live selection only to discover paste would
    /// have refused) can check first, using the exact same rule
    /// `open_or_retarget` itself applies.
    pub(in crate::editor) fn blocks_open(&self, pane: PaneId, buffer: BufferId) -> bool {
        !self.owned_by(pane, buffer) || !self.is_empty()
    }
}

/// Open a session of `kind` on `(pane, buffer)`, or take over an empty one
/// already open there.
///
/// `Ok` when: no session is open (opens fresh via `mint_group`), or the one
/// open is on this exact `(pane, buffer)` and is still empty. An empty
/// session has composed nothing, so retargeting its `kind` in place costs
/// nothing and keeps its already-correct `text_snapshot`/`pre_sels` — both
/// captured before *any* dispatch in the current action ran, exactly what a
/// fresh open right now would capture too, since nothing has touched the
/// buffer in between. This is what lets a replayed Steel body that
/// pre-opens an Insert-kind session (`Editor::replay_dot`'s default
/// assumption) and then calls a native paste command succeed instead of
/// colliding with it — the paste takes over the still-empty pre-opened slot
/// rather than finding it already occupied.
///
/// `Err` otherwise: a real, non-empty session on this `(pane, buffer)`, or
/// any session on a *different* one — both are genuine conflicts that must
/// never be silently overwritten (see this module's own doc).
pub(in crate::editor) fn open_or_retarget(
    active_session: &mut Option<EditSession>,
    pane: PaneId,
    buffer: BufferId,
    kind: EditSessionKind,
    mint_group: impl FnOnce() -> EditGroup,
) -> Result<(), CommandError> {
    match active_session {
        Some(existing) if existing.blocks_open(pane, buffer) => {
            return Err(CommandError::transient(
                "buffer has an open insert/paste session already",
            ));
        }
        Some(existing) => existing.kind = kind,
        None => {
            *active_session = Some(EditSession {
                pane,
                buffer,
                kind,
                group: mint_group(),
            });
        }
    }
    Ok(())
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
