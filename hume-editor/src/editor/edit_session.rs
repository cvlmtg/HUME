//! The one Insert, Paste, or dot-repeat-replay session live in the editor,
//! if any.
//!
//! [`EditorState::active_session`] relies on the invariant that at most one
//! pane is ever mid-Insert, has a paste session open, or holds a dot-repeat
//! replay's own placeholder. Every session records its own owner `(pane,
//! buffer)`, so `focus::end_focus_sessions` — run by
//! every focus change and buffer switch — commits or tears it down by
//! reading that owner directly rather than current focus; ordering relative
//! to the focus write doesn't matter. Recording the owner as one value also
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
use hume_rope::offset::CharOffset;

use crate::editor::error::CommandError;

/// The editor's one live Insert or paste session, if any.
///
/// Fields are private: [`open_or_retarget`] is the only way to create one or
/// change its `kind`, which is what makes the `Replay` retarget rule (see
/// [`EditSessionKind::Replay`]) a compile-time guarantee rather than a
/// convention every call site has to honor on its own.
pub(in crate::editor) struct EditSession {
    /// The pane that opened this session — always the focused pane, since
    /// Insert/paste sessions only ever open there.
    pane: PaneId,
    /// The buffer this session is editing.
    buffer: BufferId,
    kind: EditSessionKind,
    group: EditGroup,
    /// Armed around an Insert-key binding dispatch, or a completion accept,
    /// that might turn out interactive — see [`DotCapture`]'s own doc.
    /// `None` outside such a dispatch, and always on a non-`Insert` session
    /// (only `handle_insert`/`accept_completion_selection` arm it, and both
    /// require an open Insert session first).
    dot_capture: Option<DotCapture>,
}

impl EditSession {
    pub(in crate::editor) fn pane(&self) -> PaneId {
        self.pane
    }

    pub(in crate::editor) fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub(in crate::editor) fn kind(&self) -> EditSessionKind {
        self.kind
    }

    pub(in crate::editor) fn group_mut(&mut self) -> &mut EditGroup {
        &mut self.group
    }

    /// The armed dot-capture, if any — see [`DotCapture`]'s own doc.
    pub(in crate::editor) fn dot_capture(&self) -> Option<&DotCapture> {
        self.dot_capture.as_ref()
    }

    pub(in crate::editor) fn dot_capture_mut(&mut self) -> Option<&mut DotCapture> {
        self.dot_capture.as_mut()
    }

    /// Arms a fresh [`DotCapture`] — `handle_insert`'s Leaf branch and
    /// `accept_completion_selection` are the only callers, each just before
    /// the operation that might go interactive. Neither ever nests inside
    /// the other's dispatch (an Insert-key binding that itself calls
    /// `completion-accept!` shares the *outer* capture instead — see
    /// `mark_dot_interactive`'s own doc), so a capture is always resolved by
    /// `Editor::resolve_dot_capture` or `tear_down_insert`'s backstop before
    /// the next arm.
    pub(in crate::editor) fn arm_dot_capture(
        &mut self,
        head_before: CharOffset,
        has_placeholder: bool,
    ) {
        debug_assert!(
            self.dot_capture.is_none(),
            "arm_dot_capture: a capture was already armed and never resolved"
        );
        self.dot_capture = Some(DotCapture {
            head_before,
            edits: Vec::new(),
            interactive: false,
            has_placeholder,
        });
    }

    /// Takes the armed capture, if any — `Editor::resolve_dot_capture`'s own
    /// finalize/drop paths, and `tear_down_insert`'s teardown-time backstop.
    pub(in crate::editor) fn take_dot_capture(&mut self) -> Option<DotCapture> {
        self.dot_capture.take()
    }

    /// Consumes the session, handing its [`EditGroup`] to the caller that's
    /// about to commit it (`doc_ops::commit_open_session`) — the one place
    /// that needs the group by value rather than by reference.
    pub(in crate::editor) fn into_group(self) -> EditGroup {
        self.group
    }

