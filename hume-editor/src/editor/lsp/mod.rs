//! Editor-side LSP state: holds the backend and per-server client state,
//! and drains events at frame cadence. Wires the backend + `AsyncSource`
//! plumbing, per-client lifecycle state, request/callback bookkeeping and
//! server->client dispatch (this module), document sync, diagnostics,
//! registration, and observability commands.

mod bridge;
pub(crate) mod completion;
mod diagnostics;
pub(crate) mod edits;
pub(crate) mod introspect;
mod registry;
pub(crate) mod sync;

#[cfg(test)]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustc_hash::{FxHashMap, FxHashSet};

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::{LspBackend, ServerId, ThreadedLspBackend, WakeCallback};
use hume_lsp::client::{
    ClientAction, LspClient, Outcome, RequestMeta, ServerState, server_request_response,
};
use hume_lsp::codec::{Message, RequestId, ResponseError};
#[cfg(test)]
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::transport::InboundEvent;
use lsp_types::request::Request as _;

use super::Editor;
use super::async_source::AsyncSource;
use super::message_log::Severity;
use diagnostics::DiagnosticsStore;
pub(crate) use diagnostics::{DiagSeverity, StoredDiag};
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

/// One active work-done-progress task, built from a `begin` notification and
/// updated in place by `report`s. `percentage` is optional per the LSP spec —
/// a `report` omitting it leaves it unchanged, so it's merged rather than the
/// task being replaced wholesale.
#[derive(Debug, Clone)]
pub(crate) struct ProgressTask {
    // Not read in production — the statusline only shows the spinner +
    // percentage (`introspect::LspActivity::Progress` carries no title).
    // Kept so the `$/progress` begin/report merge machine has something to
    // assert against in tests, via `LspState::progress_title_for_test`.
    #[allow(dead_code)]
    pub(crate) title: String,
    pub(crate) percentage: Option<u32>,
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

/// How often the loading spinner advances a frame — independent of how
/// often `drain_lsp` itself runs (`next_wake` may wake faster than this
/// while a handshake or `$/progress` task is active).
const SPINNER_INTERVAL: Duration = Duration::from_millis(100);

/// Monotonic animation-frame counter for the statusline spinner
/// (`elements/diagnostics.rs`'s loading state). `frame` is a plain `usize`
/// so the render side (`format`) stays a deterministic, clock-free function
/// of its inputs — only this clock needs a real `Instant`.
#[derive(Default)]
struct SpinnerClock {
    frame: usize,
    last_advance: Option<Instant>,
}

impl SpinnerClock {
    /// Bumps `frame` by one if at least `SPINNER_INTERVAL` has elapsed since
    /// the last advance (or this is the first call).
    fn maybe_advance(&mut self, now: Instant) {
        if self
            .last_advance
            .is_none_or(|last| now.saturating_duration_since(last) >= SPINNER_INTERVAL)
        {
            self.frame = self.frame.wrapping_add(1);
            self.last_advance = Some(now);
        }
    }
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
    /// diagnostics are what `resync_config_state` replays, per `docs/lsp.md`.
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
        let spinner = self.has_animating_server().then(|| now + SPINNER_INTERVAL);

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

