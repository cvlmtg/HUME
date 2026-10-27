use hume_core::{
    BindMode, BufferId, ConfigOrigin, GrammarSource, LanguageFields, LanguageIdentity, PaneId,
    Side, Tie,
};

use crate::command_procs::{MappableProc, TypedProc};

/// A buffer's language: a later write from one origin replaces the earlier,
/// whether the editor applies it or a script reads it back before then.
pub const BUFFER_LANGUAGE_TIE: Tie = Tie::LaterWins;

mod lsp;
pub use lsp::{
    CapabilityQuery, DocSpan, FeatureFilter, ListEntry, ListTier, LspFeature, LspFeatureSet,
    LspServerStatusEntry, LspServerTarget, Params, PendingLspNotify, PendingLspRequest,
    PendingLspServerOp, PendingLspServerReg, RequestMode, RequestParams, RouteSpec,
    SERVER_REGISTRATION_TIE, ServerName, TriggerKind, TriggerScope, WhenUnavailable,
};

/// A buffer paired with the pane it was invoked/observed through, if any:
/// the value injected everywhere command dispatch, hooks, completion
/// sources, and `(buffers)`/`(panes)` name a buffer, and the sole argument
/// shape every buffer-taking builtin decodes.
///
/// `pane` is `None` for a value with no pane of its own (a buffer-level hook
/// argument, `(buffers)`'s list, `open-buffer!`'s return). A builtin that
/// needs pane state (selections, viewport, focus) fails fast on `None`
/// rather than guessing one. A builtin that needs only the buffer
/// ([`Self::buffer`]) works the same either way. A text edit acts through
/// the pane when the value carries one and through no pane otherwise.
///
/// Private fields: the only mints are [`Self::with_pane`]/[`Self::buffer_only`],
/// so a value can't be assembled from a bid and an unrelated pid that never
/// actually showed it: every mint site already holds both halves together
/// (`FocusedPane::current`, a completion session's own `pane_id`, a resolved
/// dispatch target).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneHandle {
    buffer: BufferId,
    pane: Option<PaneId>,
}

impl PaneHandle {
    /// A buffer observed/invoked through a specific pane.
    pub fn with_pane(buffer: BufferId, pane: PaneId) -> Self {
        Self {
            buffer,
            pane: Some(pane),
        }
    }

    /// A buffer with no pane of its own. See this type's own doc for when
    /// that's the right mint.
    pub fn buffer_only(buffer: BufferId) -> Self {
        Self { buffer, pane: None }
    }

    pub fn buffer(self) -> BufferId {
        self.buffer
    }

    pub fn pane(self) -> Option<PaneId> {
        self.pane
    }
}

/// A Steel command definition built by `define-command!` during init or plugin load.
///
/// Passed immediately to [`crate::host::CommandHost::register_command`] so the
/// editor can insert a `SteelBacked` entry in its `CommandRegistry` inline, with no
/// deferred second pass after a successful eval.
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
    /// Mutually exclusive with `inline_output`, enforced at definition time.
    pub repeatable: bool,
    /// The lambda and the owner it was defined under, stored on the
    /// registry entry so dispatch runs the proc of the entry it resolved.
    pub proc: MappableProc,
}

/// A Steel typed-command definition built by `define-typed-command!` during
/// init or plugin load.
///
/// Passed immediately to [`crate::host::CommandHost::register_typed_command`]
/// so the editor can insert a typed `Command::Typed` entry in its
/// `CommandRegistry` inline, with no deferred second pass after a successful eval.
#[derive(Debug)]
pub struct SteelTypedCmdDef {
    pub name: String,
    pub doc: String,
    /// `#:complete`: the name of the completion source (`register-
    /// completion-source!`'s, or a native one such as `"path"`) that
    /// completes this command's `:` argument on Tab. Resolved by name at
    /// completion time, same as a built-in's declared completer.
    pub completer: Option<String>,
    /// What the editor stores on the registry entry and dispatches from.
    pub body: SteelTypedCmdBody,
}

