//! Editor-side LSP state: the server registry (`registry.rs`), running
//! instances (`instances.rs`), per-buffer documents and their attachments
//! (`document.rs`, mutated only through `attach.rs`), document sync,
//! request/callback bookkeeping and server->client dispatch (`drain.rs`),
//! diagnostics, and observability commands.

mod attach;
mod bridge;
pub(in crate::editor) mod diagnostics;
pub(in crate::editor) mod document;
mod drain;
pub(in crate::editor) mod edits;
mod features;
mod instances;
pub(crate) mod introspect;
pub(in crate::editor) mod params;
mod progress;
mod pull;
mod registry;
mod route;
pub(in crate::editor) mod sync;

use std::path::PathBuf;
use std::time::Instant;

use rustc_hash::FxHashSet;

use hume_editing::text::TextVersion;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_lsp::backend::{LspBackend, ServerId, ThreadedLspBackend};
use hume_lsp::client::{LspClient, RequestMeta, ServerState};
use hume_lsp::codec::RequestId;
#[cfg(test)]
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::transport::WakeCallback;
use hume_rope::position_encoding::PositionEncoding;
use hume_scripting::ServerName;

use super::Editor;
use super::async_source::AsyncSource;
use bridge::Deliveries;
pub(in crate::editor) use bridge::RustResponder;
use instances::Instances;
use progress::SpinnerClock;
use registry::Registry;

/// `lsp_types::Range` → char-offset span via
/// [`hume_lsp::position::from_lsp_range`] and
/// [`hume_rope::position_encoding::wire_range_to_char_range`]; see
/// `hume_lsp::position` for why the `lsp_types` crossing is a free function
/// rather than a `From` impl.
pub(in crate::editor) fn wire_range_to_chars(
    rope: &ropey::Rope,
    range: &lsp_types::Range,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> hume_rope::offset::ExclusiveRange<hume_rope::offset::CharOffset> {
    hume_rope::position_encoding::wire_range_to_char_range(
        rope,
        hume_lsp::position::from_lsp_range(range),
        encoding,
    )
}

/// The cluster a wire position names in `text`, where goto lands: the line
/// clamped to the text, the column placed on the line's content, so a column
/// past the line's end lands on its last content cluster. Decoded through
/// `place_char_column`, not `wire_to_char`, so a position between a base
/// character and a combining mark lands on the cluster's start, as every
/// other goto target does.
pub(in crate::editor) fn wire_to_cluster(
    text: &hume_editing::text::BufferText,
    pos: hume_rope::position_encoding::WirePos,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> hume_rope::cluster::ClusterStart {
    let (line, char_col) =
        hume_rope::position_encoding::wire_to_line_char_col(text.rope(), pos, encoding);
    text.columns().place_char(line, char_col)
}

/// The value cached for `encoding` in `cache`, computed by `make` and
/// cached the first time that encoding is asked for.
pub(in crate::editor::lsp) fn per_encoding<T>(
    cache: &mut Vec<(PositionEncoding, T)>,
    encoding: PositionEncoding,
    make: impl FnOnce() -> T,
) -> &T {
    let index = match cache.iter().position(|(cached, _)| *cached == encoding) {
        Some(index) => index,
        None => {
            cache.push((encoding, make()));
            cache.len() - 1
        }
    };
    &cache[index].1
}

/// How a crashed server is named to the user, with the command that
/// restarts it.
pub(in crate::editor) fn crashed_text(name: &str, error: Option<&str>) -> String {
    let detail = error.map(|e| format!(": {e}")).unwrap_or_default();
    format!("{name} crashed{detail} (:lsp-restart {name})")
}

/// Everything a delivery's callback needs checked against once its answers
/// land, gathered at send time so `EditorState::anchor_admits` (`drain.rs`)
/// has one place to apply both checks.
///
/// `Copy`: a queued `PendingWork::Call` carries its own copy alongside the
/// delivery's (`Editor::run_pending_batch` re-checks it at dequeue time;
/// see that function's doc for why one check at drain isn't enough), and
/// the struct is four primitives, cheap to duplicate.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) struct ResponseAnchor {
    pub(in crate::editor) bid: BufferId,
    /// If `bid` has moved past this generation by drain time, the outcome is
    /// dropped silently unless `allow_stale` opts out: the parse-worker
    /// staleness discipline.
    pub(in crate::editor) version: TextVersion,
    /// `#:allow-stale`: skips the text version check above.
    pub(in crate::editor) allow_stale: bool,
    /// `#:require-focus`: the pane the request was made from, if the
    /// outcome should be dropped (Trace-logged) unless it's still the
    /// focused pane, and still shows `bid`, when the response lands. Never
    /// re-derived from `bid` alone (which would admit any pane still
    /// showing it, not the exact one the request was made from). `None` for
    /// a request whose delivery doesn't depend on focus.
    pub(in crate::editor) require_focus: Option<PaneId>,
    /// `#:tracked`: a tracked position the request holds, released once its
    /// callback has run or will never run (`release_request_position`),
    /// unless the callback kept it.
    pub(in crate::editor) tracked: Option<hume_scripting::host::HostToken>,
}

/// Whether attachment changes apply now. `:reload-config` clears every
/// buffer's language before `init.scm` re-registers servers, so reconciling
/// in between would detach (and stop) every server the new config keeps.
/// `reset_config` suspends; `resync_config_state` resumes and reconciles
/// once, against the complete new config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcilePhase {
    Live,
    Suspended,
}

