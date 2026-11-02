use hume_core::{
    BindMode, BufferId, ConfigOrigin, GrammarSource, LanguageFields, LanguageIdentity, Side,
    TriggerListener,
};

use hume_rope::spanned::SpannedText;

use crate::command_procs::{MappableProc, TypedProc};

mod lsp;
pub use lsp::{
    CapabilityQuery, DocSpan, LspErrKind, LspRoute, LspServerStatusEntry, LspServerTarget, Params,
    PendingLspNotify, PendingLspRequest, PendingLspServerOp, PendingLspServerReg, RequestMode,
    RequestParams, RouteSpec, TriggerScope, WhenUnavailable, lsp_err_value,
};

/// A Steel command definition built by `define-command!` during init or
/// plugin load, passed straight to
/// [`crate::host::CommandHost::register_command`].
#[derive(Debug)]
pub struct SteelCmdDef {
    pub name: String,
    pub doc: String,
    /// What the editor stores on the registry entry and dispatches from.
    pub body: SteelCmdBody,
}

/// The part of a `define-command!` definition that only a Steel command has.
#[derive(Clone, Debug)]
pub struct SteelCmdBody {
    /// Number of required positional parameters the lambda accepts, read
    /// from its arity at definition.
    pub arity: u16,
    /// `true` if the lambda accepts a rest parameter (variadic).
    pub is_variadic: bool,
    /// `true` if dispatch should bracket this command with an alt-screen exit
    /// so subprocess output streams live to the terminal.
    pub inline_output: bool,
    /// `#:repeatable #t`: pressing `.` repeats this command. Exclusive with
    /// `inline_output`, checked at definition.
    pub repeatable: bool,
    /// The lambda and the owner it was defined under, stored on the
    /// registry entry so dispatch runs the proc of the entry it resolved.
    pub proc: MappableProc,
}

/// A Steel typed-command definition built by `define-typed-command!` during
/// init or plugin load, passed straight to
/// [`crate::host::CommandHost::register_typed_command`].
#[derive(Debug)]
pub struct SteelTypedCmdDef {
    pub name: String,
    pub doc: String,
    /// `#:complete`: the name of the completion source (one
    /// `register-completion-source!` registered, or a native one such as
    /// `"path"`) that completes this command's `:` argument on Tab, resolved
    /// by name at completion time.
    pub completer: Option<String>,
    /// What the editor stores on the registry entry and dispatches from.
    pub body: SteelTypedCmdBody,
}

/// The part of a `define-typed-command!` definition that only a Steel typed
/// command has. No `repeatable`: `.` does not repeat a `:` command, and
/// `define-typed-command!` declares no `#:repeatable` keyword.
#[derive(Clone, Debug)]
pub struct SteelTypedCmdBody {
    /// Number of required positional parameters the lambda accepts, at most
    /// 3: `(pane arg force)`, or a prefix of it.
    pub arity: u16,
    /// `true` if the lambda accepts a rest parameter (variadic).
    pub is_variadic: bool,
    /// `true` if dispatch should bracket this command with an alt-screen exit
    /// so subprocess output streams live to the terminal.
    pub inline_output: bool,
    /// The lambda and the owner it was defined under.
    pub proc: TypedProc,
}

/// Language registration queued by `(define-language! …)`,
/// `(configure-language! …)` or `(register-grammar! …)`, applied in queue
/// order as an [`Effect::LanguageReg`].
#[derive(Debug)]
pub enum PendingLanguageReg {
    Identity {
        name: String,
        identity: LanguageIdentity,
        origin: ConfigOrigin,
    },
    /// Per-language values; `None` leaves a field as it is.
    Configure {
        name: String,
        fields: LanguageFields,
        origin: ConfigOrigin,
    },
    Grammar(GrammarReg),
}

/// Everything `(register-grammar! …)` supplies, so the init-mode effect and
/// the command-mode host call hand the language registry one payload.
#[derive(Debug, Clone)]
pub struct GrammarReg {
    pub name: String,
    pub source: GrammarSource,
    /// Whose code registered it: it never replaces a grammar a higher
    /// origin registered ([`hume_core::Ranked::rank`]).
    pub origin: ConfigOrigin,
}

/// One `(set-virtual-lines! …)` entry but its line, decoded from its Steel
/// hashmap shape.
#[derive(Debug, Clone)]
pub struct VirtualLineSpec {
    /// The text, and the scope name styling each `'segments` part of it.
    pub content: SpannedText<String>,
    /// `'side 'before` renders above its line, the default `'after` below.
    pub side: Side,
    /// Whole-line base style; chars not covered by a segment fall back to
    /// this (or `ui.virtual` if also absent).
    pub scope: Option<String>,
}

