use hume_engine::pipeline::BufferId;
use steel::rvals::SteelVal;

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

/// LSP server registration queued during `eval_init` and flushed by
/// `Editor::flush_pending_lsp_server_regs` after init.scm finishes.
///
/// `init_options`/`settings` are decoded at the Steel boundary via
/// [`crate::json::steel_to_json`] — Steel data structures in, real JSON out.
#[derive(Debug)]
pub struct PendingLspServerReg {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
    pub root_markers: Vec<String>,
    pub init_options: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
}

/// `set-buffer-language!` calls deferred during a command or hook eval.
/// Each entry is `(buffer_id, language_name_or_none)`.
pub type PendingLanguageSets = Vec<(BufferId, Option<String>)>;

/// `(lsp-request server method params callback #:allow-stale bool)` calls
/// queued during a command, hook, or queued-Steel-call eval and flushed by
/// `Editor::flush_pending_lsp_requests` right after, mirroring
/// `pending_language_sets`'s per-eval drain (not `PendingLspServerReg`'s
/// once-per-init-boundary queue — a request can be sent at any eval, not
/// just init.scm's top level).
///
/// `server` is a registered language name, or `None` for "the focused
/// buffer's attached server". `params` is already decoded to JSON via
/// [`crate::json::steel_to_json`]; `callback` is the raw Steel closure,
/// delivered `(err result)` through the queued-Steel-call mechanism once the
/// response (or timeout) arrives.
pub struct PendingLspRequest {
    pub server: Option<String>,
    pub method: String,
    pub params: serde_json::Value,
    pub callback: SteelVal,
    pub allow_stale: bool,
}

// Manual (not derived): `SteelVal` has no `Debug` impl. Placeholder the
// closure — everything else is real data, still useful in a panic message.
impl std::fmt::Debug for PendingLspRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingLspRequest")
            .field("server", &self.server)
            .field("method", &self.method)
            .field("params", &self.params)
            .field("callback", &"<closure>")
            .field("allow_stale", &self.allow_stale)
            .finish()
    }
}

/// `(lsp-notify server method params)` calls queued the same way as
/// [`PendingLspRequest`], minus the callback — notifications get no response.
#[derive(Debug)]
pub struct PendingLspNotify {
    pub server: Option<String>,
    pub method: String,
    pub params: serde_json::Value,
}

/// Result returned by [`super::ScriptingHost::call_steel_cmd`].
#[derive(Debug)]
pub struct SteelCmdResult {
    pub wait_char_request: Option<String>,
    pub pending_language_sets: PendingLanguageSets,
    /// Language names for which `(register-grammar! …)` just attached a grammar;
    /// drained by the executor into `sweep_buffers_for_grammars`.
    pub grammar_sweeps: Vec<String>,
    pub pending_lsp_requests: Vec<PendingLspRequest>,
    pub pending_lsp_notifies: Vec<PendingLspNotify>,
}

/// Result returned by [`super::ScriptingHost::fire_hook`] and
/// [`super::ScriptingHost::run_steel_calls`].
#[derive(Debug)]
pub struct HookResult {
    pub pending_language_sets: PendingLanguageSets,
    pub grammar_sweeps: Vec<String>,
    pub pending_lsp_requests: Vec<PendingLspRequest>,
    pub pending_lsp_notifies: Vec<PendingLspNotify>,
}