pub(crate) struct LspState {
    backend: Box<dyn LspBackend>,
    registry: Registry,
    instances: Instances,
    /// Steel requests in flight, each waiting for every server it went to.
    deliveries: Deliveries,
    /// Drives the statusline loading spinner's animation frame. Advanced
    /// (at most) once per `drain_lsp` call, gated on its own interval so
    /// the animation speed doesn't depend on the event loop's wake cadence.
    spinner: SpinnerClock,
    /// `publishDiagnostics` not yet ingested: gathered by a drain, and left
    /// over when its `canonicalize()` budget runs out.
    publishes: diagnostics::PublishQueue,
    reconcile: ReconcilePhase,
    /// The `(name, root)` instances a stop ended. A reconcile does not
    /// start them again: `:lsp-restart` and registering the name clear them.
    stopped: FxHashSet<(ServerName, PathBuf)>,
}

impl LspState {
    /// State over `backend`, the one constructor body every entry point
    /// shares. Tests pass an already-scripted `InlineLspBackend`, which
    /// `backend_mut` cannot reach through the trait object.
    pub(in crate::editor) fn with_backend(backend: Box<dyn LspBackend>) -> Self {
        Self {
            backend,
            registry: Registry::default(),
            instances: Instances::default(),
            deliveries: Deliveries::default(),
            spinner: SpinnerClock::default(),
            publishes: diagnostics::PublishQueue::default(),
            reconcile: ReconcilePhase::Live,
            stopped: FxHashSet::default(),
        }
    }

    /// `true` while any server needs the statusline spinner animating:
    /// mid-handshake (`Starting`) or reporting `$/progress` (indexing,
    /// loading, ...). Single source of truth for the two sites that must
    /// agree: `AsyncSource::next_wake` (*when* to wake for the next spinner
    /// tick) and `drain_lsp` (*whether* to advance the frame once woken). If
    /// they diverged, the spinner would freeze or wake without advancing.
    pub(in crate::editor) fn has_animating_server(&self) -> bool {
        self.instances
            .iter()
            .any(|(_, i)| i.client.state() == ServerState::Starting || !i.progress.is_empty())
    }

    /// Clears what `:reload-config` must not let survive; see
    /// `Editor::reset_config_state`. `registry` is the `init.scm`-built
    /// config the new eval re-populates. `deliveries` holds closures and
    /// `SteelVal`s from the outgoing engine; `lsp_dispatch_completed`
    /// tolerates an answer no delivery waits for, so dropping it is safe.
    /// Running instances, the
    /// documents attached to them and their diagnostics survive: reconcile
    /// is suspended until `resync_config_state` matches them against the
    /// new registry.
    pub(in crate::editor) fn reset_config(&mut self) {
        self.registry = Registry::default();
        self.stopped.clear();
        self.deliveries.clear();
        self.reconcile = ReconcilePhase::Suspended;
    }

    /// Drops the stop of every root of `name`, so the next reconcile may
    /// start it.
    pub(in crate::editor) fn forget_stopped(&mut self, name: &ServerName) {
        self.stopped.retain(|(stopped, _)| stopped != name);
    }

    /// Lets attachment changes apply again after a reload's `reset_config`.
    pub(in crate::editor) fn resume_reconcile(&mut self) {
        self.reconcile = ReconcilePhase::Live;
    }

    /// Production constructor: one real server process per instance.
    /// `wake` is forwarded to every spawned server's reader/stderr threads,
    /// so the main loop wakes instead of polling for completion.
    pub(in crate::editor) fn new_threaded(wake: WakeCallback) -> Self {
        Self::with_backend(Box::new(ThreadedLspBackend::with_waker(wake)))
    }

    /// Test constructor: scripted responses, no process, no threads.
    #[cfg(test)]
    pub(in crate::editor) fn new_inline() -> Self {
        Self::with_backend(Box::new(InlineLspBackend::new()))
    }

    /// Reach the raw backend directly, to push server-initiated traffic.
    #[cfg(test)]
    pub(in crate::editor) fn backend_mut(&mut self) -> &mut dyn LspBackend {
        self.backend.as_mut()
    }

    /// Number of running server instances.
    #[cfg(test)]
    pub(in crate::editor) fn instance_count_for_test(&self) -> usize {
        self.instances.iter().count()
    }

    /// Every running instance of the server called `name`.
    #[cfg(test)]
    pub(in crate::editor) fn instances_named_for_test(&self, name: &str) -> Vec<ServerId> {
        ServerName::parse(name)
            .map(|name| self.instances.ids_named(&name))
            .unwrap_or_default()
    }

    /// The config `name` is registered with, or `None` if it is not.
    #[cfg(test)]
    fn config_for_test(&self, name: &str) -> Option<&registry::LspServerConfig> {
        let name = hume_scripting::ServerName::parse(name).ok()?;
        self.registry.get(&name)
    }

