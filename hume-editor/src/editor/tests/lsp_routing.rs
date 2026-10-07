// Requests routed among a buffer's servers when they are sent: feature,
// capability and `#:to` routing, deliveries to every routed server, and
// opaque positions encoded for each server.

use super::lsp_rig::{LspRig, RigSpec, TWO_RUST_SERVERS};
use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;
use hume_scripting::LspFeature;
use test_fixtures::unicode::ASTRAL;

const RA: ServerId = ServerId(0);
const LINT: ServerId = ServerId(1);

/// An `initialize` result advertising `capabilities` (merged over
/// incremental sync in `encoding`).
fn initialize(encoding: &str, capabilities: serde_json::Value) -> serde_json::Value {
    let mut caps = serde_json::json!({ "textDocumentSync": 2, "positionEncoding": encoding });
    caps.as_object_mut()
        .unwrap()
        .extend(capabilities.as_object().unwrap().clone());
    serde_json::json!({ "capabilities": caps })
}

/// `rust-analyzer` (`RA`) and `ra-lint` (`LINT`) answering `initialize`
/// with `ra` and `lint`; `script` adds each test's own answers.
fn backend(
    ra: serde_json::Value,
    lint: serde_json::Value,
    script: impl FnOnce(&mut RecordingLspBackend),
) -> RecordingLspBackend {
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to_server(RA, "initialize", ra);
    backend.respond_to_server(LINT, "initialize", lint);
    script(&mut backend);
    backend
}

fn spec(marked: &str) -> RigSpec<'_> {
    RigSpec::rust(marked).with_init(TWO_RUST_SERVERS)
}

/// Both servers running on `src/main.rs`.
fn two_servers(tmp: &tempfile::TempDir, backend: RecordingLspBackend) -> LspRig {
    LspRig::drained(tmp.path(), spec("-[f]>n main() {}\n"), backend)
}

fn hover_caps() -> serde_json::Value {
    serde_json::json!({ "hoverProvider": true })
}

/// A callback logging `(err result)` as one Warning: `err:<message>` or
/// `ok:<answer>`, where a string answer shows as itself, `null` as `null`
/// and anything else as `json`.
const LOG_ONE: &str = r#"(lambda (err res)
  (log! 'warn (if err
                  (string-append "err:" (hash-ref err 'message))
                  (string-append "ok:" (cond ((string? res) res) ((void? res) "null") (else "json"))))))"#;

