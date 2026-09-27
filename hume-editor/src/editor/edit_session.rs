//! The one Insert, Paste, or dot-repeat-replay session live in the editor,
//! if any.
//!
//! [`EditorState::active_session`] relies on the invariant that at most one
//! pane is ever mid-Insert, has a paste session open, or holds a dot-repeat
//! replay's own placeholder. Every session records its own owner `(pane,
//! buffer)`, so `focus::end_focus_sessions` (run by
//! every focus change and buffer switch) commits or tears it down by
//! reading that owner directly rather than current focus; ordering relative
//! to the focus write doesn't matter. Recording the owner as one value also
//! makes "is there a conflicting session elsewhere" a single field
//! comparison rather than a scan over every pane, and makes a stale session
//! left open by a caller that forgot to tear it down impossible to
//! represent silently: there is exactly one `Option` to get right, not one
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
    /// The pane that opened this session, always the focused pane, since
    /// Insert/paste sessions only ever open there.
    pane: PaneId,
    /// The buffer this session is editing.
    buffer: BufferId,
    kind: EditSessionKind,
    group: EditGroup,
    /// Armed around an Insert-key binding dispatch, a completion accept, or
    /// a re-armed picker hand-off; see [`DotCapture`]'s own doc. `None`
    /// outside such a dispatch, and always on a non-`Insert` session (every
    /// armer requires an open Insert session first).
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

    pub(in crate::editor) fn dot_capture_mut(&mut self) -> Option<&mut DotCapture> {
        self.dot_capture.as_mut()
    }

    /// Arms `cap`. Called by `Editor::run_dot_captured`, just before the
    /// operation that might go interactive, and again to re-arm a capture
    /// handed off from a resolved picker. Never nests: an Insert-key binding
    /// that itself calls `completion-accept!` shares the *outer* capture
    /// instead (see `EditorState::mark_dot_interactive`'s own doc), so a
    /// capture is always taken back out (by `Editor::run_dot_captured`
    /// (finalizing it, or handing it to a picker that opened mid-dispatch),
    /// or by `tear_down_insert`'s backstop) before the next arm.
    pub(in crate::editor) fn arm_dot_capture(&mut self, cap: DotCapture) {
        debug_assert!(
            self.dot_capture.is_none(),
            "arm_dot_capture: a capture was already armed and never resolved"
        );
        self.dot_capture = Some(cap);
    }

    /// Takes the armed capture, if any: `Editor::run_dot_captured`'s own
    /// finalize/hand-off paths, and `tear_down_insert`'s teardown-time
    /// backstop.
    pub(in crate::editor) fn take_dot_capture(&mut self) -> Option<DotCapture> {
        self.dot_capture.take()
    }

    /// Consumes the session, handing its [`EditGroup`] to the caller that's
    /// about to commit it (`doc_ops::commit_open_session`), the one place
    /// that needs the group by value rather than by reference.
    ///
    /// A live `dot_capture` here would be silently dropped along with the
    /// rest of the session. `tear_down_insert`'s own backstop is what must
    /// take it out first (see that function's own doc), so reaching this
    /// with one still armed is a bug in that ordering, not a normal exit.
    pub(in crate::editor) fn into_group(self) -> EditGroup {
        debug_assert!(
            self.dot_capture.is_none(),
            "into_group: a capture was still armed; tear_down_insert's backstop should have \
             taken it first"
        );
        self.group
    }

    /// `true` if this is an Insert-kind session on `(pane, buffer)`: the
    /// check every caller composing an edit into an *Insert* session (as
    /// opposed to a Paste one, which has its own `group` but a different
    /// commit lifecycle) must make before touching it. A caller that only
    /// checked `pane`/`buffer` and skipped `kind` could silently compose an
    /// Insert-mode edit into an open Paste session's group instead of
    /// panicking on the mismatch. This is the one place that three-way
    /// check is spelled out, so it can't be forgotten at a new call site.
    pub(in crate::editor) fn is_insert_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Insert)
    }

    /// `true` if this is a Paste-kind session on `(pane, buffer)`: the
    /// `is_insert_at` counterpart for the other kind [`EditSessionKind`]
    /// can hold.
    pub(in crate::editor) fn is_paste_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Paste { .. })
    }

    /// `true` if this session belongs to `(pane, buffer)`, regardless of
    /// kind: the check [`open_or_retarget`] and every generic teardown
    /// (`focus::end_focus_sessions`'s own remaining-shape handling) needs,
    /// as opposed to `is_insert_at`'s kind-specific one.
    pub(in crate::editor) fn owned_by(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer
    }

    /// `true` if this is `Editor::replay_dot`'s own still-unclaimed
    /// placeholder on `(pane, buffer)`, the one kind [`open_or_retarget`]
    /// may retarget in place. See [`EditSessionKind::Replay`]'s own doc for
    /// why a real, empty `Insert`/`Paste` session must not also satisfy
    /// this.
    pub(in crate::editor) fn is_replay_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Replay)
    }

    /// `true` if this session would block a fresh [`open_or_retarget`]/
    /// [`check_can_open`] call at `(pane, buffer)`: anything except the
    /// still-unclaimed replay placeholder sitting there is a real conflict.
    fn blocks_open(&self, pane: PaneId, buffer: BufferId) -> bool {
        !self.is_replay_at(pane, buffer)
    }
}