    #[cfg(test)]
    pub(in crate::editor) fn registered_command_for_test(&self, name: &str) -> Option<String> {
        self.config_for_test(name).map(|c| c.command.clone())
    }

    #[cfg(test)]
    pub(in crate::editor) fn registered_env_for_test(
        &self,
        name: &str,
    ) -> Option<Vec<(String, String)>> {
        self.config_for_test(name).map(|c| c.env.clone())
    }

    #[cfg(all(test, unix))]
    pub(in crate::editor) fn registered_settings_for_test(
        &self,
        name: &str,
    ) -> Option<serde_json::Value> {
        self.config_for_test(name).and_then(|c| c.settings.clone())
    }

    #[cfg(all(test, unix))]
    pub(in crate::editor) fn registered_init_options_for_test(
        &self,
        name: &str,
    ) -> Option<serde_json::Value> {
        self.config_for_test(name)
            .and_then(|c| c.init_options.clone())
    }

    /// `server`'s client, for a test that drives its state directly.
    #[cfg(test)]
    pub(in crate::editor) fn client_for_test(
        &mut self,
        server: ServerId,
    ) -> Option<&mut LspClient> {
        self.instances.get_mut(server).map(|i| &mut i.client)
    }

    /// The most recent active `$/progress` task's title for `server`. Lets
    /// tests assert the begin/report merge machine (title persists across a
    /// `report` that omits it) without going through `LspActivity`, which
    /// doesn't carry `title` (it's not rendered; see `introspect::activity`).
    #[cfg(test)]
    pub(in crate::editor) fn progress_title_for_test(&self, server: ServerId) -> Option<&str> {
        self.instances
            .get(server)?
            .progress
            .last()
            .map(|(_, task)| task.title.as_str())
    }

    /// Number of requests a delivery still waits on, a leak check: every one
    /// must be answered, timed out or cancelled (response, timeout, or
    /// teardown), never orphaned.
    #[cfg(test)]
    pub(in crate::editor) fn callback_count_for_test(&self) -> usize {
        self.deliveries.waiting_requests()
    }

    /// Number of publishes a drain deferred.
    #[cfg(test)]
    pub(in crate::editor) fn deferred_publishes_for_test(&self) -> usize {
        self.publishes.len()
    }

    /// Number of Steel deliveries still waiting for an answer, a leak check.
    #[cfg(test)]
    pub(in crate::editor) fn delivery_count_for_test(&self) -> usize {
        self.deliveries.len()
    }

    /// Disjoint-borrow accessor for callers that need to drive a client and
    /// its backend in the same call (`send_or_queue`, `cancel`).
    pub(in crate::editor) fn client_and_backend(
        &mut self,
        server: ServerId,
    ) -> Option<(&mut LspClient, &mut dyn LspBackend)> {
        let LspState {
            instances, backend, ..
        } = self;
        let client = &mut instances.get_mut(server)?.client;
        Some((client, backend.as_mut()))
    }

    /// Sends a request through `server`'s client. `None` if `server` is not
    /// a running instance.
    pub(in crate::editor) fn send_request(
        &mut self,
        server: ServerId,
        method: &str,
        params: serde_json::Value,
        meta: RequestMeta,
    ) -> Option<RequestId> {
        let (client, backend) = self.client_and_backend(server)?;
        Some(client.send_request(backend, method, params, meta))
    }
}

impl AsyncSource for LspState {
    fn next_wake(&self, now: Instant) -> Option<Instant> {
        // Deadlines only: response *arrival* needs no wake here: the
        // transport threads wake the event loop directly via
        // `termina::PlatformWaker` the moment a message lands. What remains
        // is the earliest pending-request timeout across every server
        // (initialize/shutdown included; see `LspClient::earliest_deadline`),
        // so a silent server's timeout sweep in `take_completed` still
        // fires promptly.
        let deadline = self
            .instances
            .iter()
            .filter_map(|(_, i)| i.client.earliest_deadline())
            .min();

        // A server mid-handshake or reporting `$/progress` (indexing,
        // loading, ...) needs the statusline spinner to keep animating, so
        // wake at the spinner's own cadence. `Starting` is included (not
        // just progress): without it the spinner freezes against
        // `initialize`'s 30s deadline.
        let spinner = self
            .has_animating_server()
            .then(|| now + progress::SPINNER_INTERVAL);

        // Publishes a drain left unresolved are due at once; input is still
        // polled first, so waiting on them never starves the keyboard.
        let deferred = (!self.publishes.is_empty()).then_some(now);

        [deadline, spinner, deferred].into_iter().flatten().min()
    }
}

impl Editor {
    /// `(errors, warnings)` for `bid` from the diagnostics store.
    #[cfg(test)]
    pub(in crate::editor) fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        introspect::diagnostic_counts(&self.state, bid)
    }

    /// `bid`'s attached servers' lifecycle/loading state.
    #[cfg(test)]
    pub(in crate::editor) fn lsp_activity(&self, bid: BufferId) -> introspect::LspActivity {
        introspect::activity(&self.state, bid)
    }
}