    /// Per-frame drain: routes every backend event through its client's
    /// `on_event`, dispatches the resulting `ClientAction`s, then pulls
    /// each client's completed requests (responses + timeouts) via
    /// `take_completed` and dispatches those too.
    pub(super) fn drain_lsp(&mut self) {
        self.flush_lsp_pending_changes();

        let events = self.lsp.backend.drain();
        // Coalesce publishDiagnostics within this batch: keep only the last
        // one per (server, uri) — servers burst-publish and only the newest
        // matters. Ingested after the loop so a later action for the same
        // (server, uri) always wins regardless of arrival order within the
        // batch.
        // clippy's `mutable_key_type` flags `lsp_types::Uri` for the `Cell`s
        // inside its underlying `fluent_uri::Uri`'s parse-offset cache — but
        // `Uri`'s `Hash`/`PartialEq`/`Eq` are hand-implemented against
        // `.as_str()` only (lsp-types 0.97.0's uri.rs), which those cells
        // never affect. A false positive for this specific type.
        #[allow(clippy::mutable_key_type)]
        let mut diag_batch: FxHashMap<
            (ServerId, lsp_types::Uri),
            lsp_types::PublishDiagnosticsParams,
        > = FxHashMap::default();
        for (server_id, ev) in events {
            let actions = match self.lsp.servers.get_mut(&server_id) {
                Some(entry) => entry.client.on_event(ev),
                None => continue,
            };
            for action in actions {
                if let ClientAction::Diagnostics(params) = action {
                    diag_batch.insert((server_id, params.uri.clone()), params);
                    continue;
                }
                self.dispatch_lsp_action(server_id, action);
            }
        }
        // OnDiagnosticsChanged fires once per buffer this batch actually
        // touched — a FxHashSet dedupes two (server, uri) entries that both
        // resolved to the same buffer (multiple roots, same file; not a v1
        // scenario, but cheap to get right).
        let mut touched: FxHashSet<BufferId> = FxHashSet::default();
        for ((server_id, _uri), params) in diag_batch {
            if let Some(bid) = self.ingest_publish_diagnostics(server_id, params) {
                touched.insert(bid);
            }
        }
        for bid in touched {
            self.fire_hook_diagnostics_changed(bid);
        }

        let now = Instant::now();

        // Advance the statusline loading spinner while any server is mid-
        // handshake or reporting `$/progress` — idle otherwise, so the
        // frame counter doesn't drift while there's nothing to animate.
        if self.lsp.has_animating_server() {
            self.lsp.spinner.maybe_advance(now);
        }

        let server_ids: Vec<ServerId> = self.lsp.servers.keys().copied().collect();
        for server_id in server_ids {
            let LspState {
                servers, backend, ..
            } = &mut self.lsp;
            let (completed, actions) = match servers.get_mut(&server_id) {
                Some(entry) => entry.client.take_completed(backend.as_mut(), now),
                None => continue,
            };
            for action in actions {
                self.dispatch_lsp_action(server_id, action);
            }
            for (id, meta, outcome) in completed {
                self.dispatch_completed(server_id, id, meta, outcome);
            }
        }
    }

    /// [`lsp_shutdown_all`](Self::lsp_shutdown_all)'s production grace
    /// window — the value `run`'s post-loop teardown actually uses; tests
    /// pass their own to exercise the zero- and long-window edges.
    /// `hume_platform::QUIT_GRACE` is sized against this constant — keep the
    /// two in step.
    pub(in crate::editor) const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

