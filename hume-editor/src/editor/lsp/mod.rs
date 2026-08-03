//! Editor-side LSP state: holds the backend and per-server client state,
//! and drains events at frame cadence. Wires the backend + `AsyncSource`
//! plumbing, per-client lifecycle state, request/callback bookkeeping and
//! server->client dispatch (`drain.rs`), document sync, diagnostics,
//! registration, and observability commands.

mod bridge;
pub(crate) mod completion;
pub(crate) mod diagnostics;
mod drain;
pub(crate) mod edits;
pub(crate) mod introspect;
mod progress;
mod registry;
pub(crate) mod sync;

#[cfg(test)]
use std::path::PathBuf;
use std::time::Instant;

use rustc_hash::FxHashMap;

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::{LspBackend, ServerId, ThreadedLspBackend};
use hume_lsp::client::{LspClient, Outcome, RequestMeta, ServerState};
use hume_lsp::codec::RequestId;
#[cfg(test)]
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::transport::WakeCallback;

use super::Editor;
use super::async_source::AsyncSource;
use diagnostics::{DiagSeverity, DiagnosticsStore, StoredDiag};
use progress::{ProgressTask, SpinnerClock};
use registry::{LanguageName, LspServerConfig};

/// A Rust closure run with a completed request's outcome. `hume-lsp` never
/// holds this — it only ever sees the `(ServerId, RequestId)` pair the
/// editor keys its callback under, which `hume-lsp` already hands back from
/// `send_request`/`take_completed`/`drain_pending`.
pub(crate) type LspCallback = Box<dyn FnOnce(&mut Editor, Outcome)>;

struct CallbackEntry {
    callback: LspCallback,
    /// If `Some((bid, text_gen))` and the buffer has moved past `text_gen`
    /// by drain time, the outcome is dropped silently unless the request's
    /// `allow_stale` opts out — the parse-worker staleness discipline.
    stale_check: Option<(BufferId, u64)>,
}

/// Everything tracked per running (or starting) LSP server, one entry per
/// `ServerId` — single source of truth, no separate (language, root) index:
/// `client.root()` already carries the workspace root, so an attach/resolve
/// lookup scans `LspState.servers` (at most a handful of entries running at
/// once) instead of maintaining a second map that could drift out of sync
/// with this one.
struct ServerEntry {
    client: LspClient,
    /// The language this server was registered under
    /// (`register-lsp-server!`'s key) — `None` only for a client inserted
    /// directly by a test without going through `lsp_attach_buffer`.
    language: Option<LanguageName>,
    /// Display name (the registered `command`, e.g. `"rust-analyzer"`) —
    /// used to prefix stderr/log lines so `:messages` reads legibly
    /// with multiple servers running.
    name: String,
    /// Decoded `ServerCapabilities`, cached once at handshake completion
    /// (`dispatch_lsp_action`'s `BecameRunning` arm) — the
    /// `(lsp-capabilities …)` builtin reads this rather than reconverting the typed
    /// caps on every call.
    capabilities_json: Option<serde_json::Value>,
    /// Active `$/progress` tasks, in begin order — a server can run more
    /// than one concurrently (e.g. rust-analyzer indexing + a flycheck run).
    /// The statusline shows the most recent (last); a token is removed on
    /// its `end` notification. Empty ⇒ nothing to show for this server.
    progress: Vec<(String, ProgressTask)>,
}