/// A callback logging `(err results)` as one Warning: each server's name
/// and its error or answer, shown as [`LOG_ONE`] shows it, in order.
const LOG_ALL: &str = r#"(lambda (err results)
  (log! 'warn
    (if err
        (string-append "err:" (hash-ref err 'message))
        (apply string-append
          (map (lambda (r)
                 (string-append (lsp-server-name (hash-ref r 'server)) "="
                   (let ((e (hash-ref r 'err)))
                     (if e
                         (string-append "err:" (hash-ref e 'message))
                         (let ((v (hash-ref r 'result)))
                           (cond ((string? v) v) ((void? v) "null") (else "json")))))
                   ";"))
               results)))))"#;

#[test]
fn lsp_servers_filters_by_capability_and_filter() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::drained(
        tmp.path(),
        spec("-[f]>n main() {}\n").with_init(concat!(
            r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(register-lsp-server! "ra-lint" #:command "ra-lint")
(register-lsp-server! "ra-extra" #:command "ra-extra")"#,
            "\n",
            r#"(set-language-servers! "rust" (list "rust-analyzer" (hash 'name "ra-lint" 'except-features '(hover)) "ra-extra"))"#
        )),
        {
            let (mut backend, _, _) = RecordingLspBackend::new();
            backend.respond_to_server(RA, "initialize", initialize("utf-16", hover_caps()));
            backend.respond_to_server(LINT, "initialize", initialize("utf-16", hover_caps()));
            backend.respond_to_server(ServerId(2), "initialize", initialize("utf-16", serde_json::json!({})));
            backend
        },
    );

    rig.probe(
        r#"(log! 'warn (to-string (map lsp-server-name (lsp-servers pane #:feature 'hover))))"#,
    );
    rig.probe(r#"(log! 'warn (to-string (map lsp-server-name (lsp-servers pane))))"#);

    assert_eq!(
        rig.warnings(),
        vec![
            r#"("rust-analyzer")"#.to_string(),
            r#"("rust-analyzer" "ra-lint" "ra-extra")"#.to_string(),
        ]
    );
}

#[test]
fn lsp_handles_agrees_with_route_for_every_server() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::drained(
        tmp.path(),
        spec("-[f]>n main() {}\n").with_init(concat!(
            r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(register-lsp-server! "ra-lint" #:command "ra-lint")
(register-lsp-server! "ra-extra" #:command "ra-extra")"#,
            "\n",
            r#"(set-language-servers! "rust" (list "rust-analyzer" (hash 'name "ra-lint" 'except-features '(hover)) "ra-extra"))"#
        )),
        {
            let (mut backend, _, _) = RecordingLspBackend::new();
            backend.respond_to_server(RA, "initialize", initialize("utf-16", hover_caps()));
            backend.respond_to_server(LINT, "initialize", initialize("utf-16", hover_caps()));
            backend.respond_to_server(ServerId(2), "initialize", initialize("utf-16", serde_json::json!({})));
            backend
        },
    );
    let sids = [RA, LINT, ServerId(2)];
    let agree = |rig: &LspRig| -> Vec<(bool, bool)> {
        sids.iter()
            .map(|&sid| {
                (
                    rig.ed.state.lsp_handles(rig.bid, sid, LspFeature::Hover),
                    rig.ed.state.lsp_routes_to(rig.bid, sid, LspFeature::Hover),
                )
            })
            .collect()
    };

    assert_eq!(agree(&rig), [(true, true), (false, false), (false, false)]);

    rig.crash(RA);
    assert_eq!(
        agree(&rig),
        [(false, false), (false, false), (false, false)]
    );
}

#[test]
fn ranges_formatting_needs_ranges_support() {
    let tmp = safe_tempdir();
    let range_formatting = serde_json::json!({"documentRangeFormattingProvider": true});
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", range_formatting.clone()),
            initialize("utf-16", range_formatting),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/rangesFormatting" (hash "ranges" '()) {LOG_ONE})"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:format is not supported by rust-analyzer, ra-lint".to_string()]
    );
}

#[test]
fn lsp_servers_narrows_a_feature_by_method() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize(
                "utf-16",
                serde_json::json!({ "documentFormattingProvider": true }),
            ),
            initialize(
                "utf-16",
                serde_json::json!({
                    "documentFormattingProvider": true,
                    "documentRangeFormattingProvider": true,
                }),
            ),
            |_| {},
        ),
    );

    rig.probe(
        r#"(log! 'warn (to-string (map lsp-server-name
             (lsp-servers pane #:method "textDocument/rangeFormatting"))))"#,
    );

    assert_eq!(rig.warnings(), vec![r#"("ra-lint")"#.to_string()]);
}

#[test]
fn lsp_servers_accepts_a_method_alone() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", hover_caps()),
            |_| {},
        ),
    );

    rig.probe(
        r#"(log! 'warn (to-string (map lsp-server-name (lsp-servers pane #:method "textDocument/hover"))))"#,
    );

    assert_eq!(rig.warnings(), vec![r#"("ra-lint")"#.to_string()]);
}

#[test]
fn lsp_servers_excludes_starting_when_feature_given() {
    let tmp = safe_tempdir();
    let (backend, _, _) = RecordingLspBackend::new();
    let mut rig = LspRig::open(tmp.path(), spec("-[f]>n main() {}\n"), backend);

    rig.probe(
        r#"(log! 'warn (to-string (map lsp-server-name (lsp-servers pane))))
           (log! 'warn (to-string (lsp-servers pane #:feature 'hover)))"#,
    );

    assert_eq!(
        rig.warnings(),
        vec![
            r#"("rust-analyzer" "ra-lint")"#.to_string(),
            "()".to_string()
        ]
    );
}

#[test]
fn lsp_servers_rejects_an_unknown_feature() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |_| {},
        ),
    );

    rig.probe(r#"(log! 'warn (to-string (lsp-servers pane #:feature 'hovering)))"#);

    assert!(rig.warnings().is_empty());
    let log = rig.ed.state.message_log.format_for_display();
    assert!(log.contains("lsp-servers #:feature"), "{log}");
}

#[test]
fn request_picks_the_first_admitting_running_server() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", hover_caps()),
            |b| b.respond_to_server(LINT, "textDocument/hover", serde_json::json!("lint")),
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE})"#
    ));

    assert!(rig.requests_to(RA, "textDocument/hover").is_empty());
    assert_eq!(rig.requests_to(LINT, "textDocument/hover").len(), 1);
    assert_eq!(rig.warnings(), vec!["ok:lint".to_string()]);
}

#[test]
fn request_without_a_route_goes_to_the_first_running_server() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to_server(RA, "custom/ping", serde_json::json!("pong")),
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "custom/ping" (hash) {LOG_ONE})"#
    ));

    assert_eq!(rig.requests_to(RA, "custom/ping").len(), 1);
    assert!(rig.requests_to(LINT, "custom/ping").is_empty());
    assert_eq!(rig.warnings(), vec!["ok:pong".to_string()]);
}

