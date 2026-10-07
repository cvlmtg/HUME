use hume_engine::pipeline::{BufferId, PaneId};
use steel::rvals::SteelVal;

/// A buffer paired with the pane it was invoked/observed through, if any:
/// the value injected everywhere command dispatch, hooks, completion
/// sources, and `(buffers)`/`(panes)` name a buffer, and the sole argument
/// shape every buffer-taking builtin decodes.
///
/// `pane` is `None` for a value with no pane of its own (a buffer-level hook
/// argument, `(buffers)`'s list, `open-buffer!`'s return). A builtin that
/// needs pane state (selections, viewport, focus) fails fast on `None`
/// rather than guessing one. A builtin that needs only the buffer
/// ([`Self::buffer`]) works the same either way.
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
}

/// A Steel typed-command definition built by `define-typed-command!` during
/// init or plugin load.
///
/// Passed immediately to [`crate::host::CommandHost::register_typed_command`]
/// so the editor can insert a typed `Command::Typed` entry in its
/// `CommandRegistry` inline, with no deferred second pass after a successful eval.
/// No `repeatable` field: dot-repeat is meaningless for a `:` command, so
/// `define-typed-command!`'s Scheme wrapper declares no `#:repeatable`
/// keyword to carry one from (an unrecognized `#:key value` pair at the call
/// site is silently ignored by Steel's keyword-arg lambda syntax, not
/// rejected: there is no enforcement here beyond the absent keyword).
#[derive(Debug)]
pub struct SteelTypedCmdDef {
    pub name: String,
    pub doc: String,
    /// Number of required positional parameters the lambda accepts (0, 1, or
    /// 2: `(arg)`/`(arg force)`). Introspected once at definition time from
    /// the closure's arity.
    pub arity: u16,
    /// `true` if the lambda accepts a rest parameter (variadic).
    pub is_variadic: bool,
    /// `true` if dispatch should bracket this command with an alt-screen exit
    /// so subprocess output streams live to the terminal.
    pub inline_output: bool,
    /// `#:complete`: the name of the completion source (`register-
    /// completion-source!`'s, or a native one such as `"path"`) that
    /// completes this command's `:` argument on Tab. Resolved by name at
    /// completion time, same as a built-in's declared completer.
    pub completer: Option<String>,
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
        lsp_language_id: Option<String>,
        roots: Vec<String>,
    },
    Grammar(GrammarReg),
}

/// Everything `(register-grammar! …)` supplies, in one payload.
///
/// A named struct rather than six fields inlined into the variant *and* six
/// parameters on [`crate::host::LanguageHost::attach_grammar`], because both
/// spellings feed the same registry call. An owned struct is what lets the
/// init-mode effect and the command-mode host call reach one shared
/// implementation instead of two adapters that must be kept identical.
/// `SteelCmdDef` already crosses the host boundary this way.
///
/// Lives here, not as `hume_treesitter`'s `QueryPaths`: `hume-scripting` has
/// no `hume-treesitter` dependency, so that type is not nameable at this
/// layer. The editor converts on the far side, in one place.
#[derive(Debug, Clone)]
pub struct GrammarReg {
    pub name: String,
    pub grammar_path: std::path::PathBuf,
    pub symbol: String,
    pub highlights_path: std::path::PathBuf,
    /// `None` when the language embeds nothing.
    pub injections_path: Option<std::path::PathBuf>,
    /// `None` when the language ships no structural text objects.
    pub textobjects_path: Option<std::path::PathBuf>,
}

/// One `(set-virtual-lines! …)` entry, decoded from its Steel hashmap shape
/// (`hume-scripting/src/builtins/decorations.rs`'s `virtual_line_specs`),
/// which only validates shape (arity, types): `segments` here are
/// **caller-supplied and unvalidated** char ranges, not yet sorted,
/// bounds-checked, or overlap-checked. `DecorationHost::set_virtual_lines`
/// (the host boundary) is the sole enforcement point: it sorts, validates
/// (bounds, ordering, non-overlap, grapheme-cluster alignment), and converts
/// to byte offsets before anything downstream sees them.
#[derive(Debug, Clone)]
pub struct VirtualLineSpec {
    pub line: usize,
    pub text: String,
    /// `true` for `'anchor 'before` (render above `line`), `false` for the
    /// default `'after`.
    pub before: bool,
    /// Whole-line base style; chars not covered by `segments` fall back to
    /// this (or `ui.virtual` if also absent).
    pub scope: Option<String>,
    /// `(char_start, char_end, scope_name)` into `text`, styling only the
    /// covered chars (unvalidated, see the struct doc). The host boundary
    /// converts these to the byte offsets `VirtualLineEntry` stores.
    pub segments: Vec<(usize, usize, String)>,
}

