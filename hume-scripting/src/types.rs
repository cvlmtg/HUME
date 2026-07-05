use hume_engine::pipeline::BufferId;

/// A Steel command definition built by `define-command!` during init or plugin load.
///
/// Passed immediately to [`crate::host::EditorHost::register_command`] so the
/// editor can insert a `SteelBacked` entry in its `CommandRegistry` inline — no
/// deferred second pass after a successful eval.
#[derive(Debug)]
pub struct SteelCmdDef {
    pub name: String,
    pub doc: String,
    /// Number of required positional parameters the lambda accepts.
    /// Introspected once at `define-command!` time from the closure's arity.
    pub arity: u16,
    /// `true` if the lambda accepts a rest parameter (variadic).
    pub is_variadic: bool,
    /// `true` if dispatch should bracket this command with an alt-screen exit
    /// so subprocess output streams live to the terminal.
    pub inline_output: bool,
    /// `true` if pressing `.` should repeat this command.
    ///
    /// Opt in via `#:repeatable #t` in `(define-command! …)`.
    /// Mutually exclusive with `inline_output` — enforced at definition time.
    pub repeatable: bool,
}

/// Language identity registration queued during `eval_init` and flushed by
/// `Editor::flush_pending_language_regs` after each `eval_init` boundary.
#[derive(Debug)]
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
        injections_path: Option<std::path::PathBuf>,
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