#[test]
fn request_to_a_starting_only_buffer_fails_with_starting() {
    let tmp = safe_tempdir();
    let (backend, _, _) = RecordingLspBackend::new();
    let mut rig = LspRig::open(tmp.path(), spec("-[f]>n main() {}\n"), backend);

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE})"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:rust-analyzer, ra-lint still starting".to_string()]
    );
    assert!(
        rig.requests
            .borrow()
            .iter()
            .all(|(_, m, _)| m == "initialize")
    );
}

#[test]
fn request_unsupported_by_every_server_names_them() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE})"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:hover is not supported by rust-analyzer, ra-lint".to_string()]
    );
}

#[test]
fn request_unavailable_empty_answers_void_without_an_error() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE} #:unavailable 'empty)"#
    ));

    assert_eq!(rig.warnings(), vec!["ok:null".to_string()]);
}

#[test]
fn request_all_unavailable_empty_answers_an_empty_list_without_an_error() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request-all! pane "textDocument/hover" (lsp-position-params pane) {LOG_ALL} #:unavailable 'empty)"#
    ));

    assert_eq!(rig.warnings(), vec![String::new()]);
}

#[test]
fn resolve_to_a_server_without_a_resolve_provider_is_unsupported() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize(
                "utf-16",
                serde_json::json!({ "codeActionProvider": { "resolveProvider": true } }),
            ),
            initialize("utf-16", serde_json::json!({ "codeActionProvider": true })),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "codeAction/resolve" (hash "title" "t") {LOG_ONE}
             #:to (cadr (lsp-servers pane)))"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:code-action is not supported by ra-lint".to_string()]
    );
}

#[test]
fn request_with_to_names_the_server() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |b| b.respond_to_server(LINT, "textDocument/hover", serde_json::json!(null)),
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE}
             #:to (cadr (lsp-servers pane)))"#
    ));

    assert!(rig.requests_to(RA, "textDocument/hover").is_empty());
    assert_eq!(rig.requests_to(LINT, "textDocument/hover").len(), 1);
}

#[test]
fn request_with_to_an_unattached_server_errors() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |_| {},
        ),
    );
    rig.eval("(define *lint* #f)");
    rig.probe("(set! *lint* (cadr (lsp-servers pane)))");
    rig.stop("ra-lint");

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE} #:to *lint*)"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:ra-lint is not attached to this buffer".to_string()]
    );
}

#[test]
fn all_delivers_once_in_input_order() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| {
                b.respond_to_server(LINT, "custom/ping", serde_json::json!("b"));
                b.respond_to_server(RA, "custom/ping", serde_json::json!("a"));
            },
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request-all! pane "custom/ping" (hash) {LOG_ALL})"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["rust-analyzer=a;ra-lint=b;".to_string()]
    );
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 0);
}