/// A language server's registration name: non-empty, no whitespace, so it
/// reads unambiguously as a `:lsp-stop`/`:lsp-restart` argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerName(std::sync::Arc<str>);

impl ServerName {
    /// `Err` names what is wrong with `name`, for a builtin to prefix with
    /// its own name.
    pub fn parse(name: &str) -> Result<Self, String> {
        if name.is_empty() {
            return Err("server name is empty".to_string());
        }
        if name.chars().any(char::is_whitespace) {
            return Err(format!("server name {name:?} contains whitespace"));
        }
        Ok(Self(std::sync::Arc::from(name)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ServerName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One macro list generates [`LspFeature`], its wire-name table and
/// [`LspFeature::ALL`], so the variants, the names and the full list cannot
/// drift apart.
macro_rules! lsp_features {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// A language-server capability requests are routed by: Helix's
        /// `language-servers` feature vocabulary, spelled the same way so a
        /// Helix `only-features`/`except-features` list carries over verbatim.
        /// Features HUME has no command for are still valid names.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum LspFeature {
            $($variant),+
        }

        impl LspFeature {
            pub const ALL: [LspFeature; [$(LspFeature::$variant),+].len()] =
                [$(LspFeature::$variant),+];

            /// Every `(wire name, feature)` pair, the table a feature
            /// argument is decoded through.
            pub const NAMED: [(&'static str, LspFeature); [$(LspFeature::$variant),+].len()] =
                [$(($name, LspFeature::$variant)),+];

            pub fn name(self) -> &'static str {
                match self {
                    $(LspFeature::$variant => $name),+
                }
            }
        }
    };
}

lsp_features! {
    Format => "format",
    GotoDeclaration => "goto-declaration",
    GotoDefinition => "goto-definition",
    GotoTypeDefinition => "goto-type-definition",
    GotoReference => "goto-reference",
    GotoImplementation => "goto-implementation",
    SignatureHelp => "signature-help",
    Hover => "hover",
    DocumentHighlight => "document-highlight",
    Completion => "completion",
    CodeAction => "code-action",
    DocumentLinks => "document-links",
    WorkspaceCommand => "workspace-command",
    DocumentSymbols => "document-symbols",
    WorkspaceSymbols => "workspace-symbols",
    Diagnostics => "diagnostics",
    PullDiagnostics => "pull-diagnostics",
    RenameSymbol => "rename-symbol",
    InlayHints => "inlay-hints",
    DocumentColors => "document-colors",
    CallHierarchy => "call-hierarchy",
}

/// What `(lsp-capability …)` asks a server for: the capability of a feature,
/// or the one a request method needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityQuery<'a> {
    Feature(LspFeature),
    Method(&'a str),
}

/// A set of [`LspFeature`]s, one bit per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspFeatureSet(u32);

impl LspFeatureSet {
    pub fn insert(&mut self, feature: LspFeature) {
        self.0 |= 1 << feature as u32;
    }

    pub fn contains(self, feature: LspFeature) -> bool {
        self.0 & (1 << feature as u32) != 0
    }

    /// Members in [`LspFeature::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = LspFeature> {
        LspFeature::ALL
            .into_iter()
            .filter(move |f| self.contains(*f))
    }
}

impl FromIterator<LspFeature> for LspFeatureSet {
    fn from_iter<I: IntoIterator<Item = LspFeature>>(iter: I) -> Self {
        let mut set = Self::default();
        for feature in iter {
            set.insert(feature);
        }
        set
    }
}

/// Which features a server may be asked for within one language's server
/// list: Helix's `only-features`/`except-features`, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureFilter {
    All,
    Only(LspFeatureSet),
    Except(LspFeatureSet),
}

impl FeatureFilter {
    pub fn admits(self, feature: LspFeature) -> bool {
        match self {
            FeatureFilter::All => true,
            FeatureFilter::Only(set) => set.contains(feature),
            FeatureFilter::Except(set) => !set.contains(feature),
        }
    }
}

/// One entry of a language's ordered server list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: ServerName,
    pub filter: FeatureFilter,
}

