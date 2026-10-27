//! The pane a native command or pane-taking builtin acts through, and the
//! focused pane as a type of its own.

use hume_core::{BufferId, PaneHandle, PaneId};
use hume_engine::pipeline::{EngineView, PanePool};

use crate::editor::EditorState;
use crate::editor::pane_state::{PaneBufferState, PaneEntries};

/// A pane resolved as the target of a native command's Pane-category body,
/// or of a pane-taking builtin: the pane a `(call! "cmd" pane)` or a
/// builtin acts through, which need not be the focused pane. The private
/// field means the only ways to get one are [`Self::resolve`] or narrowing a
/// [`FocusedPane`], never wrapping an arbitrary `PaneId` from outside.
///
/// `bid` is read live rather than cached at resolution time: a jump or
/// `goto-*-buffer` body switches the pane's own buffer, and the AFTER steps
/// (`with_jump`, `step_align_view`) must see that switch, not the
/// buffer the target was resolved for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct CommandPane(PaneId);

impl CommandPane {
    /// Resolve `handle` to its pane, requiring the buffer to be live, the
    /// handle to carry a pane, that pane to still exist, and it to still
    /// show the handle's buffer. A handle with a pane is a claim about a
    /// view, and a view that no longer shows its buffer is stale.
    pub(in crate::editor) fn resolve(
        state: &EditorState,
        view: &EngineView,
        handle: PaneHandle,
    ) -> Result<Self, TargetError> {
        let bid = handle.buffer();
        if state.buffers.try_get(bid).is_none() {
            return Err(TargetError::Closed);
        }
        let pid = handle.pane().ok_or(TargetError::NoPane)?;
        let Some(pane) = view.panes.get(pid) else {
            return Err(TargetError::PaneClosed);
        };
        if pane.buffer_id() != bid {
            return Err(TargetError::PaneShowsOther);
        }
        Ok(Self(pid))
    }

    /// `pid` as a target if that pane still exists: for a session that
    /// recorded its own pane when it opened, which focus may have left since.
    pub(in crate::editor) fn existing(view: &EngineView, pid: PaneId) -> Option<Self> {
        view.panes.contains_key(pid).then_some(Self(pid))
    }

    pub(in crate::editor) fn pid(self) -> PaneId {
        self.0
    }

    pub(in crate::editor) fn bid(self, view: &EngineView) -> BufferId {
        view.panes[self.0].buffer_id()
    }

    /// `t`'s state for the buffer it shows.
    pub(in crate::editor) fn state<'a>(
        self,
        pane_state: &'a PaneEntries,
        view: &EngineView,
    ) -> &'a PaneBufferState {
        pane_state.shown(&view.panes, self.0)
    }

    /// [`Self::state`]'s mutable counterpart.
    pub(in crate::editor) fn state_mut<'a>(
        self,
        pane_state: &'a mut PaneEntries,
        view: &EngineView,
    ) -> &'a mut PaneBufferState {
        pane_state.shown_mut(&view.panes, self.0)
    }
}

/// What a change to `buffer` requested through `requester` acts through:
/// `requester` while it shows `buffer`, no pane otherwise. Unlike
/// [`CommandPane::resolve`], a requester that shows another buffer is not an
/// error, so one request can edit files no pane shows.
pub(in crate::editor) fn shown_through(
    panes: &PanePool,
    requester: Option<PaneId>,
    buffer: BufferId,
) -> PaneHandle {
    match requester.filter(|&pid| panes.get(pid).is_some_and(|p| p.buffer_id() == buffer)) {
        Some(pid) => PaneHandle::with_pane(buffer, pid),
        None => PaneHandle::buffer_only(buffer),
    }
}