/// The part of a `define-typed-command!` definition that only a Steel typed
/// command has.
///
/// No `repeatable` field: dot-repeat is meaningless for a `:` command, so
/// `define-typed-command!`'s Scheme wrapper declares no `#:repeatable`
/// keyword to carry one from (an unrecognized `#:key value` pair at the call
/// site is ignored by Steel's keyword-arg lambda syntax, not rejected:
/// there is no enforcement here beyond the absent keyword).
#[derive(Clone, Debug)]
pub struct SteelTypedCmdBody {
    /// Number of required positional parameters the lambda accepts (0, 1, or
    /// 2: `(arg)`/`(arg force)`). Introspected once at definition time from
    /// the closure's arity.
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
/// order via `Editor::apply_pending_language_regs` as part of
/// `Effect::LanguageReg` application (`Editor::apply_script_effects`).
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

/// Everything `(register-grammar! …)` supplies, in one payload.
///
/// An owned struct lets the init-mode effect and the command-mode host call
/// hand the language registry the same payload instead of two adapters that
/// must be kept identical. `SteelCmdDef` crosses the host boundary the same
/// way.
#[derive(Debug, Clone)]
pub struct GrammarReg {
    pub name: String,
    pub source: GrammarSource,
    /// Whose code registered it: it never replaces a grammar a higher
    /// origin registered ([`ConfigOrigin::rank`]).
    pub origin: ConfigOrigin,
}

/// One `(set-virtual-lines! …)` entry, decoded from its Steel hashmap shape
/// (`hume-scripting/src/builtins/decorations.rs`'s `virtual_line_specs`),
/// which only validates shape (arity, types): `segments` here are
/// **caller-supplied and unvalidated** char ranges, not yet sorted,
/// bounds-checked, or overlap-checked. `DecorationHost::set_virtual_lines`
/// (the host boundary) is the sole enforcement point: it sorts, validates
/// (bounds, ordering, non-overlap), widens edges inside a grapheme cluster to
/// the whole cluster, and converts to byte offsets before anything
/// downstream sees them.
#[derive(Debug, Clone)]
pub struct VirtualLineSpec {
    pub line: usize,
    pub text: String,
    /// `'anchor 'before` renders above `line`, the default `'after` below.
    pub side: Side,
    /// Whole-line base style; chars not covered by `segments` fall back to
    /// this (or `ui.virtual` if also absent).
    pub scope: Option<String>,
    /// `(char_start, char_end, scope_name)` into `text`, styling only the
    /// covered chars (unvalidated, see the struct doc). The host boundary
    /// converts these to the byte offsets `VirtualLineEntry` stores.
    pub segments: Vec<(usize, usize, String)>,
}

/// One side effect queued by a Steel builtin during an eval: a mutation
/// that needs `&mut Editor` state the Steel-eval-time `EditorHost` doesn't
/// hold, so it's logged instead of applied inline.
///
/// Every eval entry point ([`super::ScriptingHost::call_steel_cmd`],
/// [`super::ScriptingHost::fire_hook`], [`super::ScriptingHost::run_steel_calls`],
/// [`super::ScriptingHost::eval_init`], [`super::ScriptingHost::activate_plugin_inline`])
/// returns the effects it queued, in the exact order Steel builtins pushed
/// them (`SteelCtx::effects`, backed by the persistent `ScriptingHost::effects`
/// log). The editor applies them in that same order: a single ordered log,
/// not five separate channels with a hardcoded apply order.
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
    /// Queued rather than applied inline through a host capability so a failed
    /// plugin activation's binds are *never applied*: `pop_effect_marks(false)`
    /// drops them with everything else the failed body queued, so there is no
    /// ledger to keep and no unbind pass to run, and a bind that would have
    /// shadowed an existing one leaves it untouched, since nothing was ever
    /// overwritten. Mode and key-sequence validation still fails synchronously
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
    /// for `origin` and every lower origin. Queued like the two binders
    /// above so a same-eval bind-then-unbind on one key applies in Steel's
    /// emission order.
    UnbindKey {
        mode: BindMode,
        keys: Vec<termina::event::KeyEvent>,
        keymap: crate::host::KeymapTarget,
        origin: ConfigOrigin,
    },
    /// `(attach-keymap! pane name)`: the named keymap applies in `buffer`.
    /// Queued like the binds so a same-eval bind-then-attach applies in order.
    AttachKeymap {
        buffer: BufferId,
        name: String,
    },
    /// `(detach-keymap! pane name)`.
    DetachKeymap {
        buffer: BufferId,
        name: String,
    },
    /// `(register-completion-source! …)`: applied into the editor's
    /// completion source registry. Queued rather than applied inline through
    /// a host capability for exactly [`Effect::BindKey`]'s reason: a failed
    /// plugin activation's registration is never applied, so there is no
    /// owner ledger to keep and no unregister pass to run. Argument
    /// validation (a callable `proc`, a `#:target` that exists) still fails
    /// synchronously inside the builtin.
    RegisterCompletionSource(crate::host::PendingCompletionSource),
    /// `(set-hook-triggers! source language chars)`,
    /// `(set-completion-triggers! source language chars)` or their
    /// `set-attachment-…` forms: `source`'s trigger characters of `kind` in
    /// `scope`, replacing that triple's previous set; an empty `chars`
    /// removes it. A `Completion` set must name a registered `Buffer`
    /// source, checked when this applies. Queued alongside
    /// [`Effect::RegisterCompletionSource`] for the same ordering reason
    /// [`Effect::UnbindKey`] gives for the two binders it follows: a
    /// source registered earlier in the *same* eval must exist by the time
    /// this applies, and `ScriptingHost`'s effects are one ordered queue
    /// applied in emission order. Checking the registry synchronously
    /// would race a same-eval `register-completion-source!`, which only
    /// takes effect once the whole eval succeeds.
    SetTriggers {
        kind: TriggerKind,
        source: String,
        scope: TriggerScope,
        chars: Vec<char>,
    },
}

/// One entry in the shared effect log (`ScriptingHost::effects`).
#[derive(Debug)]
pub(crate) struct QueuedEffect {
    pub(crate) effect: Effect,
    /// Set by `SteelCtx::pop_effect_marks(true)` when the plugin activation
    /// that queued this effect finishes successfully. Committed effects
    /// survive an enclosing eval's failure (see `ScriptingHost::take_eval_effects`).
    pub(crate) committed: bool,
}

/// A failed eval, carrying effects committed by nested successful plugin
/// activations (see `QueuedEffect`). Callers MUST apply `effects` (in order)
/// before reporting `message`: a committed activation's effects are
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

#[cfg(test)]
mod tests;