/// Which of a language's two server lists a call writes. A `User` list
/// always wins over a `Default` one, whichever was set first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListLayer {
    User,
    Default,
}

/// One `(register-lsp-server! …)` call queued for the end-of-eval drain.
///
/// `init_options`/`settings` are decoded at the Steel boundary via
/// `crate::json::steel_to_params`: Steel data structures in, real JSON out.
#[derive(Debug)]
pub struct PendingLspServerReg {
    /// The registration's identity: re-registering a name replaces its
    /// config, and `unregister-lsp-server!`/`lsp-stop!`/`lsp-restart!` name it.
    pub name: ServerName,
    pub command: String,
    pub args: Vec<String>,
    pub init_options: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
    /// `#:env`: extra environment variables applied additively (never
    /// clearing the inherited environment) when the server process spawns.
    pub env: Vec<(String, String)>,
}

/// `(lsp-stop! target)` / `(lsp-restart! target)`'s target: every server
/// attached to one buffer, or every running instance of a registration
/// name. A caller names the buffer explicitly, so a stop/restart queued
/// from a hook or callback isn't at the mercy of whatever buffer happens to
/// be focused when the effect log drains.
#[derive(Debug)]
pub enum LspServerTarget {
    Buffer(BufferId),
    Name(ServerName),
}

/// An LSP server registration, unregistration, stop/restart, or status-view
/// request queued during any eval (init.scm, plugin activation, or a
/// command/hook body) and applied, in order, by
/// `Editor::apply_lsp_server_ops` as part of `Effect::LspServerOp` application.
///
/// `Stop`/`Restart`/`ShowStatus` ride the same op enum as `Register`/
/// `Unregister` because they too need `&mut Editor`, which the Steel-eval-time
/// `EditorHost` impl doesn't hold. A reinstall's `Unregister` then `Register`
/// stay ordered because they're both entries in the same [`Effect`] log.
#[derive(Debug)]
pub enum PendingLspServerOp {
    Register(PendingLspServerReg),
    Unregister {
        name: ServerName,
    },
    /// Sets (`Some`) or clears (`None`) one of `language`'s server lists.
    SetLanguageServers {
        language: String,
        layer: ListLayer,
        entries: Option<Vec<ListEntry>>,
    },
    Stop {
        target: LspServerTarget,
    },
    Restart {
        target: LspServerTarget,
    },
    ShowStatus,
}

/// One entry of `(lsp-server-status)`, mirroring `:lsp-status`'s data
/// (`Editor::lsp_status_text`) in structured form for Steel.
#[derive(Debug, Clone)]
pub struct LspServerStatusEntry {
    pub name: ServerName,
    pub languages: Vec<String>,
    pub root: std::path::PathBuf,
    /// `ServerState::name`'s lowercase spelling (`"running"`, `"starting"`,
    /// …), the symbol Steel receives.
    pub state: &'static str,
    pub pending: usize,
    pub encoding: hume_rope::position_encoding::PositionEncoding,
}

/// Which listeners a trigger-char set is for: `on-trigger-char` hooks named
/// by the source, or the completion source of that name, which a typed
/// character invokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerKind {
    Hook,
    Completion,
}

/// Where a trigger-char set applies: the buffers of a language, or one
/// buffer's attachment to one server for one feature, whose entries go when
/// that buffer detaches from the server. An attachment set applies only
/// while the attachment's list entry admits `feature` and the server
/// advertises it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerScope {
    Language(String),
    Attachment {
        buffer: hume_engine::pipeline::BufferId,
        server: hume_lsp::backend::ServerId,
        feature: LspFeature,
    },
}

