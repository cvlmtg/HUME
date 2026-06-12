use hume_engine::pipeline::BufferId;

/// A Steel command that has been fully registered in the Steel engine and is ready
/// to be inserted into the `CommandRegistry`.
///
/// Returned by [`super::ScriptingHost::eval_init`] and
/// [`super::ScriptingHost::activate_plugin`]; the editor layer registers
/// the commands after a successful eval.
#[derive(Debug)]
pub struct SteelCmdDef {
    pub name: String,
    pub doc: String,
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
pub type PendingLanguageSets = Vec<(BufferId, Option<String>)>;

/// Result returned by [`super::ScriptingHost::call_steel_cmd`].
#[derive(Debug)]
pub struct SteelCmdResult {
    pub wait_char_request: Option<String>,
    pub pending_language_sets: PendingLanguageSets,
    /// Language names for which `(register-grammar! …)` just attached a grammar;
    /// drained by the executor into `sweep_buffers_for_grammars`.
    pub grammar_sweeps: Vec<String>,
}

/// Result returned by [`super::ScriptingHost::fire_hook`].
#[derive(Debug)]
pub struct HookResult {
    pub pending_language_sets: PendingLanguageSets,
    pub grammar_sweeps: Vec<String>,
}

