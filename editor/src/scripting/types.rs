use steel::rvals::SteelVal;

use engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use crate::core::jump_list::JumpList;
use crate::editor::buffer_store::BufferStore;
use crate::editor::keymap::Keymap;
use crate::editor::pane_state::PaneBufferState;
use crate::settings::EditorSettings;

use super::attribution;

/// A `(define-command! …)` call captured during `eval_init`, to be processed
/// after the eval completes.
pub(crate) struct PendingSteelCmd {
    pub(crate) name: String,
    pub(crate) doc: String,
    /// The Steel lambda, captured at `define-command!` call time.
    pub(crate) proc: SteelVal,
    /// Attribution owner at call time — stored in `cmd_owners` for `(command-plugin …)`.
    pub(crate) current_owner: attribution::Owner,
    /// Whether this command participates in sticky-Ctrl extend.
    /// Set by `(define-command-extend! …)`.
    pub(crate) extendable: bool,
    /// Whether dispatch brackets the call with an alt-screen exit for live
    /// subprocess output. Set by `(define-command-inline-output! …)`.
    pub(crate) inline_output: bool,
}

/// A Steel command that has been fully registered in the engine and is ready
/// to be inserted into the `CommandRegistry`.
///
/// Returned by [`super::ScriptingHost::eval_init`] and
/// [`super::ScriptingHost::activate_plugin`]; the editor layer registers
/// the commands after a successful eval.
pub(crate) struct SteelCmdDef {
    pub(crate) name: String,
    pub(crate) doc: String,
    /// Name under which the lambda is bound in Steel's global namespace
    /// (e.g. `"%hume-cmd-my-command"`).  Used by
    /// [`crate::scripting::ScriptingHost::call_steel_cmd`] at dispatch time.
    pub(crate) steel_proc: String,
    pub(crate) extendable: bool,
    /// Number of required positional parameters the lambda accepts.
    /// Introspected once at `define-command!` time from the closure's arity.
    pub(crate) arity: u16,
    /// `true` if the lambda accepts a rest parameter (variadic).
    pub(crate) is_variadic: bool,
    /// `true` if dispatch should bracket this command with an alt-screen exit
    /// so subprocess output streams live to the terminal.
    pub(crate) inline_output: bool,
}

/// Language identity registration queued during `eval_init` and flushed by
/// `Editor::flush_pending_language_regs` after each `eval_init` boundary.
pub(crate) enum PendingLanguageReg {
    Identity {
        name: String,
        extensions: Vec<String>,
        globs: Vec<String>,
        shebangs: Vec<String>,
    },
}

/// `set-buffer-language!` calls deferred during a command or hook eval.
/// Each entry is `(buffer_id, language_name_or_none)`.
/// Drained by consumers (mappings.rs, fire_hook_silent) BEFORE cmd_queue.
pub(crate) type PendingLanguageSets = Vec<(BufferId, Option<String>)>;

/// A command queued by `(call! …)` inside a Steel command body.
///
/// `register` captures the sticky prefix active when `(call! …)` was reached
/// (set via `(set-register-prefix! …)`).  `None` means no register was
/// active at enqueue time, so the editor's existing `register_prefix` is left
/// untouched when this entry is dispatched.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueuedCommand {
    pub(crate) name: String,
    pub(crate) args: Vec<SteelVal>,
    pub(crate) register: Option<char>,
}

/// Result returned by [`super::ScriptingHost::call_steel_cmd`].
#[derive(Debug)]
pub(crate) struct SteelCmdResult {
    pub(crate) cmd_queue: Vec<QueuedCommand>,
    pub(crate) wait_char_request: Option<String>,
    pub(crate) pending_language_sets: PendingLanguageSets,
}

/// Result returned by [`super::ScriptingHost::fire_hook`].
#[derive(Debug)]
pub(crate) struct HookResult {
    pub(crate) cmd_queue: Vec<QueuedCommand>,
    pub(crate) pending_language_sets: PendingLanguageSets,
}

/// Editor-side references bundled for a single Steel eval in command mode.
///
/// Passed to [`super::ScriptingHost::call_steel_cmd`] and [`super::ScriptingHost::fire_hook`]
/// to replace the previous 8-parameter sprawl.  All fields have the same
/// lifetime `'a` so a single `'a` annotation on those methods suffices.
pub(crate) struct EditorSteelRefs<'a> {
    pub(crate) settings: &'a mut EditorSettings,
    pub(crate) keymap: &'a mut Keymap,
    pub(crate) focused_pane_id: PaneId,
    pub(crate) focused_buffer_id: BufferId,
    pub(crate) buffers: Option<&'a mut BufferStore>,
    pub(crate) engine_view: Option<&'a mut EngineView>,
    pub(crate) pane_state:
        Option<&'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>>,
    pub(crate) pane_jumps: Option<&'a mut SecondaryMap<PaneId, JumpList>>,
}
