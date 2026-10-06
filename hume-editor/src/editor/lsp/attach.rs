//! The one place a buffer's server attachments change. A buffer's
//! attachments are a function of its language, its path, the registry, the
//! language's server lists and the instances a stop ended:
//! [`EditorState::lsp_reconcile_buffer`]
//! recomputes them from those inputs, and is called whenever one of them
//! changes. A server instance lives while some buffer is attached to it:
//! every funnel here ends by stopping the instances its detaches left
//! unreferenced, so no instance outlives its last attachment.

use std::path::PathBuf;

use rustc_hash::FxHashSet;

use hume_engine::pipeline::{BufferId, EngineView};
use hume_lsp::backend::ServerId;
use hume_scripting::{FeatureFilter, LspFeature, LspServerTarget, PendingLspServerOp, ServerName};

use super::LspState;
use super::ReconcilePhase;
use super::document::{Attachment, OpenedAs};
use super::registry::{LspServerConfig, RootCache};
use crate::editor::event::EditorEvent;
use crate::editor::{Editor, EditorState, Severity};

/// Whether a detach fires `OnLspDetach`/`OnDiagnosticsChanged`. A buffer
/// being closed does not: its handlers would run after the buffer is gone,
/// and `OnBufferClose` already announces it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Announce {
    Hooks,
    BufferClosing,
}

/// What one reconcile run carries from buffer to buffer: the instances its
/// detaches left to stop, the workspace roots already resolved, and the
/// `(name, root)` pairs that failed to start, so a server that cannot start
/// is reported once for the run.
#[derive(Default)]
struct Pass {
    retired: Vec<ServerId>,
    roots: RootCache,
    failed: FxHashSet<(ServerName, PathBuf)>,
}

/// What stopping a target ended: the `(name, root)` of each instance
/// stopped, and the buffers that were attached to them, each once.
struct Stopped {
    servers: Vec<(ServerName, PathBuf)>,
    buffers: Vec<BufferId>,
}

/// One server a buffer should be attached to, before its instance is
/// looked up or spawned.
struct Wanted {
    name: ServerName,
    root: PathBuf,
    filter: FeatureFilter,
}

impl EditorState {
    /// Brings `bid`'s attachments in line with what its language plans:
    /// detaches (with `didClose`) every attachment the plan no longer
    /// names, attaches (with `didOpen`) every planned server it lacks,
    /// spawning an instance for a `(name, root)` none serves yet, updates a
    /// kept attachment's filter, and orders the attachments as planned. A
    /// buffer with no path or no language plans nothing. A change to the
    /// `languageId` or URI the servers were told (a language or path
    /// change) closes the document first, so every server reopens it as
    /// the new one.
    pub(in crate::editor) fn lsp_reconcile_buffer(&mut self, view: &EngineView, bid: BufferId) {
        let mut pass = Pass::default();
        self.lsp_reconcile_one(view, bid, &mut pass);
        self.lsp_stop_unreferenced(view, pass.retired);
    }

    /// [`Self::lsp_reconcile_buffer`] for every open buffer.
    pub(in crate::editor) fn lsp_reconcile_all(&mut self, view: &EngineView) {
        let bids: Vec<BufferId> = self.buffers.iter().map(|(bid, _)| bid).collect();
        let mut pass = Pass::default();
        for bid in bids {
            self.lsp_reconcile_one(view, bid, &mut pass);
        }
        self.lsp_stop_unreferenced(view, pass.retired);
    }

    /// Detaches every server from `bid`, with `didClose`, for a buffer about
    /// to be closed: no per-server hook fires for it.
    pub(in crate::editor) fn lsp_buffer_closing(&mut self, view: &EngineView, bid: BufferId) {
        let mut retired = Vec::new();
        self.lsp_detach_all(view, bid, Announce::BufferClosing, &mut retired);
        self.lsp_stop_unreferenced(view, retired);
    }