pub(crate) struct LspState {
    backend: Box<dyn LspBackend>,
    servers: FxHashMap<ServerId, ServerEntry>,
    /// Keyed by the `(ServerId, RequestId)` pair a callback's own request
    /// was sent under — `drain_lsp` already has both in scope at dispatch
    /// time (the per-server loop, then the response/timeout's own id), so
    /// no separate token needs to be minted or round-tripped.
    callbacks: FxHashMap<(ServerId, RequestId), CallbackEntry>,
    /// Config recorded by `register-lsp-server!`, keyed by language.
    configs: FxHashMap<LanguageName, LspServerConfig>,
    diagnostics: DiagnosticsStore,
    /// Drives the statusline loading spinner's animation frame. Advanced
    /// (at most) once per `drain_lsp` call, gated on its own interval so
    /// the animation speed doesn't depend on the event loop's wake cadence.
    spinner: SpinnerClock,
    /// The LSP completion session — a singleton, starting a new one
    /// replaces the old. Lives here (not `EditorState`) so it dies with the
    /// LSP subsystem (`:lsp-stop` clears it via `lsp_stop_one`) rather than
    /// needing a second owner to reach across for that.
    pub(in crate::editor) completion: Option<completion::CompletionSession>,
    /// Insert-mode selection state for `completion` — separate from the
    /// session itself, cleared whenever the session ends.
    pub(in crate::editor) completion_ui: Option<completion::CompletionMenuUi>,
    /// `(server, supersede-key) -> the in-flight request id filed under that
    /// key` — for `lsp-request`'s `#:supersede` option: a new request under
    /// the same key cancels the previous one first. Entries are removed in
    /// `dispatch_completed` (response/timeout/crash-drain/stop-drain all
    /// funnel there) and swept per-server in `lsp_stop_one`, so an id can
    /// never linger past the request it names.
    supersede: FxHashMap<(ServerId, String), RequestId>,
}

impl LspState {
    /// Shared constructor body — every entry point differs only in which
    /// backend it plugs in.
    fn with_backend(backend: Box<dyn LspBackend>) -> Self {
        Self {
            backend,
            servers: FxHashMap::default(),
            callbacks: FxHashMap::default(),
            configs: FxHashMap::default(),
            diagnostics: DiagnosticsStore::default(),
            spinner: SpinnerClock::default(),
            completion: None,
            completion_ui: None,
            supersede: FxHashMap::default(),
        }
    }

    /// `true` while any server needs the statusline spinner animating —
    /// mid-handshake (`Starting`) or reporting `$/progress` (indexing,
    /// loading, ...). Single source of truth for the two sites that must
    /// agree: `AsyncSource::next_wake` (*when* to wake for the next spinner
    /// tick) and `drain_lsp` (*whether* to advance the frame once woken). If
    /// they diverged, the spinner would freeze or wake without advancing.
    pub(crate) fn has_animating_server(&self) -> bool {
        self.servers
            .values()
            .any(|e| e.client.state() == ServerState::Starting || !e.progress.is_empty())
    }

    /// Clears state `:reload-config`'s reset must not let survive — see
    /// `Editor::reset_config_state`. `callbacks` holds `Box<dyn FnOnce>`
    /// closures that capture `SteelVal`s from the outgoing engine
    /// (`lsp/bridge.rs`'s `lsp_callback`); `dispatch_completed` already
    /// tolerates a missing callback entry (early-returns, logging only for
    /// the internal fire-and-forget `shutdown` request), so dropping them
    /// here is safe. `configs` is the `register-lsp-server!` registration
    /// store the new `init.scm` re-populates. `completion`/`completion_ui`
    /// hold a session whose `on-completion-refilter` handler dies with the
    /// outgoing engine — leaving them would strand a menu that silently
    /// stops refetching; `supersede` is that session's in-flight-request
    /// index, meaningless once the session is gone. Deliberately *not*
    /// touching `servers`/`diagnostics`: an already-spawned process keeps
    /// running on its old config until `:lsp-restart`, and its last-known
    /// diagnostics are what `resync_config_state` replays, per `docs/LSP.md`.
    ///
    /// Every field above is named explicitly, not `..Self::with_backend(..)`
    /// struct-update syntax: `backend` is `Box<dyn LspBackend>`, which has no
    /// meaningful default to reconstruct against. A field added to this
    /// struct in the future needs an explicit line here (keep or clear) —
    /// there's no compiler nudge for that the way `ConfigState`'s wholesale
    /// rebuild gets one, so treat this list with the same suspicion as
    /// `EditorState`'s old field-by-field reset.
    pub(crate) fn reset_config(&mut self) {
        self.callbacks.clear();
        self.configs.clear();
        self.completion = None;
        self.completion_ui = None;
        self.supersede.clear();
    }