/// A side effect a Steel builtin queues during an eval because it needs
/// `&mut Editor` state the eval-time `EditorHost` doesn't hold.
///
/// Every eval entry point ([`super::ScriptingHost::call_steel_cmd`],
/// [`super::ScriptingHost::fire_hook`], [`super::ScriptingHost::run_steel_calls`],
/// [`super::ScriptingHost::eval_init`], [`super::ScriptingHost::activate_plugin_inline`])
/// returns the effects it queued in the order builtins pushed them, and the
/// editor applies them in that order.
#[derive(Debug)]
pub enum Effect {
    LanguageReg(PendingLanguageReg),
    LspServerOp(PendingLspServerOp),
    /// `(set-buffer-option! pane "language" name)`: `None` clears the
    /// language. The editor resolves the name when the effect applies, and
    /// drops it when a higher origin asserted the buffer's language.
    SetBufferLanguage {
        buffer: BufferId,
        language: Option<String>,
        origin: ConfigOrigin,
    },
    /// A language name for which `(register-grammar! …)` just attached a
    /// grammar in command mode; the executor sweeps open buffers of that
    /// language (and buffers with injection sites) via
    /// `sweep_buffers_for_grammars`.
    GrammarSweep(String),
    LspRequest(PendingLspRequest),
    LspNotify(PendingLspNotify),
    /// `(bind-key! …)` / `(bind-key-extend! …)`: applied to the global keymap,
    /// or with `#:keymap` to that named keymap.
    ///
    /// Queued so a failed plugin activation's binds are never applied:
    /// `pop_effect_marks(false)` drops them with the rest of the failed
    /// body's effects, and a binding they would have shadowed stays
    /// untouched. Mode and key-sequence validation still fails synchronously
    /// inside the builtin.
    BindKey {
        mode: BindMode,
        keys: Vec<termina::event::KeyEvent>,
        cmd: String,
        force_extend: bool,
        keymap: crate::host::KeymapTarget,
        /// Whose code bound it: a higher origin's binding of the sequence
        /// stays in force.
        origin: ConfigOrigin,
    },
    /// `(unbind-key! …)`: unbinds a sequence in the keymap `keymap` names,
    /// for `origin` and every lower origin. Queued so a same-eval
    /// bind-then-unbind on one key applies in emission order.
    UnbindKey {
        mode: BindMode,
        keys: Vec<termina::event::KeyEvent>,
        keymap: crate::host::KeymapTarget,
        origin: ConfigOrigin,
    },
    /// `(attach-keymap! pane name)`: the named keymap applies in `buffer`.
    /// Queued so a same-eval bind-then-attach applies in order.
    AttachKeymap {
        buffer: BufferId,
        name: String,
    },
    /// `(detach-keymap! pane name)`.
    DetachKeymap {
        buffer: BufferId,
        name: String,
    },
    /// `(register-completion-source! …)`, queued so a failed plugin
    /// activation's registration is never applied, as [`Effect::BindKey`]
    /// is. Argument validation (a callable `proc`, a `#:target` that exists)
    /// still fails synchronously inside the builtin.
    RegisterCompletionSource(crate::host::PendingCompletionSource),
    /// `(set-hook-triggers! language source chars)`,
    /// `(set-completion-triggers! language source chars)` or their
    /// `set-attachment-…` forms: `listener`'s trigger characters in `scope`,
    /// replacing that pair's previous set; an empty `chars` removes it. A
    /// `Completion` listener must name a registered `Buffer` source, checked
    /// when this applies so that a `register-completion-source!` earlier in
    /// the same eval counts.
    SetTriggers {
        listener: TriggerListener,
        scope: TriggerScope,
        chars: Vec<char>,
    },
}

/// One entry in the shared effect log (`HostState::effects`).
#[derive(Debug)]
pub(crate) struct QueuedEffect {
    pub(crate) effect: Effect,
    /// Set by `SteelCtx::pop_effect_marks(true)` when the plugin activation
    /// that queued this effect succeeds. A committed effect survives an
    /// enclosing eval's failure.
    pub(crate) committed: bool,
}

/// A failed eval, carrying effects committed by nested successful plugin
/// activations. Callers must apply `effects`, in order, before reporting
/// `message`: a committed activation's effects are delivered whatever
/// happens to the enclosing eval.
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