    /// Stops `target`'s servers: every buffer attached to one detaches from
    /// it, and the instance stops.
    fn lsp_stop_targets(&mut self, view: &EngineView, target: &LspServerTarget) -> Stopped {
        let sids: Vec<ServerId> = match target {
            LspServerTarget::Buffer(bid) => self.buffer_positions.lsp.servers(*bid).collect(),
            LspServerTarget::Name(name) => self.lsp.instances.ids_named(name),
        };
        let mut stopped = Stopped {
            servers: Vec::new(),
            buffers: Vec::new(),
        };
        for sid in sids {
            if let Some(instance) = self.lsp.instances.get(sid) {
                stopped
                    .servers
                    .push((instance.name.clone(), instance.client.root().to_path_buf()));
            }
            for bid in self.buffer_positions.lsp.buffers_of(sid) {
                self.lsp_detach(view, bid, sid, Announce::Hooks);
                if !stopped.buffers.contains(&bid) {
                    stopped.buffers.push(bid);
                }
            }
            self.lsp_stop_instance(view, sid);
        }
        stopped
    }

    /// Clears the stopped mark of the instances `target` names, so the next
    /// reconcile starts them: every root of a name, or the servers a
    /// buffer's language plans for it. Returns the marks it cleared.
    fn lsp_unstop(&mut self, target: &LspServerTarget) -> Vec<(ServerName, PathBuf)> {
        let cleared: Vec<(ServerName, PathBuf)> = match target {
            LspServerTarget::Name(name) => self
                .lsp
                .stopped
                .iter()
                .filter(|(stopped, _)| stopped == name)
                .cloned()
                .collect(),
            LspServerTarget::Buffer(bid) => self
                .lsp_planned(*bid, &mut RootCache::default())
                .map(|(planned, _)| planned)
                .unwrap_or_default()
                .into_iter()
                .map(|w| (w.name, w.root))
                .filter(|key| self.lsp.stopped.contains(key))
                .collect(),
        };
        for key in &cleared {
            self.lsp.stopped.remove(key);
        }
        cleared
    }

    /// Every `(buffer, server)` attachment whose server is `Running`.
    pub(in crate::editor) fn lsp_running_attachments(&self) -> Vec<(BufferId, ServerId)> {
        self.buffers
            .iter()
            .flat_map(|(bid, _)| {
                self.buffer_positions
                    .lsp
                    .servers(bid)
                    .map(move |sid| (bid, sid))
            })
            .filter(|&(_, sid)| self.lsp.instances.is_running(sid))
            .collect()
    }

    /// Queues `OnLspAttach` for `bid` and `sid` once `sid` is `Running`; a
    /// server still starting fires it for every attached buffer when its
    /// handshake completes.
    pub(in crate::editor) fn queue_lsp_attach(&mut self, bid: BufferId, sid: ServerId) {
        if !self.lsp.instances.is_running(sid) {
            return;
        }
        if let Some(server) = self.lsp.instances.server_ref(sid) {
            self.queue_event(EditorEvent::OnLspAttach {
                buffer: bid,
                server,
            });
        }
    }

    fn lsp_reconcile_one(&mut self, view: &EngineView, bid: BufferId, pass: &mut Pass) {
        if self.lsp.reconcile == ReconcilePhase::Suspended {
            return;
        }
        let Some((mut wanted, opened)) = self.lsp_planned(bid, &mut pass.roots) else {
            self.lsp_detach_all(view, bid, Announce::Hooks, &mut pass.retired);
            return;
        };
        wanted.retain(|w| !self.lsp.stopped.contains(&(w.name.clone(), w.root.clone())));
        if self
            .buffer_positions
            .lsp
            .opened_as(bid)
            .is_some_and(|current| *current != opened)
        {
            self.lsp_detach_all(view, bid, Announce::Hooks, &mut pass.retired);
        }

        let attached: Vec<ServerId> = self.buffer_positions.lsp.servers(bid).collect();
        for sid in attached {
            let still_wanted = self.lsp.instances.get(sid).is_some_and(|inst| {
                wanted
                    .iter()
                    .any(|w| w.name == inst.name && w.root == inst.client.root())
            });
            if !still_wanted {
                self.lsp_detach(view, bid, sid, Announce::Hooks);
                pass.retired.push(sid);
            }
        }

        let mut order = Vec::with_capacity(wanted.len());
        for w in wanted {
            let Some(sid) = self.lsp_instance_for(&w, &mut pass.failed) else {
                continue;
            };
            match self.buffer_positions.lsp.filter_of(bid, sid) {
                None => self.lsp_attach(bid, sid, w.filter, &opened),
                Some(filter) if filter != w.filter => self.lsp_set_filter(bid, sid, w.filter),
                Some(_) => {}
            }
            order.push(sid);
        }
        self.buffer_positions.lsp.reorder(bid, &order);
    }