/// `Err` if a session open right now would refuse a fresh
/// [`open_or_retarget`] call at `(pane, buffer)`: the exact rule that
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
/// placeholder. Nothing has composed into it (nothing ever can, see that
/// variant's own doc), so retargeting its `kind` in place costs nothing and
/// keeps its already-correct `text_snapshot`/`pre_sels`, both captured
/// before *any* dispatch in the current action ran, exactly what a fresh
/// open right now would capture too, since nothing has touched the buffer
/// in between. This is what lets a replayed Steel body that pre-opens the
/// placeholder (`Editor::replay_dot`'s default) and then calls a native
/// paste command succeed instead of colliding with it: the paste takes
/// over the still-open placeholder rather than finding it already occupied.
///
/// `Err` otherwise: a real, already-open `Insert` or `Paste` session on
/// this `(pane, buffer)` (even one that's still empty) or any session on
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

/// Which kind of session [`EditSession`] is: the three shapes HUME's one
/// session slot can hold.
#[derive(Clone, Copy)]
pub(in crate::editor) enum EditSessionKind {
    /// An Insert-mode session: every keystroke since entry composes into
    /// [`EditSession::group`] until Esc/Ctrl-c/a mode change commits it as
    /// one undo step.
    Insert,
    /// An open `p`/`P` + `[`/`]` ring-cycle session. `before` is the
    /// direction the session was opened with (`true` = `P`/paste-before),
    /// meaningful only while the session is open; read by `[`/`]` so
    /// cycling re-pastes in the same direction as the opening `p`/`P`.
    Paste { before: bool },
    /// `Editor::replay_dot`'s own placeholder, pre-opened before the
    /// replayed body runs, to fold a recipe replay plus the main edit into
    /// one undo revision. Holds no data of its own: nothing ever composes
    /// into it directly, and no `InsertLayer` or paste-cycle state is
    /// attached while it's this kind. The replayed body resolves it to
    /// `Insert` or `Paste` in place (`open_or_retarget`'s retarget branch)
    /// if it claims it; otherwise it stays `Replay` for
    /// `finish_replay_session` to commit directly, as a plain edit.
    ///
    /// The *only* kind [`open_or_retarget`] may retarget away from. A real,
    /// already-open `Insert` or `Paste` session, even one that's still
    /// empty, must refuse a conflicting open rather than silently hand its
    /// group to whatever's asking. This variant exists to close exactly
    /// that bug: before it, "empty" alone stood in for "is this the replay
    /// placeholder," and a real Insert session, in the one keystroke
    /// between entry and its first typed character, matched that test just
    /// as well.
    Replay,
}

