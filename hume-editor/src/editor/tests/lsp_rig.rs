//! The LSP test fixture: servers registered through `register-lsp-server!` and
//! listed through `set-language-servers!`,
//! a real file opened through `:e`, and a scripted backend, so every
//! attachment a test sees was made by the production attach path.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::codec::Message;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog, ResponseLog, ServerNotificationLog};
use hume_lsp::transport::InboundEvent;
use hume_scripting::ScriptingHost;

use super::{Editor, eval_with_real_host, install_source, select_marked};
use crate::editor::lsp::LspState;
use test_fixtures::testing::parse_state;

/// The `file://` URI of `path`.
pub(in crate::editor) fn file_uri(path: &Path) -> String {
    hume_lsp::uri::path_to_uri(path)
        .expect("path converts")
        .as_str()
        .to_string()
}

/// One `rust-analyzer` server serving `rust` buffers, rooted at the nearest
/// `Cargo.toml`: the setup most tests need.
pub(in crate::editor) const RUST_ANALYZER: &str = r#"(%define-language! "rust" '("rs") '() '() #f '("Cargo.toml"))
(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(set-language-servers! "rust" '("rust-analyzer"))"#;

/// `ra-lint`, registered and listed after `rust-analyzer` on `rust`
/// buffers: the second server beside [`RUST_ANALYZER`], which it follows.
pub(in crate::editor) const RA_LINT: &str = r#"(register-lsp-server! "ra-lint" #:command "ra-lint")
(set-language-servers! "rust" '("rust-analyzer" "ra-lint"))"#;

/// [`RUST_ANALYZER`] then [`RA_LINT`].
pub(in crate::editor) const TWO_RUST_SERVERS: &str = r#"(%define-language! "rust" '("rs") '() '() #f '("Cargo.toml"))
(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(register-lsp-server! "ra-lint" #:command "ra-lint")
(set-language-servers! "rust" '("rust-analyzer" "ra-lint"))"#;

/// Stops every instance of the server called `name`, as `:lsp-stop name`
/// does.
pub(in crate::editor) fn stop_server(ed: &mut Editor, name: &str) {
    ed.apply_lsp_server_op(hume_scripting::PendingLspServerOp::Stop {
        target: hume_scripting::LspServerTarget::Name(
            hume_scripting::ServerName::parse(name).unwrap(),
        ),
    });
}

/// What to set up: a language identity, the file to open under the rig's
/// root, and the Scheme evaluated before it opens (registrations, plugins).
pub(in crate::editor) struct RigSpec<'a> {
    pub(in crate::editor) language: &'a str,
    pub(in crate::editor) extension: &'a str,
    /// Relative to the rig's root.
    pub(in crate::editor) file: &'a str,
    /// The file's text with selection markers, as `editor_from` takes it.
    pub(in crate::editor) marked: &'a str,
    /// Empty files created at the root, for root-marker resolution.
    pub(in crate::editor) markers: &'a [&'a str],
    pub(in crate::editor) init: &'a str,
}

impl<'a> RigSpec<'a> {
    /// `src/main.rs` with `marked`'s text, under a `Cargo.toml` root, with
    /// [`RUST_ANALYZER`] registered.
    pub(in crate::editor) fn rust(marked: &'a str) -> Self {
        Self {
            language: "rust",
            extension: "rs",
            file: "src/main.rs",
            marked,
            markers: &["Cargo.toml"],
            init: RUST_ANALYZER,
        }
    }

    pub(in crate::editor) fn with_init(self, init: &'a str) -> Self {
        Self { init, ..self }
    }
}

pub(in crate::editor) struct LspRig {
    pub(in crate::editor) ed: Editor,
    pub(in crate::editor) bid: BufferId,
    pub(in crate::editor) root: PathBuf,
    pub(in crate::editor) notifications: ServerNotificationLog,
    pub(in crate::editor) requests: RequestLog,
    pub(in crate::editor) responses: ResponseLog,
    /// Probes run so far: each defines its own command name, spelled in
    /// letters because a command name ending in digits does not resolve.
    probes: usize,
}

impl LspRig {
    /// Registers `spec.language`, evaluates `spec.init`, and opens
    /// `spec.file` with `spec.marked`'s selections. Not drained: every
    /// server spawned for the file is still `Starting`.
    pub(in crate::editor) fn open(
        tmp: &Path,
        spec: RigSpec<'_>,
        backend: RecordingLspBackend,
    ) -> Self {
        let root = std::fs::canonicalize(tmp).expect("tempdir canonicalizes");
        for marker in spec.markers {
            std::fs::write(root.join(marker), b"").expect("write root marker");
        }
        let file = root.join(spec.file);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).expect("create file dir");
        }
        let (text, _) = parse_state(spec.marked);
        std::fs::write(&file, text.to_string()).expect("write file");

        let notifications = backend.server_notification_log();
        let requests = backend.request_log();
        let responses = backend.response_log();
        let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).expect("editor opens");
        ed.state.lsp = LspState::with_backend(Box::new(backend));
        ed.state
            .config
            .languages
            .register_identity(spec.language, &[spec.extension], &[], &[], None)
            .expect("register test language");

        let mut host = ScriptingHost::new();
        eval_with_real_host(&mut ed, &mut host, spec.init, &root);
        ed.scripting = Some(host);
        ed.execute_typed("e", Some(file.to_str().expect("utf-8 path")))
            .expect(":e opens the file");
        let bid = ed.focused_buffer_id();
        select_marked(&mut ed, spec.marked);
        Self {
            ed,
            bid,
            root,
            notifications,
            requests,
            responses,
            probes: 0,
        }
    }

    /// [`Self::open`], drained once, so every server that answers
    /// `initialize` is `Running` and has the file open.
    pub(in crate::editor) fn drained(
        tmp: &Path,
        spec: RigSpec<'_>,
        backend: RecordingLspBackend,
    ) -> Self {
        let mut rig = Self::open(tmp, spec, backend);
        rig.ed.drain_lsp();
        rig
    }

    /// One `rust-analyzer` server, answering `initialize` with
    /// `initialize_result`, `Running` on `src/main.rs`.
    pub(in crate::editor) fn rust(
        tmp: &Path,
        marked: &str,
        initialize_result: serde_json::Value,
    ) -> Self {
        let (mut backend, _, _) = RecordingLspBackend::new();
        backend.respond_to("initialize", initialize_result);
        Self::drained(tmp, RigSpec::rust(marked), backend)
    }

    /// Evaluates `source` in the rig's scripting host, as `init.scm` would.
    pub(in crate::editor) fn eval(&mut self, source: &str) {
        let host = self.ed.scripting.take().expect("the rig installs a host");
        install_source(&mut self.ed, host, source, &self.root);
    }

    /// Stops every instance of the server called `name`.
    pub(in crate::editor) fn stop(&mut self, name: &str) {
        stop_server(&mut self.ed, name);
    }

    /// The one running instance of the server called `name`.
    pub(in crate::editor) fn sid(&self, name: &str) -> ServerId {
        match self.ed.state.lsp.instances_named_for_test(name).as_slice() {
            [sid] => *sid,
            other => panic!("expected one instance of {name}, found {other:?}"),
        }
    }

    /// Runs `body`, a Scheme expression over `pane` (the focused pane), as
    /// a typed command, then settles, so every request it sends is answered
    /// and every callback has run.
    pub(in crate::editor) fn probe(&mut self, body: &str) {
        self.probes += 1;
        let suffix: String = std::iter::successors(Some(self.probes), |n| Some(n / 26))
            .take_while(|&n| n > 0)
            .map(|n| char::from(b'a' + (n % 26) as u8))
            .collect();
        let name = format!("probe-{suffix}");
        self.eval(&format!(
            r#"(define-typed-command! "{name}" "" (lambda (pane) {body}))"#
        ));
        super::type_cmd(&mut self.ed, &format!(":{name}"));
        self.ed.settle();
    }

    /// The text of every Warning the log holds, oldest first: what a probe
    /// reports through `(log! 'warn …)`.
    pub(in crate::editor) fn warnings(&self) -> Vec<String> {
        self.ed
            .state
            .message_log
            .entries()
            .filter(|e| e.severity == crate::editor::Severity::Warning)
            .map(|e| e.text.clone())
            .collect()
    }

    /// Every request sent to `sid` with `method`, in order: its params.
    pub(in crate::editor) fn requests_to(
        &self,
        sid: ServerId,
        method: &str,
    ) -> Vec<serde_json::Value> {
        self.requests
            .borrow()
            .iter()
            .filter(|(s, m, _)| *s == sid && m == method)
            .map(|(_, _, params)| params.clone())
            .collect()
    }

    /// The servers the rig's buffer is attached to, in order.
    pub(in crate::editor) fn attached(&self) -> Vec<ServerId> {
        self.ed
            .state
            .buffer_positions
            .lsp
            .servers(self.bid)
            .collect()
    }

    /// Every notification sent to `sid` with `method`, in order.
    pub(in crate::editor) fn sent(&self, sid: ServerId, method: &str) -> Vec<serde_json::Value> {
        self.notifications
            .borrow()
            .iter()
            .filter(|(s, m, _)| *s == sid && m == method)
            .map(|(_, _, params)| params.clone())
            .collect()
    }

    /// The rig's file URI.
    pub(in crate::editor) fn uri(&self) -> String {
        let path = self
            .ed
            .state
            .buffers
            .get(self.bid)
            .path()
            .expect("the rig's buffer has a path");
        file_uri(path)
    }

    /// Delivers `msg` from `sid` as if it had just arrived.
    pub(in crate::editor) fn push(&mut self, sid: ServerId, msg: Message) {
        self.deliver(sid, InboundEvent::Message(msg));
    }

    /// `sid` publishes `diagnostics` (a wire `Diagnostic[]`) for the rig's
    /// file.
    pub(in crate::editor) fn publish(&mut self, sid: ServerId, diagnostics: serde_json::Value) {
        let params = serde_json::json!({ "uri": self.uri(), "diagnostics": diagnostics });
        self.push(
            sid,
            Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params,
            },
        );
    }

    /// `sid`'s connection dies.
    pub(in crate::editor) fn crash(&mut self, sid: ServerId) {
        self.deliver(sid, InboundEvent::Eof { error: None });
    }

    fn deliver(&mut self, sid: ServerId, event: InboundEvent) {
        let actions = self
            .ed
            .state
            .lsp
            .client_for_test(sid)
            .expect("a running instance")
            .on_event(event);
        for action in actions {
            self.ed.dispatch_lsp_action(sid, action);
        }
        self.ed.ingest_published();
    }
}
