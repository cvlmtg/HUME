// Registration by name, per-language server lists, instance sharing by
// (name, root), and instance lifetime: alive exactly while some buffer is
// attached to it.

use super::lsp_rig::{LspRig, RigSpec, TWO_RUST_SERVERS};
use super::*;
use crate::editor::lsp::introspect;
use hume_lsp::test_util::RecordingLspBackend;
use hume_scripting::{FeatureFilter, LspServerTarget, PendingLspServerOp, ServerName};

fn answering(n: usize) -> RecordingLspBackend {
    let (mut backend, _, _) = RecordingLspBackend::new();
    for _ in 0..n {
        backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    }
    backend
}

fn two_servers(tmp: &tempfile::TempDir) -> LspRig {
    LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(TWO_RUST_SERVERS),
        answering(2),
    )
}

fn name(s: &str) -> ServerName {
    ServerName::parse(s).unwrap()
}

/// Opens `relative` (holding `text`) under the rig's root and returns its
/// buffer.
fn open(rig: &mut LspRig, relative: &str, text: &str) -> BufferId {
    let path = rig.root.join(relative);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&path, text).unwrap();
    rig.ed
        .execute_typed("e", Some(path.to_str().unwrap()))
        .unwrap();
    rig.ed.focused_buffer_id()
}

fn servers_of(rig: &LspRig, bid: BufferId) -> Vec<hume_lsp::backend::ServerId> {
    rig.ed.state.buffer_positions.lsp.servers(bid).collect()
}