#[test]
fn all_member_timeout_fills_slot() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to_server(RA, "custom/ping", serde_json::json!("a")),
        ),
    );
    rig.ed.state.settings.lsp_request_timeout_ms = 0;

    rig.probe(&format!(
        r#"(lsp-request-all! pane "custom/ping" (hash) {LOG_ALL})"#
    ));
    rig.ed.settle();

    assert_eq!(
        rig.warnings(),
        vec!["rust-analyzer=a;ra-lint=err:timed out;".to_string()]
    );
}

#[test]
fn request_to_a_stopped_server_fills_its_slot_and_the_rest_still_answer() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to_server(RA, "custom/ping", serde_json::json!("a")),
        ),
    );

    rig.eval(&format!(
        r#"(define-typed-command! "send" "" (lambda (pane) (lsp-request-all! pane "custom/ping" (hash) {LOG_ALL})))"#
    ));
    type_cmd(&mut rig.ed, ":send");
    rig.stop("ra-lint");
    rig.ed.settle();

    assert_eq!(
        rig.warnings(),
        vec!["rust-analyzer=a;ra-lint=err:server stopped before answering;".to_string()]
    );
}

#[test]
fn all_stale_anchor_drops_whole_delivery_and_releases_tracked() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| {
                b.respond_to_server(RA, "custom/ping", serde_json::json!("a"));
                b.respond_to_server(LINT, "custom/ping", serde_json::json!("b"));
            },
        ),
    );

    rig.eval("(define *tok* #f)");
    rig.eval(&format!(
        r#"(define-typed-command! "send" "" (lambda (pane)
             (set! *tok* (track-position! pane))
             (lsp-request-all! pane "custom/ping" (hash) {LOG_ALL} #:tracked *tok*)))"#
    ));
    type_cmd(&mut rig.ed, ":send");
    rig.ed.feed_key(key('i'));
    type_chars(&mut rig.ed, "x");
    rig.ed.feed_key(key_esc());
    rig.ed.settle();

    assert!(rig.warnings().is_empty());
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 0);
    assert_eq!(rig.ed.state.lsp.callback_count_for_test(), 0);
    rig.probe(r#"(log! 'warn (if (tracked-position-params *tok*) "held" "released"))"#);
    assert_eq!(rig.warnings(), vec!["released".to_string()]);
}

#[test]
fn all_supersede_cancels_all_members() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| {
                b.respond_to("custom/ping", serde_json::json!("first"));
                b.respond_to("custom/ping", serde_json::json!("first"));
                b.respond_to("custom/ping", serde_json::json!("second"));
                b.respond_to("custom/ping", serde_json::json!("second"));
            },
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request-all! pane "custom/ping" (hash) {LOG_ALL} #:supersede "ping")
           (lsp-request-all! pane "custom/ping" (hash) {LOG_ALL} #:supersede "ping")"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["rust-analyzer=second;ra-lint=second;".to_string()]
    );
    for sid in [RA, LINT] {
        assert_eq!(rig.sent(sid, "$/cancelRequest").len(), 1, "{sid:?}");
    }
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 0);
}

#[test]
fn delivery_count_leak_check_after_stop() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.eval(&format!(
        r#"(define-typed-command! "send" "" (lambda (pane)
             (lsp-request-all! pane "custom/ping" (hash) {LOG_ALL} #:supersede "ping")))"#
    ));
    type_cmd(&mut rig.ed, ":send");
    rig.stop("rust-analyzer");
    rig.stop("ra-lint");
    rig.ed.settle();

    assert_eq!(
        rig.warnings(),
        vec![
            "rust-analyzer=err:server stopped before answering;\
             ra-lint=err:server stopped before answering;"
                .to_string()
        ]
    );
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 0);
    assert_eq!(rig.ed.state.lsp.callback_count_for_test(), 0);
}

