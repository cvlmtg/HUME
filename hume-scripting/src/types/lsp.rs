//! The LSP effects and queries a script hands the editor: server
//! registrations and lifecycle ops, requests and their routing and params,
//! and trigger-char sets.

use hume_core::{
    BufferId, ConfigOrigin, ListEntry, ListTier, LspFeature, LspServerConfig, PaneId, ServerName,
};

/// What `(lsp-capability …)` asks a server for: the capability of a feature,
/// or the one a request method needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityQuery<'a> {
    Feature(LspFeature),
    Method(&'a str),
}

/// One `(register-lsp-server! …)` call queued for the end-of-eval drain.
///
/// `config`'s `init_options`/`settings` are decoded at the Steel boundary via
/// `crate::json::steel_to_params`: Steel data structures in, real JSON out.
#[derive(Debug)]
pub struct PendingLspServerReg {
    /// The registration's identity: re-registering a name replaces its
    /// config, and `unregister-lsp-server!`/`lsp-stop!`/`lsp-restart!` name it.
    pub name: ServerName,
    pub config: LspServerConfig,
    /// Whose code registered it: a registration never replaces one from a
    /// higher origin ([`ConfigOrigin::rank`]).
    pub origin: ConfigOrigin,
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
/// stay ordered because they're both entries in the same [`Effect`](super::Effect) log.
#[derive(Debug)]
pub enum PendingLspServerOp {
    Register(PendingLspServerReg),
    /// Removes `name`'s registration, unless a higher origin made it.
    Unregister {
        name: ServerName,
        origin: ConfigOrigin,
    },
    /// Sets (`Some`) or clears (`None`) `language`'s `tier` server list from
    /// `origin`. The list from the highest origin that set one in the
    /// highest tier that has one applies.
    SetLanguageServers {
        language: String,
        tier: ListTier,
        origin: ConfigOrigin,
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
    /// Steel reads it as the symbol [`hume_lsp::client::ServerState::name`]
    /// spells.
    pub state: hume_lsp::client::ServerState,
    pub pending: usize,
    pub encoding: hume_rope::position_encoding::PositionEncoding,
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
        buffer: BufferId,
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
    pub callback: crate::Callback,
    pub allow_stale: bool,
    /// `#:supersede`: a new request under the same key cancels the
    /// previous one still in flight under it, on every server it went to.
    /// An explicit opt-in, so two features issuing the same method never
    /// cancel each other.
    pub supersede: Option<crate::host::SupersedeKey>,
    /// `#:require-focus`: the pane the request was made from, if the
    /// callback should fire only while that pane is still focused when the
    /// answers arrive. `None` for a background request (formatting, rename,
    /// completion), which delivers regardless of focus. The decode refuses
    /// `#:require-focus` with no pane.
    pub require_focus: Option<PaneId>,
    /// `#:tracked`: the request's hold on a tracked position, which releases
    /// it once dropped, when the callback has run or will never run, unless
    /// the callback kept it.
    pub tracked: Option<crate::host::TrackedHold>,
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