/// Which of a buffer's attached servers a request or notification may go
/// to, beyond what its method implies. Routing happens when the request is
/// sent, among the servers attached and `Running` then: `feature` keeps the
/// servers whose list entry admits it and whose capabilities advertise it,
/// and is given only for a method with no standard feature of its own, and
/// `to` names the one server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteSpec {
    pub feature: Option<LspFeature>,
    pub to: Option<crate::ServerRef>,
}

/// A request's params: one value every routed server receives, or one
/// value per named server (`lsp-request-all!` with `(server . params)`
/// pairs, each server kept whole so its result names it even after it
/// stopped). A `DocPos`/`DocRange` inside stays unencoded until the request
/// is serialized for each server, in that server's position encoding.
#[derive(Debug)]
pub enum RequestParams {
    Shared(Params),
    PerServer(Vec<(crate::ServerRef, Params)>),
}

/// A buffer position or range in request params, which each server receives
/// in its own position encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocSpan {
    Pos(crate::DocPos),
    Range(crate::DocRange),
}

/// The params of a request or notification, converted from the Steel value
/// the caller built. A subtree holding no position is already its JSON, the
/// same for every server; a [`DocSpan`] is encoded per server by
/// [`Params::render`].
#[derive(Debug, Clone, PartialEq)]
pub enum Params {
    Json(serde_json::Value),
    Doc(DocSpan),
    Array(Vec<Params>),
    Object(Vec<(String, Params)>),
}

impl Params {
    /// An array of `items`, collapsed to plain JSON when none holds a position.
    pub fn array(items: Vec<Params>) -> Self {
        if items.iter().all(|item| matches!(item, Params::Json(_))) {
            Params::Json(serde_json::Value::Array(
                items.into_iter().filter_map(Params::into_json).collect(),
            ))
        } else {
            Params::Array(items)
        }
    }

    /// An object of `entries`, collapsed to plain JSON when none holds a position.
    pub fn object(entries: Vec<(String, Params)>) -> Self {
        if entries
            .iter()
            .all(|(_, value)| matches!(value, Params::Json(_)))
        {
            Params::Json(serde_json::Value::Object(
                entries
                    .into_iter()
                    .filter_map(|(key, value)| value.into_json().map(|json| (key, json)))
                    .collect(),
            ))
        } else {
            Params::Object(entries)
        }
    }

    fn into_json(self) -> Option<serde_json::Value> {
        match self {
            Params::Json(value) => Some(value),
            _ => None,
        }
    }

    /// These params as JSON, with each position replaced by what `encode`
    /// returns for it. `encode`'s first error fails the whole render.
    pub fn render(
        &self,
        encode: &mut dyn FnMut(DocSpan) -> Result<serde_json::Value, String>,
    ) -> Result<serde_json::Value, String> {
        match self {
            Params::Json(value) => Ok(value.clone()),
            Params::Doc(span) => encode(*span),
            Params::Array(items) => items
                .iter()
                .map(|item| item.render(encode))
                .collect::<Result<Vec<_>, _>>()
                .map(serde_json::Value::Array),
            Params::Object(entries) => entries
                .iter()
                .map(|(key, value)| Ok((key.clone(), value.render(encode)?)))
                .collect::<Result<serde_json::Map<_, _>, String>>()
                .map(serde_json::Value::Object),
        }
    }
}

/// How many of the routed servers a request goes to, and so how its
/// callback is called: `Single` sends to the first and calls back with
/// `(err result)`, `All` sends to every one and calls back once with
/// `(err results)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestMode {
    Single,
    All,
}

impl RequestMode {
    /// The Steel builtin that queues a request in this mode.
    pub fn verb(self) -> &'static str {
        match self {
            RequestMode::Single => "lsp-request!",
            RequestMode::All => "lsp-request-all!",
        }
    }
}

/// What a request's callback receives when no server can take it:
/// `Error` an `'unavailable` error saying why, `Empty` the answer of a
/// server with nothing to give (void, or `'()` for `lsp-request-all!`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhenUnavailable {
    #[default]
    Error,
    Empty,
}