#[test]
fn explicit_members_send_each_server_its_own_params() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to("custom/ping", serde_json::json!(null)),
        ),
    );

    rig.probe(&format!(
        r#"(let ((servers (lsp-servers pane)))
             (lsp-request-all! pane "custom/ping"
               (list (cons (cadr servers) (hash "who" "lint"))
                     (cons (car servers) (hash "who" "ra")))
               {LOG_ALL}))"#
    ));

    assert_eq!(
        rig.requests_to(RA, "custom/ping"),
        vec![serde_json::json!({ "who": "ra" })]
    );
    assert_eq!(
        rig.requests_to(LINT, "custom/ping"),
        vec![serde_json::json!({ "who": "lint" })]
    );
}

#[test]
fn notify_defaults_to_every_running_attached_server() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(r#"(lsp-notify! pane "custom/note" (hash "n" 1))"#);
    rig.probe(r#"(lsp-notify! pane "custom/only" (hash) #:to (cadr (lsp-servers pane)))"#);

    for sid in [RA, LINT] {
        assert_eq!(
            rig.sent(sid, "custom/note"),
            vec![serde_json::json!({ "n": 1 })],
            "{sid:?}"
        );
    }
    assert!(rig.sent(RA, "custom/only").is_empty());
    assert_eq!(rig.sent(LINT, "custom/only").len(), 1);
}

#[test]
fn doc_pos_params_serialize_in_each_servers_encoding() {
    let tmp = safe_tempdir();
    let marked = format!("a{ASTRAL}-[b]>c\n");
    let mut rig = LspRig::drained(
        tmp.path(),
        spec(&marked),
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-8", serde_json::json!({})),
            |b| b.respond_to("custom/at", serde_json::json!(null)),
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request-all! pane "custom/at" (lsp-position-params pane) {LOG_ALL})"#
    ));

    let utf16 = 1 + ASTRAL.encode_utf16().count();
    let utf8 = 1 + ASTRAL.len();
    assert_eq!(
        rig.requests_to(RA, "custom/at")[0]["position"],
        serde_json::json!({ "line": 0, "character": utf16 })
    );
    assert_eq!(
        rig.requests_to(LINT, "custom/at")[0]["position"],
        serde_json::json!({ "line": 0, "character": utf8 })
    );
}

#[test]
fn doc_pos_for_another_buffer_fills_the_slot_with_an_error() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to("custom/at", serde_json::json!(null)),
        ),
    );
    rig.eval("(define *tok* #f)");
    rig.probe("(set! *tok* (track-position! pane))");
    let other = rig.root.join("src/other.rs");
    std::fs::write(&other, "fn other() {}\n").unwrap();
    let other = rig.ed.open_extra_file(&other).expect("the file opens");
    let fp = crate::editor::commands::FocusedPane::current(&rig.ed.state);
    rig.ed.switch_to_buffer_without_jump(fp, other);
    rig.ed.drain_lsp();

    rig.probe(&format!(
        r#"(lsp-request! pane "custom/at" (tracked-position-params *tok*) {LOG_ONE})"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:lsp-request! params: position belongs to another buffer".to_string()]
    );
    assert!(rig.requests_to(RA, "custom/at").is_empty());
}

#[test]
fn doc_pos_from_before_a_same_eval_edit_errors() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::drained(
        tmp.path(),
        spec("fn ma-[i]>n() {}\n"),
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(let ((params (lsp-position-params pane)))
             (call! "delete" pane)
             (lsp-request! pane "custom/at" params {LOG_ONE}))"#
    ));

    assert_eq!(
        rig.warnings(),
        vec![
            "err:lsp-request! params: position is from an earlier version of the buffer"
                .to_string()
        ]
    );
    assert!(rig.requests_to(RA, "custom/at").is_empty());
}

#[test]
fn doc_pos_past_end_after_a_same_eval_delete_errors() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::drained(
        tmp.path(),
        spec("fn ma-[i]>n() {}\n"),
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(let ((params (lsp-position-params pane)))
             (call! "select-all" pane)
             (call! "delete" pane)
             (lsp-request! pane "custom/at" params {LOG_ONE}))"#
    ));

    assert_eq!(
        rig.warnings(),
        vec![
            "err:lsp-request! params: position is from an earlier version of the buffer"
                .to_string()
        ]
    );
}

