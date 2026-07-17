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

/// Language identity registration queued by `(define-language! …)`, applied
/// via `Editor::apply_pending_language_regs` as part of `Effect::LanguageReg`
/// application (`Editor::apply_script_effects`).
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

/// One `(register-lsp-server! …)` call queued for the end-of-eval drain.
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

/// An LSP server registration, unregistration, stop/restart, or status-view
/// request queued during any eval (init.scm, plugin activation, or a
/// command/hook body) and applied — in order — by
/// `Editor::apply_lsp_server_op` as part of `Effect::LspServerOp` application.
///
/// `Stop`/`Restart`/`ShowStatus` ride the same op enum as `Register`/
/// `Unregister` because they too need `&mut Editor`, which the Steel-eval-time
/// `EditorHost` impl doesn't hold — a reinstall's `Unregister` then `Register`
/// stay ordered because they're both entries in the same [`Effect`] log.
#[derive(Debug)]
pub enum PendingLspServerOp {
    Register(PendingLspServerReg),
    Unregister { language: String },
    Stop { language: Option<String> },
    Restart { language: Option<String> },
    ShowStatus,
}

/// One entry of `(lsp-server-status)` — mirrors `:lsp-status`'s data
/// (`Editor::lsp_status_text`) in structured form for Steel.
#[derive(Debug, Clone)]
pub struct LspServerStatusEntry {
    pub language: String,
    pub root: std::path::PathBuf,
    /// `LspClient::state`'s `Debug` spelling (`"Running"`, `"Starting"`, …) —
    /// the trait boundary stays free of a `hume-lsp` dependency.
    pub state: String,
    pub pending: usize,
}

/// `(lsp-request server method params callback #:allow-stale bool)` calls
/// queued during a command, hook, or queued-Steel-call eval and sent by
/// `Editor::send_one_lsp_request` as part of `Effect::LspRequest` application.
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
    /// If `Some(key)`, the bridge cancels the caller's own previous
    /// still-pending request filed under `(server, key)` before sending
    /// this one — an explicit opt-in, not automatic by method/buffer, so
    /// two features issuing the same method concurrently never cancel each
    /// other by accident.
    pub supersede: Option<String>,
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
            .field("supersede", &self.supersede)
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
    pub effects: Vec<Effect>,
}

/// One side effect queued by a Steel builtin during an eval — a mutation
/// that needs `&mut Editor` state the Steel-eval-time `EditorHost` doesn't
/// hold, so it's logged instead of applied inline.
///
/// Every eval entry point ([`super::ScriptingHost::call_steel_cmd`],
/// [`super::ScriptingHost::fire_hook`], [`super::ScriptingHost::run_steel_calls`],
/// [`super::ScriptingHost::eval_init`], [`super::ScriptingHost::activate_plugin_inline`])
/// returns the effects it queued, in the exact order Steel builtins pushed
/// them (`SteelCtx::effects`, backed by the persistent `ScriptingHost::effects`
/// log). The editor applies them in that same order — a single ordered log,
/// not five separate channels with a hardcoded apply order.
#[derive(Debug)]
pub enum Effect {
    LanguageReg(PendingLanguageReg),
    LspServerOp(PendingLspServerOp),
    SetBufferLanguage {
        buffer: BufferId,
        language: Option<String>,
    },
    /// A language name for which `(register-grammar! …)` just attached a
    /// grammar in command mode; the executor sweeps open buffers of that
    /// language (and buffers with injection sites) via
    /// `sweep_buffers_for_grammars`.
    GrammarSweep(String),
    LspRequest(PendingLspRequest),
    LspNotify(PendingLspNotify),
    /// `(bind-key! …)` / `(bind-key-extend! …)` — applied via
    /// `Keymap::bind_user_with_extend`.
    ///
    /// Queued rather than applied inline through a host capability so a failed
    /// plugin activation's binds are *never applied*: `pop_effect_marks(false)`
    /// drops them with everything else the failed body queued, so there is no
    /// ledger to keep and no unbind pass to run — and a bind that would have
    /// shadowed an existing one leaves it untouched, since nothing was ever
    /// overwritten. Mode and key-sequence validation still fails synchronously
    /// inside the builtin.
    BindKey {
        mode: crate::host::BindMode,
        keys: Vec<crossterm::event::KeyEvent>,
        cmd: String,
        force_extend: bool,
    },
    /// `(bind-wait-char! …)` — applied via `Keymap::bind_wait_char_user`.
    ///
    /// Separate from [`Effect::BindKey`] rather than a flag on it: a WaitChar
    /// node has no `force_extend` notion, so merging the two would make an
    /// illegal state representable.
    BindWaitChar {
        mode: crate::host::BindMode,
        keys: Vec<crossterm::event::KeyEvent>,
        cmd: String,
    },
    /// `(unbind-key! …)` — applied via `Keymap::unbind_user`. Queued like the
    /// three binders above so a same-eval bind-then-unbind on one key applies
    /// in Steel's emission order.
    UnbindKey {
        mode: crate::host::BindMode,
        keys: Vec<crossterm::event::KeyEvent>,
    },
}

/// One entry in the shared effect log (`ScriptingHost::effects`).
#[derive(Debug)]
pub(crate) struct QueuedEffect {
    pub(crate) effect: Effect,
    /// Set by `SteelCtx::pop_effect_marks(true)` when the plugin activation
    /// that queued this effect finishes successfully. Committed effects
    /// survive an enclosing eval's failure — see `ScriptingHost::take_eval_effects`.
    pub(crate) committed: bool,
}

/// A failed eval, carrying effects committed by nested successful plugin
/// activations (see `QueuedEffect`). Callers MUST apply `effects` (in order)
/// before reporting `message` — a committed activation's effects are
/// delivered regardless of the enclosing eval's fate.
#[derive(Debug)]
pub struct EvalError {
    pub message: String,
    pub effects: Vec<Effect>,
}

impl From<String> for EvalError {
    fn from(message: String) -> Self {
        Self {
            message,
            effects: Vec::new(),
        }
    }
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