    /// Buffers currently attached to a `Running` server, paired with the
    /// server's registered language — the exact set `:reload-config`'s
    /// `resync_config_state` re-fires `OnLspAttach`/`OnDiagnosticsChanged`
    /// for. `Starting` is excluded: it fires its own `OnLspAttach` shortly
    /// after, from this same struct's `BecameRunning` handling in
    /// `dispatch_lsp_action`, and firing here too would double it. `Crashed`
    /// must not fire at all.
    pub(super) fn running_attached_buffers(
        &self,
        buffers: &crate::editor::BufferStore,
    ) -> Vec<(BufferId, LanguageName)> {
        buffers
            .iter()
            .filter_map(|(bid, buf)| {
                let entry = self.servers.get(&buf.lsp_server?)?;
                if entry.client.state() != ServerState::Running {
                    return None;
                }
                Some((bid, entry.language.clone()?))
            })
            .collect()
    }

    /// Every buffer with a cached diagnostic, from any server, alive or
    /// crashed — see `DiagnosticsStore::buffers_with_diagnostics`. Unlike
    /// `running_attached_buffers`, this is not filtered by server state:
    /// `:reload-config`'s resync uses it to replay `OnDiagnosticsChanged`
    /// from the surviving cache regardless of whether the server that
    /// published it is still `Running`.
    pub(super) fn buffers_with_diagnostics(&self) -> impl Iterator<Item = BufferId> + '_ {
        self.diagnostics.buffers_with_diagnostics()
    }

    /// Production constructor: one real server process per registration.
    /// `wake` is forwarded to every spawned server's reader/stderr threads,
    /// so the main loop wakes instead of polling for completion.
    pub(crate) fn new_threaded(wake: WakeCallback) -> Self {
        Self::with_backend(Box::new(ThreadedLspBackend::with_waker(wake)))
    }

    /// Test constructor: scripted responses, no process, no threads.
    #[cfg(test)]
    pub(crate) fn new_inline() -> Self {
        Self::with_backend(Box::new(InlineLspBackend::new()))
    }

    /// Test-only: swap in an already-scripted backend (e.g. one built via
    /// `InlineLspBackend::with_default_handshake` plus extra `respond_to`
    /// calls) — `backend_mut` only exposes the trait object, which can't
    /// reach `InlineLspBackend`'s scripting methods.
    #[cfg(test)]
    pub(crate) fn from_backend_for_test(backend: Box<dyn LspBackend>) -> Self {
        Self::with_backend(backend)
    }

    /// Reach the raw backend directly. Test-only in practice (the scripted
    /// round-trip test): production code goes through `drain_lsp`'s direct
    /// field access instead.
    #[allow(dead_code)]
    pub(crate) fn backend_mut(&mut self) -> &mut dyn LspBackend {
        self.backend.as_mut()
    }

    /// Test-only direct client insertion; the real registration
    /// path (`register-lsp-server!` -> spawn-on-first-open) populates
    /// this map in production. Inserted with no language — tests that need
    /// one call `insert_server_key_for_test` next.
    #[cfg(test)]
    pub(crate) fn insert_client_for_test(&mut self, client: LspClient) -> ServerId {
        let id = client.id();
        self.servers.insert(
            id,
            ServerEntry {
                client,
                language: None,
                name: "lsp".to_string(),
                capabilities_json: None,
                progress: Vec::new(),
            },
        );
        id
    }

    /// `root` must match the client's own `root` (`LspClient::new`'s
    /// second argument) — a real attach never has these disagree, since
    /// both come from the same `resolve_root` call.
    #[cfg(test)]
    pub(crate) fn insert_server_key_for_test(
        &mut self,
        language: String,
        root: PathBuf,
        server_id: ServerId,
    ) {
        let entry = self
            .servers
            .get_mut(&server_id)
            .expect("insert_client_for_test first");
        assert_eq!(
            entry.client.root(),
            root,
            "test key root must match the client's own root"
        );
        entry.language = Some(language);
    }

    #[cfg(test)]
    pub(crate) fn insert_server_name_for_test(&mut self, server_id: ServerId, name: String) {
        if let Some(entry) = self.servers.get_mut(&server_id) {
            entry.name = name;
        }
    }

    #[cfg(test)]
    pub(crate) fn client_for_test(&mut self, server: ServerId) -> Option<&mut LspClient> {
        self.servers.get_mut(&server).map(|e| &mut e.client)
    }

    /// The registered `command` for `language`, or `None` if unregistered —
    /// lets tests observe last-wins replacement and unregistration without
    /// reaching into the private `configs` map directly.
    #[cfg(test)]
    pub(crate) fn config_command_for_test(&self, language: &str) -> Option<String> {
        self.configs.get(language).map(|c| c.command.clone())
    }

    /// The registered `settings` JSON for `language`, or `None` if
    /// unregistered or registered with no settings — lets tests assert the
    /// Steel-to-JSON settings conversion without reaching into the private
    /// `configs` map directly.
    #[cfg(test)]
    pub(crate) fn config_settings_for_test(&self, language: &str) -> Option<serde_json::Value> {
        self.configs.get(language).and_then(|c| c.settings.clone())
    }

    /// Same as `config_settings_for_test`, for `init_options` — the seeded
    /// catalog registers the same blob under both keywords (see
    /// `core:lsp/registration.scm`), so a test asserting the conversion
    /// needs both, not just `settings`.
    #[cfg(test)]
    pub(crate) fn config_init_options_for_test(&self, language: &str) -> Option<serde_json::Value> {
        self.configs
            .get(language)
            .and_then(|c| c.init_options.clone())
    }

    /// The registered `#:env` pairs for `language`, or `None` if
    /// unregistered — lets a test assert the `#:env` decode/round-trip into
    /// `LspServerConfig.env` without reaching into the private `configs`
    /// map directly.
    #[cfg(test)]
    pub(crate) fn config_env_for_test(&self, language: &str) -> Option<Vec<(String, String)>> {
        self.configs.get(language).map(|c| c.env.clone())
    }

    /// Number of tracked servers — one entry per `backend.start`, so a
    /// second buffer attaching under the same (language, root) key (rather
    /// than spawning) leaves this unchanged.
    #[cfg(test)]
    pub(crate) fn server_count_for_test(&self) -> usize {
        self.servers.len()
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_counts_for_test(&self, bid: BufferId) -> (usize, usize) {
        self.diagnostics.counts(bid)
    }

    /// The most recent active `$/progress` task's title for `server` — lets
    /// tests assert the begin/report merge machine (title persists across a
    /// `report` that omits it) without going through `LspActivity`, which
    /// doesn't carry `title` (it's not rendered — see `introspect::activity`).
    #[cfg(test)]
    pub(crate) fn progress_title_for_test(&self, server: ServerId) -> Option<&str> {
        self.servers
            .get(&server)?
            .progress
            .last()
            .map(|(_, task)| task.title.as_str())
    }

    /// Number of registered callbacks still awaiting dispatch — a leak
    /// check: every callback must eventually be removed by `dispatch_completed`
    /// (response, timeout, or teardown), never orphaned.
    #[cfg(test)]
    pub(crate) fn callback_count_for_test(&self) -> usize {
        self.callbacks.len()
    }

    /// Number of tracked `#:supersede` keys — leak check: an entry must be
    /// removed once its request finishes (response/timeout) or its server
    /// stops, never orphaned.
    #[cfg(test)]
    pub(crate) fn supersede_count_for_test(&self) -> usize {
        self.supersede.len()
    }

    /// Diagnostics visible in `range` (buffer-wide char offsets) for `bid`,
    /// at or above `floor` severity — the render write side reads this
    /// directly (no JSON round-trip; that's
    /// `introspect::diagnostics_for_buffer`'s job for Steel).
    pub(crate) fn diagnostics_for_range(
        &self,
        bid: BufferId,
        range: std::ops::Range<usize>,
        floor: DiagSeverity,
    ) -> impl Iterator<Item = &StoredDiag> {
        self.diagnostics.for_range(bid, range, floor)
    }

    /// Drops every diagnostic for `bid`, across every server — called from
    /// `close_buffer` (a pure memory-leak fix there) and from `:e!` reload
    /// (a correctness fix: stale offsets must not survive against the new
    /// content). Returns whether anything was actually removed.
    pub(crate) fn remove_buffer_diagnostics(&mut self, bid: BufferId) -> bool {
        self.diagnostics.remove_buffer(bid)
    }

    #[cfg(test)]
    pub(crate) fn diagnostics_for_test(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.diagnostics
            .for_range(bid, 0..usize::MAX, diagnostics::DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
    }

    /// Disjoint-borrow accessor for callers that need to drive a client and
    /// its backend in the same call (`send_or_queue`, `start_handshake`) —
    /// a plain two-method-call sequence can't do this from outside
    /// `LspState` since `backend_mut`/`client_for_test` each borrow the
    /// whole struct. Production caller: every send site in `sync.rs`, so
    /// document sync respects the Starting-queue instead of writing to the
    /// wire directly.
    pub(crate) fn client_and_backend(
        &mut self,
        server: ServerId,
    ) -> Option<(&mut LspClient, &mut dyn LspBackend)> {
        let LspState {
            servers, backend, ..
        } = self;
        let client = &mut servers.get_mut(&server)?.client;
        Some((client, backend.as_mut()))
    }

    /// Files `callback` under an already-sent request's `(server, id)` —
    /// `drain_lsp`'s per-server loop already has both in scope at dispatch
    /// time, so no separate token needs to be minted. Production caller:
    /// `bridge::send_one_lsp_request`, called after `send_request`
    /// returns the id.
    pub(crate) fn register_callback(
        &mut self,
        server: ServerId,
        id: RequestId,
        stale_check: Option<(BufferId, u64)>,
        callback: LspCallback,
    ) {
        self.callbacks.insert(
            (server, id),
            CallbackEntry {
                callback,
                stale_check,
            },
        );
    }

    /// Sends a request through `server`'s client, if one is registered.
    /// `None` if `server` has no tracked client (can't happen with the real
    /// registration path; still must not panic).
    pub(crate) fn send_request(
        &mut self,
        server: ServerId,
        method: &str,
        params: serde_json::Value,
        meta: RequestMeta,
    ) -> Option<RequestId> {
        let client = &mut self.servers.get_mut(&server)?.client;
        Some(client.send_request(self.backend.as_mut(), method, params, meta))
    }
}

impl AsyncSource for LspState {
    fn next_wake(&self, now: Instant) -> Option<Instant> {
        // Real deadlines only: response *arrival* needs no wake here — the
        // transport threads wake the event loop directly via
        // `termina::PlatformWaker` the moment a message lands. What remains
        // is the earliest pending-request timeout across every server
        // (initialize/shutdown included — see `LspClient::earliest_deadline`),
        // so a silent server's timeout sweep in `take_completed` still
        // fires promptly.
        let deadline = self
            .servers
            .values()
            .filter_map(|e| e.client.earliest_deadline())
            .min();

        // A server mid-handshake or reporting `$/progress` (indexing,
        // loading, ...) needs the statusline spinner to keep animating —
        // wake at the spinner's own cadence. `Starting` is included (not
        // just progress): without it the spinner freezes against
        // `initialize`'s 30s deadline.
        let spinner = self
            .has_animating_server()
            .then(|| now + progress::SPINNER_INTERVAL);

        [deadline, spinner].into_iter().flatten().min()
    }
}

impl Editor {
    /// `(errors, warnings)` for `bid` from the diagnostics store — the
    /// statusline's `Diagnostics` element reads this directly (never through
    /// Steel; `self.lsp` is private to `editor` and its descendants, so
    /// callers outside it, like `ui::statusline`, go through this).
    pub(crate) fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        introspect::diagnostic_counts(&self.lsp, bid)
    }

    /// `bid`'s attached server's lifecycle/loading state — the statusline's
    /// `Diagnostics` element reads this to decide whether to show the
    /// loading spinner instead of counts. Same access rationale as
    /// `diagnostic_counts` above.
    pub(crate) fn lsp_activity(&self, bid: BufferId) -> introspect::LspActivity {
        introspect::activity(&self.state, &self.lsp, bid)
    }

    /// Current animation frame for the statusline loading spinner.
    pub(crate) fn lsp_spinner_frame(&self) -> usize {
        self.lsp.spinner.frame
    }
}