#[test]
fn tracked_position_params_needs_no_server() {
    let tmp = safe_tempdir();
    let (backend, _, _) = RecordingLspBackend::new();
    let mut rig = LspRig::open(
        tmp.path(),
        RigSpec::rust("fn -[m]>ain() {}\n").with_init(""),
        backend,
    );
    assert!(rig.attached().is_empty());

    rig.probe(
        r#"(log! 'warn (if (hash-contains? (tracked-position-params (track-position! pane)) "position") "yes" "no"))"#,
    );

    assert_eq!(rig.warnings(), vec!["yes".to_string()]);
}

#[test]
fn notification_hook_receives_server_handle() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );
    rig.eval(
        r#"(register-lsp-notification-hook! "custom/hello"
             (lambda (server method params) (log! 'warn (lsp-server-name server))))"#,
    );

    rig.push(
        LINT,
        hume_lsp::codec::Message::Notification {
            method: "custom/hello".to_string(),
            params: serde_json::json!({}),
        },
    );
    rig.ed.settle();

    assert_eq!(rig.warnings(), vec!["ra-lint".to_string()]);
}

/// A callback logging `err`'s `'kind` (and `'code` when there is one), or
/// `none`.
const LOG_KIND: &str = r#"(lambda (err res)
  (log! 'warn (if err
                  (string-append (symbol->string (hash-ref err 'kind))
                                 (if (hash-contains? err 'code)
                                     (string-append ":" (number->string (hash-ref err 'code)))
                                     ""))
                  "none")))"#;

#[test]
fn routing_failure_err_is_unavailable() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_KIND})"#
    ));

    assert_eq!(rig.warnings(), vec!["unavailable".to_string()]);
}

#[test]
fn server_error_err_carries_kind_and_code() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.fail_with_server(RA, "custom/ping", -32601, "no such method"),
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "custom/ping" (hash) {LOG_KIND})"#
    ));

    assert_eq!(rig.warnings(), vec!["server:-32601".to_string()]);
}

#[test]
fn stopped_err_is_tagged() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );
    rig.eval(&format!(
        r#"(define-typed-command! "send" "" (lambda (pane) (lsp-request! pane "custom/ping" (hash) {LOG_KIND})))"#
    ));
    type_cmd(&mut rig.ed, ":send");
    rig.stop("rust-analyzer");
    rig.ed.settle();

    assert_eq!(rig.warnings(), vec!["stopped".to_string()]);
}

#[test]
fn timeout_err_is_tagged() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );
    rig.ed.state.settings.lsp_request_timeout_ms = 0;

    rig.probe(&format!(
        r#"(lsp-request! pane "custom/ping" (hash) {LOG_KIND})"#
    ));
    rig.ed.settle();

    assert_eq!(rig.warnings(), vec!["timeout".to_string()]);
}

#[test]
fn unsent_err_is_tagged() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );
    rig.eval("(define *tok* #f)");
    rig.probe("(set! *tok* (track-position! pane))");
    let other = rig.root.join("src/other.rs");
    std::fs::write(&other, "fn other() {}\n").unwrap();
    let other = rig.ed.open_extra_file(&other).expect("the file opens");
    let fp = crate::editor::commands::FocusedPane::current(&rig.ed.state);
    rig.ed.switch_to_buffer_without_jump(fp, other);
    rig.ed.drain_lsp();

    rig.probe(&format!(
        r#"(lsp-request! pane "custom/at" (tracked-position-params *tok*) {LOG_KIND})"#
    ));

    assert_eq!(rig.warnings(), vec!["unsent".to_string()]);
}