    /// What `bid`'s language plans for it, stopped servers included, and what
    /// its servers are told it is; `None` for a buffer with no language or no path that converts to
    /// a URI, which attaches to nothing.
    fn lsp_planned(&self, bid: BufferId, roots: &mut RootCache) -> Option<(Vec<Wanted>, OpenedAs)> {
        let buf = self.buffers.try_get(bid)?;
        let path = buf.path()?;
        let lang = buf.language?;
        let uri = self.lsp_doc_uri(bid)?;
        let language = self.config.languages.name_of(lang);
        let opened = OpenedAs {
            language_id: self.config.languages.lsp_language_id_of(lang).to_owned(),
            uri: uri.as_str().to_owned(),
        };
        let language_roots = self.config.languages.roots_of(lang);
        let wanted = self
            .lsp
            .registry
            .plan(language)
            .into_iter()
            .map(|p| Wanted {
                name: p.name.clone(),
                root: roots.root_for(language_roots, path, &self.cwd),
                filter: p.filter,
            })
            .collect();
        Some((wanted, opened))
    }

    /// The instance serving `wanted`, spawning it if none does. `None` when
    /// it fails to start, which is reported the first time it fails in a
    /// run (`failed` holds the ones that already did).
    fn lsp_instance_for(
        &mut self,
        wanted: &Wanted,
        failed: &mut FxHashSet<(ServerName, PathBuf)>,
    ) -> Option<ServerId> {
        let LspState {
            registry,
            instances,
            backend,
            ..
        } = &mut self.lsp;
        if let Some(sid) = instances.find(&wanted.name, &wanted.root) {
            return Some(sid);
        }
        let key = (wanted.name.clone(), wanted.root.clone());
        if failed.contains(&key) {
            return None;
        }
        let config = registry.get(&wanted.name)?;
        let spawned = instances
            .spawn(backend.as_mut(), &wanted.name, config, wanted.root.clone())
            .map_err(|e| format!("lsp: failed to start '{}': {e}", config.command));
        match spawned {
            Ok(sid) => Some(sid),
            Err(msg) => {
                failed.insert(key);
                self.report(Severity::Error, msg);
                None
            }
        }
    }

    /// Attaches `bid` to `sid`. Flushes `bid`'s queued changes to the
    /// servers already attached first, so `sid`, which starts from the
    /// `didOpen` text, never receives a change that text already contains.
    fn lsp_attach(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        filter: FeatureFilter,
        opened: &OpenedAs,
    ) {
        self.lsp_flush_buffer(bid);
        self.lsp_did_open_to(bid, sid, opened);
        self.buffer_positions
            .lsp
            .push(bid, Attachment::new(sid, filter), opened);
        self.queue_lsp_attach(bid, sid);
    }

    /// A kept attachment's new filter. A filter that no longer admits
    /// diagnostics drops what that server published for `bid`; one that
    /// admits them again shows them from the server's next publish. The
    /// attachment's trigger tables are emptied by the change, so
    /// `OnLspAttach` fires again for plugins to register what the new filter
    /// admits.
    fn lsp_set_filter(&mut self, bid: BufferId, sid: ServerId, filter: FeatureFilter) {
        self.buffer_positions.lsp.set_filter(bid, sid, filter);
        self.queue_lsp_attach(bid, sid);
        if !filter.admits(LspFeature::Diagnostics)
            && self
                .buffer_positions
                .diagnostics
                .remove_source_for_buffer(sid, bid)
        {
            self.queue_event(EditorEvent::OnDiagnosticsChanged { buffer: bid });
        }
    }