#[test]
fn one_instance_serves_every_language_of_the_same_name_and_root() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = RigSpec {
        language: "typescript",
        extension: "ts",
        file: "src/a.ts",
        marked: "-[x]>\n",
        markers: &["package.json"],
        init: r#"(%define-language! "typescript" '("ts") '() '() #f '("package.json"))
(%define-language! "tsx" '("tsx") '() '() #f '("package.json"))
(register-lsp-server! "tsls" #:command "tsls")
(set-language-servers! "typescript" '("tsls"))
(set-language-servers! "tsx" '("tsls"))"#,
        dirs: hume_platform::dirs::Dirs::none(),
    };
    let mut rig = LspRig::drained(tmp.path(), spec, answering(1));

    let tsx = open(&mut rig, "src/b.tsx", "y\n");

    let sid = rig.sid("tsls");
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
    assert_eq!(servers_of(&rig, rig.bid), vec![sid]);
    assert_eq!(servers_of(&rig, tsx), vec![sid]);
    let languages: Vec<String> = rig
        .sent(sid, "textDocument/didOpen")
        .iter()
        .map(|p| {
            p["textDocument"]["languageId"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(languages, vec!["typescript", "tsx"]);
}

#[test]
fn two_names_for_one_language_attach_both_in_list_order() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = two_servers(&tmp);

    assert_eq!(
        rig.attached(),
        vec![rig.sid("rust-analyzer"), rig.sid("ra-lint")]
    );
    for server in ["rust-analyzer", "ra-lint"] {
        assert_eq!(rig.sent(rig.sid(server), "textDocument/didOpen").len(), 1);
    }
}

#[test]
fn unregistering_an_attached_server_leaves_the_other_attached() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = two_servers(&tmp);
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    rig.eval(r#"(unregister-lsp-server! "ra-lint")"#);

    assert_eq!(rig.attached(), vec![ra]);
    assert_eq!(rig.sent(lint, "textDocument/didClose").len(), 1);
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

#[test]
fn stop_by_name_stops_every_root_of_that_server() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    std::fs::create_dir_all(rig.root.join("other")).unwrap();
    std::fs::write(rig.root.join("other/Cargo.toml"), b"").unwrap();
    let other = open(&mut rig, "other/src/main.rs", "fn main() {}\n");
    assert_eq!(
        rig.ed
            .state
            .lsp
            .instances_named_for_test("rust-analyzer")
            .len(),
        2
    );

    rig.ed.apply_lsp_server_op(PendingLspServerOp::Stop {
        target: LspServerTarget::Name(name("rust-analyzer")),
    });

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
    assert!(servers_of(&rig, rig.bid).is_empty());
    assert!(servers_of(&rig, other).is_empty());
    assert_eq!(
        rig.ed.state.status_msg.as_deref(),
        Some("lsp: stopped 2 server(s)")
    );
}

#[test]
fn clearing_a_buffers_language_detaches_every_server() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = two_servers(&tmp);
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    rig.ed.set_buffer_language(rig.bid, None);

    assert!(rig.attached().is_empty());
    for sid in [ra, lint] {
        assert_eq!(rig.sent(sid, "textDocument/didClose").len(), 1, "{sid:?}");
    }
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
}

#[test]
fn server_dropped_from_the_only_buffers_list_stops_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = two_servers(&tmp);

    rig.eval(r#"(set-language-servers! "rust" '("rust-analyzer"))"#);

    assert!(
        rig.ed
            .state
            .lsp
            .instances_named_for_test("ra-lint")
            .is_empty()
    );
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

#[test]
fn closing_the_last_buffer_of_a_server_stops_it_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = two_servers(&tmp);

    rig.ed.close_buffer(rig.bid);

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
}

#[test]
fn a_crashed_server_without_buffers_is_stopped_by_its_last_detach() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");
    rig.crash(sid);
    assert_eq!(
        rig.attached(),
        vec![sid],
        "a crashed instance keeps its buffers"
    );

    rig.ed.close_buffer(rig.bid);

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
}

#[test]
fn reset_config_clears_lists_and_registrations() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = two_servers(&tmp);
    rig.eval(r#"(set-language-servers! "rust" '("ra-lint"))"#);

    rig.ed.state.lsp.reset_config();
    assert!(introspect::language_servers(&rig.ed.state.lsp, "rust").is_empty());
    rig.ed.state.lsp.resume_reconcile();
    rig.eval(
        r#"(register-lsp-server! "ra-lint" #:command "ra-lint")
           (register-lsp-server! "rust-analyzer" #:command "rust-analyzer")"#,
    );
    assert!(
        introspect::language_servers(&rig.ed.state.lsp, "rust").is_empty(),
        "registrations alone name no language"
    );

    rig.eval(r#"(set-language-servers! "rust" '("ra-lint" "rust-analyzer"))"#);

    let planned: Vec<String> = introspect::language_servers(&rig.ed.state.lsp, "rust")
        .into_iter()
        .map(|e| {
            assert_eq!(e.filter, FeatureFilter::All);
            e.name.to_string()
        })
        .collect();
    assert_eq!(planned, vec!["ra-lint", "rust-analyzer"]);
}

#[test]
fn save_as_closes_the_old_uri_and_opens_the_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");
    let old_uri = rig.uri();
    let new_path = rig.root.join("src/renamed.rs");

    rig.ed
        .execute_typed("w", Some(new_path.to_str().unwrap()))
        .unwrap();

    let new_uri = rig.uri();
    assert_ne!(old_uri, new_uri);
    assert_eq!(
        rig.sid("rust-analyzer"),
        sid,
        "the instance survives the rename"
    );
    let closed = rig.sent(sid, "textDocument/didClose");
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["textDocument"]["uri"], old_uri.as_str());
    let opened: Vec<_> = rig
        .sent(sid, "textDocument/didOpen")
        .iter()
        .map(|p| p["textDocument"]["uri"].clone())
        .collect();
    assert_eq!(opened, vec![old_uri.as_str(), new_uri.as_str()]);
    let saved = rig.sent(sid, "textDocument/didSave");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["textDocument"]["uri"], new_uri.as_str());
}

/// Unregistering a name and registering it again in one eval is the way to
/// get a fresh process: the instance stops and a new one serves the buffer.
#[test]
fn unregister_then_register_in_one_eval_spawns_a_fresh_server() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n"),
        answering(2),
    );
    let original = rig.sid("rust-analyzer");

    rig.eval(
        r#"(unregister-lsp-server! "rust-analyzer")
           (register-lsp-server! "rust-analyzer" #:command "rust-analyzer")"#,
    );
    rig.ed.drain_lsp();

    let fresh = rig.sid("rust-analyzer");
    assert_ne!(
        fresh, original,
        "the unregister must have replaced the process"
    );
    assert_eq!(rig.attached(), vec![fresh]);
    assert_eq!(rig.sent(original, "textDocument/didClose").len(), 1);
    let initializes = rig
        .requests
        .borrow()
        .iter()
        .filter(|(_, method, _)| method == "initialize")
        .count();
    assert_eq!(initializes, 2);
}