#[test]
fn per_server_member_not_attached_is_unavailable() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to_server(RA, "custom/ping", serde_json::json!("a")),
        ),
    );
    rig.eval("(define *lint* #f)");
    rig.probe("(set! *lint* (cadr (lsp-servers pane)))");
    rig.stop("ra-lint");

    rig.probe(
        r#"(lsp-request-all! pane "custom/ping"
             (list (cons (car (lsp-servers pane)) (hash)) (cons *lint* (hash)))
             (lambda (err results)
               (log! 'warn (to-string (map (lambda (r)
                                             (let ((e (hash-ref r 'err)))
                                               (if e (symbol->string (hash-ref e 'kind)) "ok")))
                                           results)))))"#,
    );

    assert_eq!(rig.warnings(), vec![r#"("ok" "unavailable")"#.to_string()]);
}

#[test]
fn per_server_request_keeps_its_feature_filter() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to_server(RA, "textDocument/hover", serde_json::json!("a")),
        ),
    );
    rig.ed.state.settings.lsp_request_timeout_ms = 0;

    rig.probe(
        r#"(lsp-request-all! pane "textDocument/hover"
             (map (lambda (s) (cons s (hash))) (lsp-servers pane))
             (lambda (err results)
               (log! 'warn (to-string (map (lambda (r)
                                             (let ((e (hash-ref r 'err)))
                                               (if e (symbol->string (hash-ref e 'kind)) "ok")))
                                           results)))))"#,
    );
    rig.ed.settle();

    assert_eq!(rig.warnings(), vec![r#"("ok" "unavailable")"#.to_string()]);
}

#[test]
fn a_standard_method_is_routed_by_its_own_feature() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", serde_json::json!({})),
            |b| b.respond_to_server(RA, "textDocument/hover", serde_json::json!("a")),
        ),
    );
    rig.ed.state.settings.lsp_request_timeout_ms = 0;

    rig.probe(&format!(
        r#"(lsp-request-all! pane "textDocument/hover" (hash) {LOG_ALL})"#
    ));
    rig.ed.settle();

    assert_eq!(
        rig.warnings(),
        vec!["rust-analyzer=a;".to_string()],
        "ra-lint advertises no hover, so the request is not sent to it"
    );
}

#[test]
fn a_superseding_request_that_reaches_no_server_still_cancels_the_one_in_flight() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "custom/ping" (hash) {LOG_ONE} #:supersede "k")
           (lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE} #:supersede "k")"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:hover is not supported by rust-analyzer, ra-lint".to_string()]
    );
    assert_eq!(rig.sent(RA, "$/cancelRequest").len(), 1);
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 0);
}

#[test]
fn request_with_to_a_server_its_list_entry_excludes_names_the_exclusion() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::drained(
        tmp.path(),
        spec("-[f]>n main() {}\n").with_init(concat!(
            r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(register-lsp-server! "ra-lint" #:command "ra-lint")"#,
            "\n",
            r#"(set-language-servers! "rust" (list "rust-analyzer" (hash 'name "ra-lint" 'except-features '(hover))))"#
        )),
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |_| {},
        ),
    );

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE}
             #:to (cadr (lsp-servers pane)))"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:hover is excluded for ra-lint by the language's server list".to_string()]
    );
}

#[test]
fn request_with_to_a_server_value_from_before_a_restart_says_it_is_stale() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |b| {
                b.respond_to_server(
                    ServerId(2),
                    "initialize",
                    initialize("utf-16", hover_caps()),
                )
            },
        ),
    );
    rig.eval("(define *ra* #f)");
    rig.probe("(set! *ra* (car (lsp-servers pane)))");
    rig.ed
        .apply_lsp_server_op(hume_scripting::PendingLspServerOp::Restart {
            target: hume_scripting::LspServerTarget::Name(
                hume_scripting::ServerName::parse("rust-analyzer").unwrap(),
            ),
        });
    rig.ed.drain_lsp();

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE} #:to *ra*)"#
    ));

    assert_eq!(
        rig.warnings(),
        vec!["err:rust-analyzer was restarted; the server value taken before is stale".to_string()]
    );
}