    /// Detaches `bid` from `sid` and leaves the instance running: the
    /// funnel calling this stops it once no buffer uses it.
    fn lsp_detach(&mut self, view: &EngineView, bid: BufferId, sid: ServerId, announce: Announce) {
        self.lsp_flush_buffer(bid);
        self.lsp_did_close_to(bid, sid);
        if !self.buffer_positions.lsp.remove(bid, sid) {
            return;
        }
        let dropped = self
            .buffer_positions
            .diagnostics
            .remove_source_for_buffer(sid, bid);
        if self
            .input
            .buffer_completion()
            .is_some_and(|session| session.bid() == bid)
        {
            self.dismiss_completion(view);
        }
        if announce == Announce::BufferClosing {
            return;
        }
        if dropped {
            self.queue_event(EditorEvent::OnDiagnosticsChanged { buffer: bid });
        }
        if let Some(server) = self.lsp.instances.server_ref(sid) {
            self.queue_event(EditorEvent::OnLspDetach {
                buffer: bid,
                server,
            });
        }
    }

    fn lsp_detach_all(
        &mut self,
        view: &EngineView,
        bid: BufferId,
        announce: Announce,
        retired: &mut Vec<ServerId>,
    ) {
        let sids: Vec<ServerId> = self.buffer_positions.lsp.servers(bid).collect();
        for sid in sids {
            self.lsp_detach(view, bid, sid, announce);
            retired.push(sid);
        }
    }

    fn lsp_stop_unreferenced(&mut self, view: &EngineView, candidates: Vec<ServerId>) {
        for sid in candidates {
            if !self.buffer_positions.lsp.referenced(sid) {
                self.lsp_stop_instance(view, sid);
            }
        }
    }

    /// Shuts `sid` down and forgets it. Every request it still owes an
    /// answer completes first, a landed response with its own outcome and
    /// the rest as timed out, so no callback outlives its server.
    fn lsp_stop_instance(&mut self, view: &EngineView, sid: ServerId) {
        let LspState {
            instances,
            backend,
            publishes,
            ..
        } = &mut self.lsp;
        let Some(mut instance) = instances.remove(sid) else {
            return;
        };
        publishes.forget_server(sid);
        instance.client.begin_shutdown(backend.as_mut());
        let (completed, _actions) = instance
            .client
            .take_completed(backend.as_mut(), std::time::Instant::now());
        let pending = instance.client.drain_pending();
        backend.shutdown(sid);
        for (id, meta, outcome) in completed {
            self.lsp_dispatch_completed(view, sid, id, meta, outcome);
        }
        for (id, meta, outcome) in pending {
            self.lsp_dispatch_completed(view, sid, id, meta, outcome);
        }
    }
}

impl Editor {
    /// `op` as a batch of one, for tests driving a single op.
    #[cfg(test)]
    pub(in crate::editor) fn apply_lsp_server_op(&mut self, op: PendingLspServerOp) {
        self.apply_lsp_server_ops(vec![op]);
    }

    /// Applies `ops` in order. Registry changes (registration, list) take
    /// effect at once, but buffers are reconciled to them once per run of
    /// them, before the next op that is not one and after the last, so a
    /// server a later op in the run drops is never started. An unregister
    /// stops the name's running servers where it falls in the run, so a
    /// register of the same name after it spawns a fresh one.
    pub(in crate::editor) fn apply_lsp_server_ops(&mut self, ops: Vec<PendingLspServerOp>) {
        let mut unreconciled = false;
        for op in ops {
            let registry_change = match &op {
                PendingLspServerOp::Register(_)
                | PendingLspServerOp::Unregister { .. }
                | PendingLspServerOp::SetLanguageServers { .. } => true,
                PendingLspServerOp::Stop { .. }
                | PendingLspServerOp::Restart { .. }
                | PendingLspServerOp::ShowStatus => false,
            };
            if !registry_change && std::mem::take(&mut unreconciled) {
                self.state.lsp_reconcile_all(&self.view);
            }
            unreconciled |= registry_change;
            self.apply_lsp_server_op_unreconciled(op);
        }
        if unreconciled {
            self.state.lsp_reconcile_all(&self.view);
        }
        self.report_config_drift();
    }