/// Registrations and a list change queued by one eval apply together: a
/// server the final list leaves out is never started on the way there.
#[test]
fn one_evals_server_ops_reconcile_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(""),
        answering(2),
    );

    rig.eval(&format!(
        "{TWO_RUST_SERVERS}\n(set-language-servers! \"rust\" '(\"rust-analyzer\"))"
    ));

    let initializes = rig
        .requests
        .borrow()
        .iter()
        .filter(|(_, method, _)| method == "initialize")
        .count();
    assert_eq!(initializes, 1);
}

/// A server that cannot start is reported once for the reconcile that
/// tried it, not once per buffer of its language.
#[test]
fn a_server_that_fails_to_start_is_reported_once_for_all_its_buffers() {
    let tmp = tempfile::tempdir().unwrap();
    let mut backend = answering(0);
    backend.refuse_start("ghost");
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(""),
        backend,
    );
    open(&mut rig, "src/other.rs", "fn other() {}\n");

    rig.eval(
        r#"(register-lsp-server! "ghost" #:command "ghost")
           (set-language-servers! "rust" '("ghost"))"#,
    );

    let log = rig.ed.state.message_log.format_for_display();
    assert_eq!(
        log.matches("lsp: failed to start 'ghost'").count(),
        1,
        "{log:?}"
    );
}

/// A language change that alters the `languageId` the servers were told
/// closes the document and opens it again as the new language, on the same
/// instance.
#[test]
fn a_language_change_reopens_the_document_on_the_same_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = RigSpec {
        language: "typescript",
        extension: "ts",
        file: "src/a.ts",
        marked: "-[x]>\n",
        markers: &["package.json"],
        init: r#"(%define-language! "typescript" '("ts") '() '() #f '("package.json"))
(%define-language! "tsx" '("tsx") '() '() #f '("package.json"))
(register-lsp-server! "tsls" #:command "tsls")
(set-language-servers! "typescript" '("tsls"))
(set-language-servers! "tsx" '("tsls"))"#,
        dirs: hume_platform::dirs::Dirs::none(),
    };
    let mut rig = LspRig::drained(tmp.path(), spec, answering(1));
    let sid = rig.sid("tsls");

    rig.ed.set_buffer_language_by_name(rig.bid, Some("tsx"));

    assert_eq!(rig.sid("tsls"), sid);
    assert_eq!(rig.attached(), vec![sid]);
    assert_eq!(rig.sent(sid, "textDocument/didClose").len(), 1);
    let languages: Vec<String> = rig
        .sent(sid, "textDocument/didOpen")
        .iter()
        .map(|p| {
            p["textDocument"]["languageId"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(languages, vec!["typescript", "tsx"]);
}

// A stopped server stays stopped: no reconcile starts it again until it is
// restarted or its name is registered again.

fn rust_rig(tmp: &tempfile::TempDir) -> LspRig {
    LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n"),
        answering(4),
    )
}

fn restart(rig: &mut LspRig, target: LspServerTarget) {
    rig.ed
        .apply_lsp_server_op(PendingLspServerOp::Restart { target });
}

#[test]
fn a_stopped_server_does_not_start_for_a_buffer_opened_later() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.stop("rust-analyzer");

    let other = open(&mut rig, "src/other.rs", "fn other() {}\n");

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
    assert!(servers_of(&rig, other).is_empty());
}

#[test]
fn a_stopped_server_does_not_start_for_a_registry_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.stop("rust-analyzer");

    rig.eval(r#"(set-language-servers! "rust" '("rust-analyzer"))"#);

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
    assert!(rig.attached().is_empty());
}

#[test]
fn restarting_a_stopped_server_by_name_starts_it_for_its_buffers() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.stop("rust-analyzer");

    restart(&mut rig, LspServerTarget::Name(name("rust-analyzer")));

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
    assert_eq!(rig.attached(), vec![rig.sid("rust-analyzer")]);
    assert_eq!(
        rig.ed.state.status_msg.as_deref(),
        Some("lsp: restarted 1 server(s)")
    );
}

#[test]
fn restarting_a_buffers_stopped_servers_starts_them_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.ed.apply_lsp_server_op(PendingLspServerOp::Stop {
        target: LspServerTarget::Buffer(rig.bid),
    });
    assert!(rig.attached().is_empty());

    let bid = rig.bid;
    restart(&mut rig, LspServerTarget::Buffer(bid));

    assert_eq!(rig.attached(), vec![rig.sid("rust-analyzer")]);
}

#[test]
fn registering_a_stopped_name_again_starts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.stop("rust-analyzer");

    rig.eval(super::lsp_rig::RUST_ANALYZER);

    assert_eq!(rig.attached(), vec![rig.sid("rust-analyzer")]);
}