/// Armed around an Insert-key binding dispatch, or a completion accept,
/// that might turn out interactive: one whose outcome depends on input the
/// user gave while it ran, such as accepting a completion or picking a
/// picker item.
///
/// Owned by whatever is currently responsible for resolving it, never
/// polled: armed on the `EditSession` it belongs to by `Editor::
/// with_dot_capture`; if the dispatch it wraps opens a picker,
/// `picker::open_picker` takes it off the session and attaches it to the
/// `PickerSession` instead (so an edit made elsewhere while the picker is
/// open, such as a timer or an LSP response, does not land in `edits`, since
/// nothing is armed on the `EditSession` to feed); when the picker resolves,
/// `close_picker_with`/`PickerLayer::tear_down` hand it to the queued
/// `PendingWork::Call` that will run its `on_select`; `Editor::
/// run_pending_batch` re-arms it on the (now current) session for just that
/// call. A picker chain (`on_select` itself opening another picker) simply
/// repeats the hand-off. Whichever holder currently owns it takes it back
/// out via `Editor::run_dot_captured` once the operation it wraps returns,
/// or, if the `EditSession` it was armed on ends first (an accept
/// immediately followed by `exit-insert` in the same dispatch),
/// `tear_down_insert`'s own backstop takes it before the session is
/// dropped. `edit_session::EditSession::into_group`'s `debug_assert` is the
/// enforcement: nothing may reach it with a capture still attached.
#[derive(Debug)]
pub(in crate::editor) struct DotCapture {
    /// The `(pane, buffer)` this capture was armed on: the target `Editor::
    /// run_dot_captured` re-arms it on when a hand-off (a picker's `on_select`)
    /// resolves later against whatever session is current by then, which may
    /// no longer be this one.
    pub(in crate::editor) pane: PaneId,
    pub(in crate::editor) buffer: BufferId,
    /// The primary cursor's head in the old-document space `edits[0]`'s own
    /// `ChangeSet` was computed against: the point `cursor_replacement_at`
    /// (`replay.rs`) locates the net edit relative to, once `edits` is
    /// composed into one. Refreshed from the *current* cursor whenever
    /// `Editor::run_dot_captured` re-arms a capture whose `edits` is still
    /// empty (which is exactly the coordinate space the *next* edit
    /// `apply_doc_edit_grouped` pushes will use), but left alone once
    /// `edits` holds at least one entry, since every later entry must chain
    /// from that first one's own old-document space for
    /// `ChangeSet::compose_all` to line up. See [`Self::text_gen`] for what
    /// guards that chain against a foreign edit breaking it.
    pub(in crate::editor) head_before: CharOffset,
    /// Every `ChangeSet` composed into this session's group
    /// (`doc_ops::apply_doc_edit_grouped`'s own funnel push) while this
    /// capture was armed, in order. `ChangeSet::compose_all` folds them
    /// into the capture's net transform. Whatever ran between arming and
    /// resolving collapses into *one* edit this way: a binding that accepts
    /// a completion and then runs its own follow-up edit records both
    /// together, never the accept alone. Once any part of a dispatch goes
    /// interactive, none of it is safe to re-derive at a new cursor, so the
    /// whole thing is captured as data instead.
    pub(in crate::editor) edits: Vec<ChangeSet>,
    /// The buffer's `text_gen` right after `edits`' own last push (or at arm
    /// time, if `edits` is still empty). Set alongside every push in
    /// `apply_doc_edit_grouped`'s funnel, and at construction in
    /// `Editor::with_dot_capture`. `run_dot_captured`'s re-arm compares this
    /// against the buffer's *current* `text_gen`: a match means the buffer
    /// is exactly as this capture left it, so it's safe to refresh
    /// `head_before` (if `edits` is still empty) or keep composing (if not:
    /// `edits`' last entry's `len_after` still matches the buffer). A
    /// mismatch means a foreign edit (a hook, an LSP response, a timer)
    /// landed on this buffer while the capture sat detached from it (armed
    /// on a picker instead of this session). That is harmless when `edits` was
    /// still empty (nothing of this capture's own to break), but breaks the
    /// composition chain outright once `edits` already holds an entry:
    /// `compose_all` would panic on a length mismatch between that entry's
    /// `len_after` and the next one's `len_before`. `run_dot_captured`
    /// detects that case from the mismatch and drops the capture instead of
    /// composing it.
    pub(in crate::editor) text_gen: u64,
    /// Set by `EditorState::mark_dot_interactive`: `completion-accept!`/
    /// `completion-trigger` call it directly, `picker!`/`live-picker!`
    /// indirectly via `picker::open_picker`, whichever fires first. A
    /// capture that never sees this stay `false` belongs to an ordinary
    /// (non-interactive) binding: its `edits` are discarded at finalize
    /// time in favor of `fallback`.
    pub(in crate::editor) interactive: bool,
    /// What to record if this capture turns out non-interactive: the
    /// `Binding` entry `handle_insert`'s Leaf branch would otherwise have
    /// pushed before dispatching, or `None` for a bare Enter-key accept
    /// (`accept_completion_selection`), which has no binding of its own to
    /// fall back to.
    pub(in crate::editor) fallback: Option<super::replay::InsertInput>,
}

/// Accumulated state for an in-progress insert or paste session.
pub(in crate::editor) struct EditGroup {
    /// Buffer text snapshot taken at `begin_edit_group`. Used by
    /// `commit_edit_group` to invert the composed CS and record a single
    /// history revision.
    pub(in crate::editor) text_snapshot: BufferText,
    /// Selection state at group open, stored in the history revision so
    /// undo restores the cursor to its pre-insert position.
    pub(in crate::editor) pre_sels: SelectionSet,
    /// Running composition of all forward ChangeSets applied since the group
    /// opened. `None` until the first keystroke (empty session = no revision
    /// recorded on commit).
    pub(in crate::editor) cs: Option<ChangeSet>,
}
