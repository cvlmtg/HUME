use steel::rvals::SteelVal;

use engine::pipeline::BufferId;

use super::attribution;
/// A `(define-command! …)` call captured during `eval_init`, to be processed
/// after the eval completes.
pub struct PendingSteelCmd {
    pub name: String,
    pub doc: String,
    /// The Steel lambda, captured at `define-command!` call time.
    pub proc: SteelVal,
    /// Attribution owner at call time — stored in `cmd_owners` for `(command-plugin …)`.
    pub(crate) current_owner: attribution::Owner,
    /// Whether this command participates in sticky-Ctrl extend.
    /// Set by `(define-command-extend! …)`.
    pub extendable: bool,
    /// Whether dispatch brackets the call with an alt-screen exit for live
    /// subprocess output. Set by `(define-command-inline-output! …)`.
    pub inline_output: bool,
}

/// A Steel command that has been fully registered in the Steel engine and is ready
/// to be inserted into the `CommandRegistry`.
///
/// Returned by [`super::ScriptingHost::eval_init`] and
/// [`super::ScriptingHost::activate_plugin`]; the editor layer registers
/// the commands after a successful eval.
pub struct SteelCmdDef {
    pub name: String,
    pub doc: String,
    /// Name under which the lambda is bound in Steel's global namespace
    /// (e.g. `"%hume-cmd-my-command"`).  Used by `ScriptingHost::call_steel_cmd`
    /// at dispatch time.
    pub steel_proc: String,
    pub extendable: bool,
    /// Number of required positional parameters the lambda accepts.
    /// Introspected once at `define-command!` time from the closure's arity.
    pub arity: u16,
    /// `true` if the lambda accepts a rest parameter (variadic).
    pub is_variadic: bool,
    /// `true` if dispatch should bracket this command with an alt-screen exit
    /// so subprocess output streams live to the terminal.
    pub inline_output: bool,
}

/// Language identity registration queued during `eval_init` and flushed by
/// `Editor::flush_pending_language_regs` after each `eval_init` boundary.
pub enum PendingLanguageReg {
    Identity {
        name: String,
        extensions: Vec<String>,
        globs: Vec<String>,
        shebangs: Vec<String>,
    },
    Grammar {
        name: String,
        grammar_path: std::path::PathBuf,
        symbol: String,
        highlights_path: std::path::PathBuf,
    },
}

/// `set-buffer-language!` calls deferred during a command or hook eval.
/// Each entry is `(buffer_id, language_name_or_none)`.
/// Drained by consumers (mappings.rs, fire_hook_silent) BEFORE cmd_queue.
pub type PendingLanguageSets = Vec<(BufferId, Option<String>)>;

/// A command queued by `(call! …)` inside a Steel command body.
///
/// `register` captures the sticky prefix active when `(call! …)` was reached
/// (set via `(set-register-prefix! …)`).  `None` means no register was
/// active at enqueue time, so the editor's existing `register_prefix` is left
/// untouched when this entry is dispatched.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedCommand {
    pub name: String,
    pub args: Vec<SteelVal>,
    pub register: Option<char>,
}

/// Result returned by [`super::ScriptingHost::call_steel_cmd`].
#[derive(Debug)]
pub struct SteelCmdResult {
    pub cmd_queue: Vec<QueuedCommand>,
    pub wait_char_request: Option<String>,
    pub pending_language_sets: PendingLanguageSets,
    /// Language names for which `(register-grammar! …)` just attached a grammar;
    /// drained by the executor into `sweep_buffers_for_grammars`.
    pub grammar_sweeps: Vec<String>,
}

/// Result returned by [`super::ScriptingHost::fire_hook`].
#[derive(Debug)]
pub struct HookResult {
    pub cmd_queue: Vec<QueuedCommand>,
    pub pending_language_sets: PendingLanguageSets,
    pub grammar_sweeps: Vec<String>,
}