#[test]
fn unregistering_a_stopped_name_forgets_the_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.stop("rust-analyzer");

    rig.eval(r#"(unregister-lsp-server! "rust-analyzer")"#);

    let text = rig.ed.lsp_status_text();
    assert!(!text.contains("Stopped"), "{text:?}");
}

#[test]
fn a_config_reset_forgets_every_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    rig.stop("rust-analyzer");
    assert!(rig.ed.lsp_status_text().contains("Stopped"));

    rig.ed.reset_config_state();

    let text = rig.ed.lsp_status_text();
    assert!(!text.contains("Stopped"), "{text:?}");
}

/// A stop aimed at one buffer stops that buffer's instance of the server;
/// the same server's instance for another workspace root keeps running
/// through a later reconcile.
#[test]
fn a_buffer_stop_leaves_the_same_server_running_for_another_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rust_rig(&tmp);
    std::fs::create_dir_all(rig.root.join("other")).unwrap();
    std::fs::write(rig.root.join("other/Cargo.toml"), b"").unwrap();
    let other = open(&mut rig, "other/src/main.rs", "fn main() {}\n");
    let other_sid = servers_of(&rig, other)[0];

    rig.ed.apply_lsp_server_op(PendingLspServerOp::Stop {
        target: LspServerTarget::Buffer(rig.bid),
    });
    rig.eval(r#"(set-language-servers! "rust" '("rust-analyzer"))"#);

    assert!(rig.attached().is_empty());
    assert_eq!(servers_of(&rig, other), vec![other_sid]);
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

#[test]
fn a_user_list_attaches_a_registered_server_the_default_does_not_name() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = RigSpec {
        language: "jsx",
        extension: "jsx",
        file: "src/a.jsx",
        marked: "-[x]>\n",
        markers: &["package.json"],
        init: r#"(%define-language! "jsx" '("jsx") '() '() #f '("package.json"))
(register-lsp-server! "tsls" #:command "tsls")
(register-lsp-server! "eslint" #:command "eslint")
(set-default-language-servers! "jsx" '("tsls"))
(set-language-servers! "jsx" '("tsls" "eslint"))"#,
        dirs: hume_platform::dirs::Dirs::none(),
    };
    let rig = LspRig::drained(tmp.path(), spec, answering(2));

    assert_eq!(rig.attached(), vec![rig.sid("tsls"), rig.sid("eslint")]);
    assert_eq!(rig.warnings(), Vec::<String>::new());
}

#[test]
fn a_registered_server_no_list_names_does_not_attach() {
    let tmp = tempfile::tempdir().unwrap();
    let init = format!(
        "{}\n(register-lsp-server! \"ra-lint\" #:command \"ra-lint\")",
        super::lsp_rig::RUST_ANALYZER
    );
    let rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(&init),
        answering(2),
    );

    assert_eq!(rig.attached(), vec![rig.sid("rust-analyzer")]);
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

#[test]
fn a_servers_root_comes_from_its_languages_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = RigSpec {
        language: "rust",
        extension: "rs",
        file: "src/main.rs",
        marked: "-[f]>n main() {}\n",
        markers: &["Cargo.toml"],
        init: r#"(%define-language! "rust" '("rs") '() '() #f '("Cargo.toml"))
(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(set-language-servers! "rust" '("rust-analyzer"))"#,
        dirs: hume_platform::dirs::Dirs::none(),
    };
    let mut rig = LspRig::drained(tmp.path(), spec, answering(1));

    let sid = rig.sid("rust-analyzer");
    let root = rig.root.clone();
    assert_eq!(rig.ed.state.lsp.client_for_test(sid).unwrap().root(), root);
}