impl WhenUnavailable {
    /// `(spelling, value)` pairs for decoding `#:unavailable`.
    pub const NAMED: [(&'static str, WhenUnavailable); 2] = [
        ("error", WhenUnavailable::Error),
        ("empty", WhenUnavailable::Empty),
    ];
}

/// An `lsp-request!`/`lsp-request-all!` call queued during an eval and sent
/// by the editor when the eval's effects apply. `bid` names the buffer the
/// request is about, never live focus, so a follow-up request from a
/// response callback reaches the same buffer. `callback` is the raw Steel
/// closure, called once through the queued-Steel-call mechanism when every
/// server it went to has answered, failed, or timed out.
pub struct PendingLspRequest {
    pub bid: BufferId,
    pub method: String,
    pub params: RequestParams,
    pub mode: RequestMode,
    pub route: RouteSpec,
    pub when_unavailable: WhenUnavailable,
    pub callback: SteelVal,
    pub allow_stale: bool,
    /// `#:supersede`: a new request under the same key cancels the
    /// previous one still in flight under it, on every server it went to.
    /// An explicit opt-in, so two features issuing the same method never
    /// cancel each other.
    pub supersede: Option<String>,
    /// `#:require-focus`: the pane the request was made from, if the
    /// callback should fire only while that pane is still focused when the
    /// answers arrive. `None` for a background request (formatting, rename,
    /// completion), which delivers regardless of focus. The decode refuses
    /// `#:require-focus` with no pane.
    pub require_focus: Option<PaneId>,
    /// `#:tracked`: a tracked position this request holds, released once its
    /// callback has run or will never run, unless the callback kept it.
    pub tracked: Option<crate::host::HostToken>,
}

// Manual (not derived): the callback prints as a placeholder instead of
// its closure.
impl std::fmt::Debug for PendingLspRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingLspRequest")
            .field("bid", &self.bid)
            .field("method", &self.method)
            .field("params", &self.params)
            .field("mode", &self.mode)
            .field("route", &self.route)
            .field("callback", &"<closure>")
            .field("when_unavailable", &self.when_unavailable)
            .field("allow_stale", &self.allow_stale)
            .field("supersede", &self.supersede)
            .field("require_focus", &self.require_focus)
            .field("tracked", &self.tracked)
            .finish()
    }
}

/// An `lsp-notify!` call, queued the same way as [`PendingLspRequest`]:
/// sent to every server `route` admits, with no answer to wait for.
#[derive(Debug)]
pub struct PendingLspNotify {
    pub bid: BufferId,
    pub method: String,
    pub params: Params,
    pub route: RouteSpec,
}

/// Result returned by [`super::ScriptingHost::call_steel_cmd`].
#[derive(Debug)]
pub struct SteelCmdResult {
    pub wait_char_request: Option<String>,
    pub effects: Vec<Effect>,
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
    /// language. The editor resolves the name when the effect applies.
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
    /// `(bind-key! …)` / `(bind-key-extend! …)`: applied via
    /// `Keymap::bind_user_with_extend`.
    ///
    /// Queued rather than applied inline through a host capability so a failed
    /// plugin activation's binds are *never applied*: `pop_effect_marks(false)`
    /// drops them with everything else the failed body queued, so there is no
    /// ledger to keep and no unbind pass to run, and a bind that would have
    /// shadowed an existing one leaves it untouched, since nothing was ever
    /// overwritten. Mode and key-sequence validation still fails synchronously
    /// inside the builtin.
    BindKey {
        mode: crate::host::BindMode,
        keys: Vec<termina::event::KeyEvent>,
        cmd: String,
        force_extend: bool,
    },
    /// `(bind-wait-char! …)`: applied via `Keymap::bind_wait_char_user`.
    ///
    /// Separate from [`Effect::BindKey`] rather than a flag on it: a WaitChar
    /// node has no `force_extend` notion, so merging the two would make an
    /// illegal state representable.
    BindWaitChar {
        mode: crate::host::BindMode,
        keys: Vec<termina::event::KeyEvent>,
        cmd: String,
    },
    /// `(unbind-key! …)`: applied via `Keymap::unbind_user`. Queued like the
    /// three binders above so a same-eval bind-then-unbind on one key applies
    /// in Steel's emission order.
    UnbindKey {
        mode: crate::host::BindMode,
        keys: Vec<termina::event::KeyEvent>,
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
    /// [`Effect::UnbindKey`] gives for the three binders it follows: a
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
