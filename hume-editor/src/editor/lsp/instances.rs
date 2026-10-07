//! Running language-server processes. An instance exists while at least one
//! buffer's document is attached to it: `attach.rs` spawns one for the first
//! attachment and stops it when its last attachment detaches.

use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, ServerState};
use hume_rope::position_encoding::PositionEncoding;
use hume_scripting::{ServerName, ServerRef};

use super::progress::ProgressTask;
use super::registry::{LspServerConfig, Registry};

/// One running server process: its protocol client, the registration name
/// it was spawned under, and its `$/progress` state.
pub(in crate::editor::lsp) struct Instance {
    pub(in crate::editor::lsp) client: LspClient,
    pub(in crate::editor::lsp) name: ServerName,
    /// Active `$/progress` tasks, in begin order: a server can run more
    /// than one concurrently (e.g. rust-analyzer indexing + a flycheck run).
    /// The statusline shows the most recent (last); a token is removed on
    /// its `end` notification.
    pub(in crate::editor::lsp) progress: Vec<(String, ProgressTask)>,
    /// The config the process was started with. A registration of `name`
    /// that differs from it does not reach the running process.
    spawned: LspServerConfig,
    /// Set once the difference between `spawned` and the registration has
    /// been reported, so it is reported once.
    drift_reported: bool,
    /// Set once a diagnostic this server sent that did not parse has been
    /// reported at Warning.
    skipped_reported: bool,
    /// Set once the server has sent `publishDiagnostics`. A server that has
    /// not is asked for its diagnostics instead (`pull.rs`).
    pushed_diagnostics: bool,
}

impl Instance {
    /// The Steel value naming this instance, which is `sid` in the map.
    pub(in crate::editor::lsp) fn server_ref(&self, sid: ServerId) -> ServerRef {
        ServerRef {
            id: sid,
            name: self.name.clone(),
        }
    }
}

#[derive(Default)]
pub(in crate::editor::lsp) struct Instances {
    map: FxHashMap<ServerId, Instance>,
}

impl Instances {
    /// The instance of `name` serving the workspace at `root`, in any state.
    pub(in crate::editor::lsp) fn find(&self, name: &ServerName, root: &Path) -> Option<ServerId> {
        self.map
            .iter()
            .find(|(_, i)| i.name == *name && i.client.root() == root)
            .map(|(&sid, _)| sid)
    }

    /// Starts `config`'s command at `root` and begins its handshake.
    pub(in crate::editor::lsp) fn spawn(
        &mut self,
        backend: &mut dyn LspBackend,
        name: &ServerName,
        config: &LspServerConfig,
        root: PathBuf,
    ) -> std::io::Result<ServerId> {
        let sid = backend.start(&config.command, &config.args, &root, &config.env)?;
        let mut client = LspClient::new(sid, root);
        client.set_init_options(config.init_options.clone());
        client.set_settings(config.settings.clone());
        client.start_handshake(backend);
        self.map.insert(
            sid,
            Instance {
                client,
                name: name.clone(),
                progress: Vec::new(),
                spawned: config.clone(),
                drift_reported: false,
                skipped_reported: false,
                pushed_diagnostics: false,
            },
        );
        Ok(sid)
    }

    /// The names of servers whose registration now differs from what a
    /// running instance was started with and has not been reported yet,
    /// sorted. An instance whose registration matches again can be
    /// reported again if it drifts later.
    pub(in crate::editor::lsp) fn take_new_drift(
        &mut self,
        registry: &Registry,
    ) -> Vec<ServerName> {
        let mut names = Vec::new();
        for instance in self.map.values_mut() {
            let drifted = registry
                .get(&instance.name)
                .is_some_and(|config| *config != instance.spawned);
            if drifted && !instance.drift_reported {
                names.push(instance.name.clone());
            }
            instance.drift_reported = drifted;
        }
        names.sort();
        names.dedup();
        names
    }

    /// Whether this is the first report of skipped diagnostics for `sid`;
    /// the next one is not.
    pub(in crate::editor::lsp) fn first_skipped_report(&mut self, sid: ServerId) -> bool {
        self.map
            .get_mut(&sid)
            .is_some_and(|instance| !std::mem::replace(&mut instance.skipped_reported, true))
    }

    /// Records that `sid` pushes diagnostics.
    pub(in crate::editor::lsp) fn note_pushed_diagnostics(&mut self, sid: ServerId) {
        if let Some(instance) = self.map.get_mut(&sid) {
            instance.pushed_diagnostics = true;
        }
    }

    /// Whether `sid` has sent `publishDiagnostics`.
    pub(in crate::editor::lsp) fn has_pushed_diagnostics(&self, sid: ServerId) -> bool {
        self.map.get(&sid).is_some_and(|i| i.pushed_diagnostics)
    }

    pub(in crate::editor::lsp) fn remove(&mut self, sid: ServerId) -> Option<Instance> {
        self.map.remove(&sid)
    }

    pub(in crate::editor::lsp) fn get(&self, sid: ServerId) -> Option<&Instance> {
        self.map.get(&sid)
    }

    pub(in crate::editor::lsp) fn get_mut(&mut self, sid: ServerId) -> Option<&mut Instance> {
        self.map.get_mut(&sid)
    }

    pub(in crate::editor::lsp) fn iter(&self) -> impl Iterator<Item = (ServerId, &Instance)> {
        self.map.iter().map(|(&sid, i)| (sid, i))
    }

    pub(in crate::editor::lsp) fn ids(&self) -> Vec<ServerId> {
        self.map.keys().copied().collect()
    }

    /// Every running instance of `name`, one per workspace root.
    pub(in crate::editor::lsp) fn ids_named(&self, name: &ServerName) -> Vec<ServerId> {
        self.map
            .iter()
            .filter(|(_, i)| i.name == *name)
            .map(|(&sid, _)| sid)
            .collect()
    }

    /// Whether `sid` is an instance in the `Running` state.
    pub(in crate::editor::lsp) fn is_running(&self, sid: ServerId) -> bool {
        self.map
            .get(&sid)
            .is_some_and(|i| i.client.state() == ServerState::Running)
    }

    /// `sid`'s name and negotiated position encoding, or `None` if it is
    /// not an instance.
    pub(in crate::editor::lsp) fn origin(
        &self,
        sid: ServerId,
    ) -> Option<(ServerRef, PositionEncoding)> {
        let i = self.map.get(&sid)?;
        Some((i.server_ref(sid), i.client.encoding()))
    }

    /// The Steel value naming `sid`, or `None` if it is not an instance.
    pub(in crate::editor::lsp) fn server_ref(&self, sid: ServerId) -> Option<ServerRef> {
        self.map.get(&sid).map(|i| i.server_ref(sid))
    }
}