#[test]
fn a_feature_on_a_standard_method_is_rejected_at_the_call() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |_| {},
        ),
    );

    rig.probe(
        r#"(for-each
             (lambda (thunk) (log! 'warn (with-handler (lambda (e) (to-string e)) (thunk))))
             (list
               (lambda () (lsp-request! pane "textDocument/hover" (hash) (lambda (err res) (begin)) #:feature 'completion))
               (lambda () (lsp-request-all! pane "textDocument/hover" (hash) (lambda (err res) (begin)) #:feature 'completion))
               (lambda () (lsp-notify! pane "textDocument/hover" (hash) #:feature 'completion))))"#,
    );

    let warnings = rig.warnings();
    assert_eq!(warnings.len(), 3);
    for (warning, verb) in warnings
        .iter()
        .zip(["lsp-request!", "lsp-request-all!", "lsp-notify!"])
    {
        assert!(
            warning.contains(&format!(
                "{verb}: textDocument/hover is a standard method of the 'hover feature; #:feature is for other methods"
            )),
            "{warning:?}"
        );
    }
}

#[test]
fn a_feature_on_a_custom_method_still_routes_by_it() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(r#"(lsp-notify! pane "custom/note" (hash) #:feature 'hover)"#);

    assert_eq!(rig.sent(RA, "custom/note").len(), 1);
    assert!(rig.sent(LINT, "custom/note").is_empty());
}

#[test]
fn a_notification_of_a_standard_method_is_routed_by_its_feature() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", serde_json::json!({})),
            |_| {},
        ),
    );

    rig.probe(r#"(lsp-notify! pane "textDocument/hover" (hash))"#);

    assert_eq!(rig.sent(RA, "textDocument/hover").len(), 1);
    assert!(rig.sent(LINT, "textDocument/hover").is_empty());
}

#[test]
fn a_crashed_server_is_skipped_for_the_next_one() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(
        &tmp,
        backend(
            initialize("utf-16", hover_caps()),
            initialize("utf-16", hover_caps()),
            |b| b.respond_to_server(LINT, "textDocument/hover", serde_json::json!("b")),
        ),
    );
    rig.crash(RA);

    rig.probe(&format!(
        r#"(lsp-request! pane "textDocument/hover" (lsp-position-params pane) {LOG_ONE})"#
    ));

    assert_eq!(rig.warnings(), vec!["ok:b".to_string()]);
    assert!(rig.requests_to(RA, "textDocument/hover").is_empty());
}

/// One delivery whose slots end as an answer, a timeout and a stop calls
/// back once, with each slot's own outcome in attachment order.
#[test]
fn one_delivery_can_mix_an_answer_a_timeout_and_a_stop() {
    let tmp = safe_tempdir();
    let init = format!(
        "{TWO_RUST_SERVERS}\n{}",
        r#"(register-lsp-server! "ra-extra" #:command "ra-extra")
(set-language-servers! "rust" '("rust-analyzer" "ra-lint" "ra-extra"))"#
    );
    let mut rig = LspRig::drained(
        tmp.path(),
        spec("-[f]>n main() {}\n").with_init(&init),
        backend(
            initialize("utf-16", serde_json::json!({})),
            initialize("utf-16", serde_json::json!({})),
            |b| {
                b.respond_to_server(
                    ServerId(2),
                    "initialize",
                    initialize("utf-16", serde_json::json!({})),
                );
                b.respond_to_server(RA, "custom/ping", serde_json::json!("a"));
            },
        ),
    );
    rig.eval(&format!(
        r#"(define-typed-command! "send" "" (lambda (pane) (lsp-request-all! pane "custom/ping" (hash) {LOG_ALL})))"#
    ));

    type_cmd(&mut rig.ed, ":send");
    rig.stop("ra-extra");
    rig.ed
        .state
        .lsp
        .client_for_test(LINT)
        .expect("ra-lint is running")
        .expire_pending_deadlines_for_test();
    rig.ed.settle();

    assert_eq!(
        rig.warnings(),
        vec![
            "rust-analyzer=a;ra-lint=err:timed out;ra-extra=err:server stopped before answering;"
                .to_string()
        ]
    );
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 0);
}