    /// Graceful shutdown on quit: `begin_shutdown` (shutdown request, then
    /// exit notification) for every Running client, then a bounded grace
    /// window draining for their voluntary EOF, before transport-level
    /// teardown (`backend.shutdown`, which reaps any process still alive)
    /// regardless. Starting clients skip the protocol handshake — nothing
    /// but `initialize` is legal to send before `initialized`, so a plain
    /// transport kill is the only option for them.
    ///
    /// Events drained during the grace window are otherwise discarded — a
    /// lingering response or stderr line has nowhere useful to go while the
    /// editor is tearing down.
    pub(in crate::editor) fn lsp_shutdown_all(&mut self, grace: Duration) {
        if self.lsp.servers.is_empty() {
            return;
        }

        let server_ids: Vec<ServerId> = self.lsp.servers.keys().copied().collect();
        let mut awaiting_eof: FxHashSet<ServerId> = FxHashSet::default();
        for &server_id in &server_ids {
            let LspState {
                servers, backend, ..
            } = &mut self.lsp;
            if let Some(entry) = servers.get_mut(&server_id)
                && entry.client.state() == ServerState::Running
            {
                entry.client.begin_shutdown(backend.as_mut());
                awaiting_eof.insert(server_id);
            }
        }

        if !awaiting_eof.is_empty() {
            let deadline = Instant::now() + grace;
            while !awaiting_eof.is_empty() && Instant::now() < deadline {
                for (server_id, ev) in self.lsp.backend.drain() {
                    if matches!(ev, InboundEvent::Eof { .. }) {
                        awaiting_eof.remove(&server_id);
                    }
                }
                if !awaiting_eof.is_empty() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }

        for server_id in server_ids {
            self.lsp.backend.shutdown(server_id);
        }
    }

    pub(super) fn dispatch_lsp_action(&mut self, server_id: ServerId, action: ClientAction) {
        match action {
            ClientAction::BecameRunning { send } => {
                for msg in send {
                    self.lsp.backend.send(server_id, msg);
                }
                // Decode once here rather than per `(lsp-capabilities …)`
                // call — conversion is per-server-startup, not per-call.
                let json = self
                    .lsp
                    .servers
                    .get(&server_id)
                    .and_then(|e| e.client.capabilities())
                    .and_then(|caps| serde_json::to_value(caps).ok());
                if let Some(json) = json
                    && let Some(entry) = self.lsp.servers.get_mut(&server_id)
                {
                    entry.capabilities_json = Some(json);
                }
                // Fire on-lsp-attach for every buffer already attached to
                // this server — it was Starting until now, so `lsp_attach_buffer`
                // deliberately skipped firing it for them.
                if let Some(lang) = introspect::server_language(&self.lsp, server_id) {
                    let bids: Vec<BufferId> = self
                        .state
                        .buffers
                        .iter()
                        .filter(|(_, buf)| buf.lsp_server == Some(server_id))
                        .map(|(bid, _)| bid)
                        .collect();
                    for bid in bids {
                        self.fire_hook_lsp_attach(bid, &lang);
                    }
                }
            }
            ClientAction::Crashed { error } => {
                let name = self.lsp_server_name(server_id);
                self.report(
                    Severity::Error,
                    format!(
                        "lsp: {name} crashed{}",
                        error.map(|e| format!(": {e}")).unwrap_or_default()
                    ),
                );
                // Fail every in-flight request immediately rather than
                // leaving each to expire on its own deadline — the crash is
                // already known, so there's nothing to wait for. Mirrors
                // `:lsp-stop`'s own teardown (`lsp_stop_one`).
                if let Some(entry) = self.lsp.servers.get_mut(&server_id) {
                    // A crashed server can't finish whatever it was loading —
                    // drop its tracked progress so the statusline spinner
                    // doesn't keep animating for a server that's gone.
                    entry.progress.clear();
                    for (id, meta) in entry.client.drain_pending() {
                        self.dispatch_completed(server_id, id, meta, Outcome::TimedOut);
                    }
                }
            }
            ClientAction::ServerRequest { id, method, params } => {
                // `workspace/applyEdit` needs `&mut Editor` (the edit engine) —
                // every other request answers from the pure lookup table.
                let result = if method == lsp_types::request::ApplyWorkspaceEdit::METHOD {
                    self.apply_edit_request_response(&params)
                } else {
                    let settings = introspect::server_language(&self.lsp, server_id)
                        .and_then(|lang| self.lsp.configs.get(&lang))
                        .and_then(|cfg| cfg.settings.as_ref());
                    server_request_response(&method, &params, settings)
                };
                self.lsp
                    .backend
                    .send(server_id, Message::Response { id, result });
            }
            ClientAction::Diagnostics(params) => {
                // The uncoalesced single-notification path — `drain_lsp`'s
                // batching loop intercepts and coalesces `Diagnostics`
                // before dispatch, so this arm only fires for a test or any
                // future caller that dispatches one directly.
                if let Some(bid) = self.ingest_publish_diagnostics(server_id, params) {
                    self.fire_hook_diagnostics_changed(bid);
                }
            }
            ClientAction::Progress(params) => {
                self.handle_progress(server_id, params);
            }
            ClientAction::LogMessage(params) => {
                let name = self.lsp_server_name(server_id);
                let severity = match params.typ {
                    lsp_types::MessageType::ERROR => Severity::Error,
                    lsp_types::MessageType::WARNING => Severity::Warning,
                    _ => Severity::Trace, // Info/Log
                };
                self.report(severity, format!("{name}: {}", params.message));
            }
            ClientAction::ShowMessage(params) => {
                let name = self.lsp_server_name(server_id);
                self.report(Severity::Info, format!("{name}: {}", params.message));
            }
            ClientAction::ServerNotification { method, params } => {
                self.dispatch_server_notification(server_id, &method, params);
            }
            ClientAction::Stderr(line) => {
                // rust-analyzer logs a lot — Trace keeps :messages usable;
                // never promote stderr to a higher severity.
                let name = self.lsp_server_name(server_id);
                self.report(Severity::Trace, format!("{name}: {line}"));
            }
        }
    }

    /// Typed handling of `$/progress`: begin/end logged at Trace; the task
    /// itself is tracked on `ServerEntry.progress` for the statusline
    /// spinner, with `report`s merged into it (absent fields mean
    /// "unchanged" per the LSP spec).
    fn handle_progress(&mut self, server_id: ServerId, params: lsp_types::ProgressParams) {
        let name = self.lsp_server_name(server_id);
        let token = match params.token {
            lsp_types::NumberOrString::Number(n) => n.to_string(),
            lsp_types::NumberOrString::String(s) => s,
        };
        // `ProgressParamsValue` has exactly one variant — irrefutable.
        let lsp_types::ProgressParamsValue::WorkDone(progress) = params.value;
        match progress {
            lsp_types::WorkDoneProgress::Begin(begin) => {
                self.report(Severity::Trace, format!("{name}: {} started", begin.title));
                if let Some(entry) = self.lsp.servers.get_mut(&server_id) {
                    entry.progress.push((
                        token,
                        ProgressTask {
                            title: begin.title,
                            percentage: begin.percentage,
                        },
                    ));
                }
            }
            lsp_types::WorkDoneProgress::Report(report) => {
                let Some(entry) = self.lsp.servers.get_mut(&server_id) else {
                    return;
                };
                let Some((_, task)) = entry.progress.iter_mut().find(|(t, _)| *t == token) else {
                    return; // report for an unknown token — nothing to merge into
                };
                // An absent percentage means "unchanged" per the LSP spec — merge, don't overwrite.
                if let Some(percentage) = report.percentage {
                    task.percentage = Some(percentage);
                }
            }
            lsp_types::WorkDoneProgress::End(_) => {
                self.report(Severity::Trace, format!("{name}: progress finished"));
                if let Some(entry) = self.lsp.servers.get_mut(&server_id) {
                    entry.progress.retain(|(t, _)| *t != token);
                }
            }
        }
    }

    /// Answers a server-initiated `workspace/applyEdit` request by actually
    /// applying it. Per spec this never fails at the JSON-RPC level: a rejected or
    /// malformed edit still gets a 200 response, just with `applied: false`.
    pub(crate) fn apply_edit_request_response(
        &mut self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, ResponseError> {
        let Some(edit_json) = params.get("edit").cloned() else {
            return Ok(serde_json::json!({
                "applied": false,
                "failureReason": "missing edit",
            }));
        };
        let we: lsp_types::WorkspaceEdit = match serde_json::from_value(edit_json) {
            Ok(we) => we,
            Err(e) => {
                return Ok(serde_json::json!({
                    "applied": false,
                    "failureReason": format!("malformed edit: {e}"),
                }));
            }
        };
        let result = edits::apply_workspace_edit(&mut self.state, &mut self.view, &self.lsp, we);
        // Drain regardless of outcome: `apply_workspace_edit`'s contract is
        // "validate all, then apply all", but it opens buffers as it *validates*
        // each entry (`edits.rs`'s `resolve_or_open` calls), so a failure on
        // entry 3 of 5 still leaves entries 1-2's buffers open and queued here.
        self.detect_pending_languages();
        match result {
            Ok(_summary) => Ok(serde_json::json!({ "applied": true })),
            Err(e) => Ok(serde_json::json!({
                "applied": false,
                "failureReason": e,
            })),
        }
    }

    /// Name used to prefix this server's log lines — the registered
    /// `command` string, or `"lsp"` if the server was never registered
    /// through the normal path (shouldn't happen outside tests).
    fn lsp_server_name(&self, server_id: ServerId) -> String {
        self.lsp
            .servers
            .get(&server_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "lsp".to_string())
    }

    /// The registered language for `server_id` — the "server name" the
    /// Steel surface deals in, since that's what `register-lsp-server!` and
    /// `lsp-request`'s `server` argument both use.
    fn lsp_server_language(&self, server_id: ServerId) -> Option<String> {
        introspect::server_language(&self.lsp, server_id)
    }

    /// `textDocument/publishDiagnostics`, `$/progress`, `window/logMessage`,
    /// and `window/showMessage` never reach here — `hume-lsp` classifies
    /// them into typed `ClientAction` variants, handled directly in
    /// `dispatch_lsp_action`. Only an unclassified method, or a known
    /// method whose params fail both the strict parse and `hume-lsp`'s
    /// lenient recovery, arrives here — either goes to a registered Steel
    /// `on-lsp-notification` handler, or an "unhandled notification" Trace
    /// line if none is registered.
    fn dispatch_server_notification(
        &mut self,
        server_id: ServerId,
        method: &str,
        params: serde_json::Value,
    ) {
        let name = self.lsp_server_name(server_id);
        let handlers = self
            .scripting
            .as_ref()
            .map(|h| h.lsp_notification_handlers_for(method))
            .unwrap_or_default();
        if handlers.is_empty() {
            self.report(
                Severity::Trace,
                format!("{name}: unhandled notification {method}"),
            );
            return;
        }
        let server_val = match self.lsp_server_language(server_id) {
            Some(lang) => steel::rvals::SteelVal::StringV(lang.into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        let params_val = hume_scripting::json::json_to_steel(&params);
        for handler in handlers {
            self.queue_steel_call(handler, vec![server_val.clone(), params_val.clone()]);
        }
    }

    fn dispatch_completed(
        &mut self,
        server_id: ServerId,
        id: RequestId,
        meta: RequestMeta,
        outcome: Outcome,
    ) {
        // A tracked `#:supersede` entry for this id is finished with —
        // response, timeout, crash-drain, and `:lsp-stop`-drain all arrive
        // here, so this is the one chokepoint that can't miss any of them.
        self.lsp
            .supersede
            .retain(|(sid, _), rid| !(*sid == server_id && *rid == id));

        let Some(entry) = self.lsp.callbacks.remove(&(server_id, id)) else {
            // No callback is ever registered for the internal `shutdown`
            // request (it's fire-and-forget from `begin_shutdown`) — a
            // server-side error on it would otherwise vanish silently.
            if meta.method == lsp_types::request::Shutdown::METHOD
                && let Outcome::Err(e) = &outcome
            {
                self.report(
                    Severity::Trace,
                    format!("lsp: shutdown failed: {} ({})", e.message, e.code),
                );
            }
            return;
        };

        if matches!(outcome, Outcome::TimedOut) {
            self.report(Severity::Trace, format!("lsp: {} timed out", meta.method));
            // Dispatched (not dropped): a callback that never fires on
            // timeout means a caller (e.g. a Steel err-mapped callback)
            // has no way to notice and would hang silently. TimedOut still
            // goes through the staleness check below like any other outcome.
        }

        if let Some((bid, text_gen)) = entry.stale_check {
            let current = self.state.buffers.try_get(bid).map(|b| b.text_gen);
            if current != Some(text_gen) && !meta.allow_stale {
                return; // dropped silently — parse-worker staleness discipline
            }
        }

        (entry.callback)(self, outcome);
    }

    /// `:lsp-status` text: one line per registered server (language, root,
    /// lifecycle state, in-flight request count, negotiated encoding),
    /// followed by one line per attached buffer with its diagnostic counts.
    pub(in crate::editor) fn lsp_status_text(&self) -> String {
        let mut servers: Vec<(&str, &LspClient)> = self
            .lsp
            .servers
            .values()
            .filter_map(|e| e.language.as_deref().map(|lang| (lang, &e.client)))
            .collect();
        servers.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.root().cmp(b.1.root())));

        let mut lines = Vec::new();
        if servers.is_empty() {
            lines.push("No LSP servers registered.".to_string());
        }
        for (language, client) in servers {
            lines.push(format!(
                "{language} @ {} — {:?}, {} in flight, encoding: {:?}",
                client.root().display(),
                client.state(),
                client.pending_count(),
                client.encoding(),
            ));
        }

        let mut buffer_lines: Vec<String> = self
            .state
            .buffers
            .iter()
            .filter_map(|(bid, buf)| {
                buf.lsp_server.map(|_| {
                    let (errors, warnings) = self.lsp.diagnostics.counts(bid);
                    format!(
                        "  {} — {errors} error(s), {warnings} warning(s)",
                        buf.display_name()
                    )
                })
            })
            .collect();
        lines.append(&mut buffer_lines);

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests;