    /// `true` if this is an Insert-kind session on `(pane, buffer)` — the
    /// check every caller composing an edit into an *Insert* session (as
    /// opposed to a Paste one, which has its own `group` but a different
    /// commit lifecycle) must make before touching it. A caller that only
    /// checked `pane`/`buffer` and skipped `kind` could silently compose an
    /// Insert-mode edit into an open Paste session's group instead of
    /// panicking on the mismatch — this is the one place that three-way
    /// check is spelled out, so it can't be forgotten at a new call site.
    pub(in crate::editor) fn is_insert_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Insert)
    }

    /// `true` if this is a Paste-kind session on `(pane, buffer)` — the
    /// `is_insert_at` counterpart for the other kind [`EditSessionKind`]
    /// can hold.
    pub(in crate::editor) fn is_paste_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Paste { .. })
    }

    /// `true` if this session belongs to `(pane, buffer)`, regardless of
    /// kind — the check [`open_or_retarget`] and every generic teardown
    /// (`focus::end_focus_sessions`'s own remaining-shape handling) needs,
    /// as opposed to `is_insert_at`'s kind-specific one.
    pub(in crate::editor) fn owned_by(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer
    }

    /// `true` if this is `Editor::replay_dot`'s own still-unclaimed
    /// placeholder on `(pane, buffer)` — the one kind [`open_or_retarget`]
    /// may retarget in place. See [`EditSessionKind::Replay`]'s own doc for
    /// why a real, empty `Insert`/`Paste` session must not also satisfy
    /// this.
    pub(in crate::editor) fn is_replay_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Replay)
    }

    /// `true` if this session would block a fresh [`open_or_retarget`]/
    /// [`check_can_open`] call at `(pane, buffer)` — anything except the
    /// still-unclaimed replay placeholder sitting there is a real conflict.
    fn blocks_open(&self, pane: PaneId, buffer: BufferId) -> bool {
        !self.is_replay_at(pane, buffer)
    }
}

/// `Err` if a session open right now would refuse a fresh
/// [`open_or_retarget`] call at `(pane, buffer)` — the exact rule that
/// function applies, exposed so a caller that needs to know the outcome
/// *before* mutating anything else (`commands::paste`'s
/// `do_normal_paste`/`do_smart_paste`, which must not take the live
/// selection only to discover paste would have refused) can check first.
pub(in crate::editor) fn check_can_open(
    active_session: &Option<EditSession>,
    pane: PaneId,
    buffer: BufferId,
) -> Result<(), CommandError> {
    if active_session
        .as_ref()
        .is_some_and(|s| s.blocks_open(pane, buffer))
    {
        return Err(CommandError::transient(
            "buffer has an open insert/paste session already",
        ));
    }
    Ok(())
}

/// Open a session of `kind` on `(pane, buffer)`, or take over the
/// [`EditSessionKind::Replay`] placeholder already open there.
///
/// `Ok` when: no session is open (opens fresh via `mint_group`), or the one
/// open is on this exact `(pane, buffer)` and is still the Replay
/// placeholder. Nothing has composed into it — nothing ever can, see that
/// variant's own doc — so retargeting its `kind` in place costs nothing and
/// keeps its already-correct `text_snapshot`/`pre_sels` — both captured
/// before *any* dispatch in the current action ran, exactly what a fresh
/// open right now would capture too, since nothing has touched the buffer
/// in between. This is what lets a replayed Steel body that pre-opens the
/// placeholder (`Editor::replay_dot`'s default) and then calls a native
/// paste command succeed instead of colliding with it — the paste takes
/// over the still-open placeholder rather than finding it already occupied.
///
/// `Err` otherwise: a real, already-open `Insert` or `Paste` session on
/// this `(pane, buffer)` — even one that's still empty — or any session on
/// a *different* one. All three are genuine conflicts that must never be
/// silently overwritten (see this module's own doc and
/// [`EditSessionKind::Replay`]'s own); see [`check_can_open`] for the exact
/// rule.
pub(in crate::editor) fn open_or_retarget(
    active_session: &mut Option<EditSession>,
    pane: PaneId,
    buffer: BufferId,
    kind: EditSessionKind,
    mint_group: impl FnOnce() -> EditGroup,
) -> Result<(), CommandError> {
    check_can_open(active_session, pane, buffer)?;
    match active_session {
        Some(existing) => existing.kind = kind,
        None => {
            *active_session = Some(EditSession {
                pane,
                buffer,
                kind,
                group: mint_group(),
                dot_capture: None,
            });
        }
    }
    Ok(())
}