/// A [`CommandPane`] proven to be the focused pane at the
/// moment it was minted. The single admission point for focus-bound state
/// (the open Insert/paste session, the sticky Extend flag, dot-repeat), so a
/// body that only received a `CommandPane` has no way to reach any of it: it
/// would need a `FocusedPane`, which it was never given.
///
/// Minted three ways: [`Self::resolve`] checks an explicit handle,
/// [`Self::of`] checks a resolved target, and [`Self::current`] reads live
/// focus. `current` belongs only at an entry point: the place an input first
/// arrives and "the focused pane" is the meaning of the input itself:
/// keypress and `:` dispatch (`dispatch.rs`,
/// `input_stack/{base,command,insert,search}.rs`), dot-repeat replay, the
/// `(focused-pane)` builtin, a confirm answer (`buffer/disk.rs`), an effect
/// applied after its eval (`lsp/registry.rs`), a server's
/// `workspace/applyEdit` request (`lsp/edits.rs`), and `Editor`'s own
/// focused-pane readers (`mod.rs`). A command body, a typed command, or a
/// helper they call receives the pane instead of minting it.
///
/// `Copy`. It names the pane focused when it was minted, and a focus change
/// (a function of the `focus` module) does not update it. Holding one
/// across a command body that can move focus, as [`run`](super::run) does
/// for its after-body steps, means "the pane focused at entry", not current
/// focus.
#[derive(Clone, Copy, Debug)]
pub(in crate::editor) struct FocusedPane(CommandPane);

impl FocusedPane {
    /// The one mint for focus-context code; see this type's own doc.
    pub(in crate::editor) fn current(view: &EngineView) -> Self {
        Self(CommandPane(view.focused_pane()))
    }

    /// [`CommandPane::resolve`], additionally requiring the pane to be the
    /// focused one, for a caller that names a pane explicitly but acts on
    /// focus-bound state (a popup/menu/drawer/picker opener, a
    /// FocusedPane-category `call!`, a focus-anchored LSP response).
    pub(in crate::editor) fn resolve(
        state: &EditorState,
        view: &EngineView,
        handle: PaneHandle,
    ) -> Result<Self, TargetError> {
        let target = CommandPane::resolve(state, view, handle)?;
        Self::of(target, view).ok_or(TargetError::NotFocused)
    }

    /// `target` as the focused pane, if it is the focused one.
    pub(in crate::editor) fn of(target: CommandPane, view: &EngineView) -> Option<Self> {
        (target.pid() == view.focused_pane()).then_some(Self(target))
    }

    pub(in crate::editor) fn pane(self) -> CommandPane {
        self.0
    }

    pub(in crate::editor) fn pid(self) -> PaneId {
        self.0.pid()
    }

    pub(in crate::editor) fn bid(self, view: &EngineView) -> BufferId {
        self.0.bid(view)
    }

    /// This pane as the `pane` half of a value handed back to Steel (a hook
    /// argument, a completion source's own pane). The tuple always carries
    /// the focused pid, never bare focus state a callee would have to
    /// re-derive.
    pub(in crate::editor) fn handle(self, view: &EngineView) -> PaneHandle {
        PaneHandle::with_pane(self.bid(view), self.pid())
    }
}

/// Why a [`PaneHandle`] could not resolve to a [`CommandPane`] or
/// [`FocusedPane`]. `Display`ed by `EditorHostImpl::
/// run_command_sync` and by every pane-taking builtin's host method, the
/// places these reach a user (via `%call-native!`'s `Err`, or a builtin's
/// own raise). Checked in declaration order: a closed buffer reports
/// `Closed` even when the handle also carries no pane.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) enum TargetError {
    /// The handle's buffer names no live buffer.
    Closed,
    /// The command/builtin needs a pane, and the handle carried none.
    NoPane,
    /// The handle's pane no longer exists.
    PaneClosed,
    /// The handle's pane exists but no longer shows its buffer.
    PaneShowsOther,
    /// The command/builtin needs the *focused* pane, and the handle's pane
    /// isn't it.
    NotFocused,
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "invalid buffer id"),
            Self::NoPane => write!(f, "needs a pane, but was given none"),
            Self::PaneClosed => write!(f, "pane has been closed"),
            Self::PaneShowsOther => write!(f, "pane no longer shows this buffer"),
            Self::NotFocused => write!(f, "acts on the focused pane, which this pane isn't"),
        }
    }
}