    /// Warns once for each running server whose registration differs from
    /// the process it runs: a registration reaches a server only when it
    /// next spawns.
    fn report_config_drift(&mut self) {
        let LspState {
            instances,
            registry,
            ..
        } = &mut self.state.lsp;
        for name in instances.take_new_drift(registry) {
            self.report(
                Severity::Warning,
                format!(
                    "lsp: '{name}' was registered with a different command, arguments, \
                     environment or settings; the running server keeps what it started \
                     with until :lsp-restart {name}"
                ),
            );
        }
    }

    /// `op`'s own effect, leaving buffers to be reconciled to a registry
    /// change by the caller.
    fn apply_lsp_server_op_unreconciled(&mut self, op: PendingLspServerOp) {
        match op {
            PendingLspServerOp::Register(reg) => {
                let name = reg.name.clone();
                self.state.lsp.forget_stopped(&name);
                let replaced = self.state.lsp.registry.register(
                    reg.name,
                    LspServerConfig {
                        command: reg.command,
                        args: reg.args,
                        init_options: reg.init_options,
                        settings: reg.settings,
                        env: reg.env,
                    },
                );
                if replaced {
                    self.report(
                        Severity::Trace,
                        format!("register-lsp-server!: replaced registration for '{name}'"),
                    );
                }
            }
            PendingLspServerOp::Unregister { name } => {
                self.state
                    .lsp_stop_targets(&self.view, &LspServerTarget::Name(name.clone()));
                self.state.lsp.registry.unregister(&name);
                self.state.lsp.forget_stopped(&name);
            }
            PendingLspServerOp::SetLanguageServers {
                language,
                layer,
                entries,
            } => {
                self.state.lsp.registry.set_list(language, layer, entries);
            }
            PendingLspServerOp::Stop { target } => {
                self.stop_servers(&target, "stop", "stopped", false);
            }
            PendingLspServerOp::Restart { target } => {
                self.stop_servers(&target, "restart", "restarted", true);
            }
            PendingLspServerOp::ShowStatus => {
                let content = self.lsp_status_text();
                // Applied from the effect log after the eval that queued it,
                // so the pane focused now is where the status view opens.
                let fp = crate::editor::commands::FocusedPane::current(&self.state);
                self.open_read_only_view(
                    fp,
                    "[lsp-status]",
                    &content,
                    Some(hume_rope::line::ContentLine::new(0)),
                );
            }
        }
    }

    /// Stops `target`'s servers and reports it with `verb` (nothing matched)
    /// or `past` (n servers). A `restart` reconciles the buffers they were
    /// attached to, which starts their servers again.
    fn stop_servers(&mut self, target: &LspServerTarget, verb: &str, past: &str, restart: bool) {
        if self.report_unknown_target(target) {
            return;
        }
        let mut ended = self.state.lsp_stop_targets(&self.view, target).servers;
        if restart {
            for key in self.state.lsp_unstop(target) {
                if !ended.contains(&key) {
                    ended.push(key);
                }
            }
            self.state.lsp_reconcile_all(&self.view);
        } else {
            self.state.lsp.stopped.extend(ended.iter().cloned());
        }
        let n = ended.len();
        let text = if n == 0 {
            format!("lsp: no matching server to {verb}")
        } else {
            format!("lsp: {past} {n} server(s)")
        };
        self.report(Severity::Info, text);
    }

    /// Reports a name target that is neither registered nor running, and
    /// says whether it did.
    fn report_unknown_target(&mut self, target: &LspServerTarget) -> bool {
        let LspServerTarget::Name(name) = target else {
            return false;
        };
        let known = self.state.lsp.registry.contains(name)
            || !self.state.lsp.instances.ids_named(name).is_empty();
        if !known {
            self.report(Severity::Error, format!("lsp: no server named '{name}'"));
        }
        !known
    }
}