/// Which kind of session [`EditSession`] is — the three shapes HUME's one
/// session slot can hold.
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
    /// `Editor::replay_dot`'s own placeholder, pre-opened before the
    /// replayed body runs, to fold a recipe replay plus the main edit into
    /// one undo revision. Holds no data of its own —
    /// nothing ever composes into it directly, and no `InsertLayer` or
    /// paste-cycle state is attached while it's this kind. The replayed
    /// body resolves it to `Insert` or `Paste` in place
    /// (`open_or_retarget`'s retarget branch) if it claims it; otherwise it
    /// stays `Replay` for `finish_replay_session` to commit directly, as a
    /// plain edit.
    ///
    /// The *only* kind [`open_or_retarget`] may retarget away from. A real,
    /// already-open `Insert` or `Paste` session — even one that's still
    /// empty — must refuse a conflicting open rather than silently hand its
    /// group to whatever's asking. This variant exists to close exactly
    /// that bug: before it, "empty" alone stood in for "is this the replay
    /// placeholder," and a real Insert session, in the one keystroke
    /// between entry and its first typed character, matched that test just
    /// as well.
    Replay,
}

/// Armed around an Insert-key binding dispatch, or a completion accept,
/// that might turn out interactive — one whose outcome depends on input the
/// user gave while it ran, such as accepting a completion or picking a
/// picker item. Lives on the [`EditSession`] it belongs to, not on
/// `EditorState`: the capture's own edits only ever land in this session's
/// group (`doc_ops::apply_doc_edit_grouped` is the one path into it, and the
/// funnel this capture taps), and tying its lifetime to the session's means
/// a session that ends before the capture resolves — an accept immediately
/// followed by `exit-insert` in the same dispatch — takes the capture down
/// with it (see `tear_down_insert`'s own backstop) rather than leaving it to
/// dangle past the session it was captured for.
///
/// A completion accept resolves within the one dispatch that armed it,
/// always. Opening a picker is the one case that can't: the pick itself (or
/// a dismissal) resolves later, via `on_select`, queued as a
/// `PendingWork::Call` well after the dispatch that opened it has already
/// returned — for that case alone, `Editor::resolve_dot_capture` leaves this
/// still armed instead of finalizing, and `drain_pending_work`'s own calls
/// to it finalize once the pick lands. A picker chain (`on_select` itself
/// opening another picker) simply leaves it armed longer: `resolve_dot_
/// capture` only finalizes once no picker is left open at all, however many
/// links the chain has.
pub(in crate::editor) struct DotCapture {
    /// The primary cursor's head when this capture was armed — the point
    /// `cursor_replacement_at` (`replay.rs`) locates the net edit relative
    /// to.
    pub(in crate::editor) head_before: CharOffset,
    /// Every `ChangeSet` composed into this session's group
    /// (`doc_ops::apply_doc_edit_grouped`'s own funnel push) while this
    /// capture was armed, in order — `ChangeSet::compose_all` folds them
    /// into the capture's net transform. Whatever ran between arming and
    /// resolving collapses into *one* edit this way: a binding that accepts
    /// a completion and then runs its own follow-up edit records both
    /// together, never the accept alone — once any part of a dispatch goes
    /// interactive, none of it is safe to re-derive at a new cursor, so the
    /// whole thing is captured as data instead.
    pub(in crate::editor) edits: Vec<ChangeSet>,
    /// Set by `EditorState::mark_dot_interactive` — `completion-accept!`/
    /// `picker!`/`live-picker!`/`completion-trigger` call it, whichever
    /// fires first. A capture that never sees this stay `false` belongs to
    /// an ordinary (non-interactive) binding: its `edits` are discarded at
    /// resolve time, and the `Binding` entry recorded before the dispatch is
    /// left to replay by re-running, same as always.
    pub(in crate::editor) interactive: bool,
    /// `true` from `handle_insert`'s Leaf branch, whose `Binding` entry —
    /// pushed unconditionally before the dispatch it wraps — is what
    /// finalizing *replaces*; `false` from `accept_completion_selection`,
    /// which pushed no entry of its own for its Enter keypress, so
    /// finalizing there *appends* instead.
    pub(in crate::editor) has_placeholder: bool,
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
