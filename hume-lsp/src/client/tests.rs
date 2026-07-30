use super::*;
use crate::inline::InlineLspBackend;
use std::path::PathBuf;

fn canned_result(encoding: Option<PositionEncodingKind>) -> serde_json::Value {
    let caps = ServerCapabilities {
        position_encoding: encoding,
        ..Default::default()
    };
    serde_json::to_value(InitializeResult {
        capabilities: caps,
        ..Default::default()
    })
    .unwrap()
}

// Golden-field check on the load-bearing capability list:
// capabilities are load-bearing config — assert the exact advertised
// set rather than just "it builds".
#[test]
#[allow(deprecated)] // asserting on the deliberately-still-populated compat field
fn initialize_params_advertise_the_v1_capability_set() {
    #[cfg(windows)]
    let root = PathBuf::from(r"C:\tmp\proj");
    #[cfg(not(windows))]
    let root = PathBuf::from("/tmp/proj");
    let params = build_initialize_params(&root, None);

    assert_eq!(params.process_id, Some(std::process::id()));
    assert!(params.root_uri.is_some());
    let folders = params.workspace_folders.expect("workspace_folders set");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "proj");

    let caps = params.capabilities;
    assert_eq!(
        caps.general.unwrap().position_encodings,
        Some(vec![
            PositionEncodingKind::UTF8,
            PositionEncodingKind::UTF16
        ])
    );
    let td = caps.text_document.unwrap();
    assert_eq!(
        td.completion
            .unwrap()
            .completion_item
            .unwrap()
            .snippet_support,
        Some(false)
    );
    assert_eq!(
        td.hover.unwrap().content_format,
        Some(vec![MarkupKind::PlainText, MarkupKind::Markdown])
    );
    assert!(td.publish_diagnostics.is_some());
    assert!(td.rename.is_some());
    assert!(td.inlay_hint.is_some());
    let ws = caps.workspace.unwrap();
    assert_eq!(ws.apply_edit, Some(true));
    assert_eq!(ws.configuration, Some(true));
    // Declared unconditionally — the push happens right after
    // `initialized` with no dynamic-registration negotiation.
    assert_eq!(
        ws.did_change_configuration
            .expect("did_change_configuration capability must be declared")
            .dynamic_registration,
        Some(false)
    );
    // Manual smoke testing found rust-analyzer refuses
    // textDocument/rename outright without this declared — every
    // rename result is a WorkspaceEdit, and some servers won't attempt
    // one unless the client has confirmed it can apply it.
    let we = ws
        .workspace_edit
        .expect("workspace_edit capability must be declared");
    assert_eq!(we.document_changes, Some(true));
    // Must be present or rust-analyzer refuses every rename outright
    // (confirmed live) — HUME still can't actually apply a resource
    // op if one arrives (edits::collect_edit_entries rejects it by
    // design), but the alternative breaks the common case.
    assert_eq!(
        we.resource_operations,
        Some(vec![
            ResourceOperationKind::Create,
            ResourceOperationKind::Rename,
            ResourceOperationKind::Delete,
        ])
    );
    assert_eq!(we.failure_handling, Some(FailureHandlingKind::Abort));
    // Manual smoke testing found rust-analyzer withholds
    // diagnostic-derived quickfixes entirely without this declared —
    // a byte-perfect codeAction request still came back empty.
    let ca = td
        .code_action
        .expect("code_action capability must be declared");
    let literal = ca
        .code_action_literal_support
        .expect("code_action_literal_support must be declared");
    assert!(
        literal
            .code_action_kind
            .value_set
            .contains(&CodeActionKind::QUICKFIX.as_str().to_string())
    );
    assert_eq!(ca.is_preferred_support, Some(true));
    assert_eq!(ca.disabled_support, Some(true));
}

#[test]
fn handshake_round_trip_transitions_to_running() {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend
        .start("rust-analyzer", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    assert_eq!(client.state, ServerState::Starting);

    let events = backend.drain();
    assert_eq!(events.len(), 1);
    let (_id, ev) = events.into_iter().next().unwrap();
    let actions = client.on_event(ev);

    assert_eq!(client.state, ServerState::Running);
    assert!(client.caps.is_some());
    match &actions[..] {
        [ClientAction::BecameRunning { send }] => {
            assert_eq!(send.len(), 1);
            match &send[0] {
                Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                other => panic!("expected initialized notification, got {other:?}"),
            }
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
}

#[test]
fn handshake_sends_did_change_configuration_when_settings_set() {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend
        .start("rust-analyzer", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.set_settings(Some(serde_json::json!({"files": {"watcher": "server"}})));

    client.start_handshake(&mut backend);
    let events = backend.drain();
    let (_id, ev) = events.into_iter().next().unwrap();
    let actions = client.on_event(ev);

    match &actions[..] {
        [ClientAction::BecameRunning { send }] => {
            assert_eq!(
                send.len(),
                2,
                "expected initialized + didChangeConfiguration"
            );
            match &send[0] {
                Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                other => panic!("expected initialized notification, got {other:?}"),
            }
            match &send[1] {
                Message::Notification { method, params } => {
                    assert_eq!(method, "workspace/didChangeConfiguration");
                    assert_eq!(
                        params["settings"],
                        serde_json::json!({"files": {"watcher": "server"}})
                    );
                }
                other => panic!("expected didChangeConfiguration notification, got {other:?}"),
            }
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
}

#[test]
fn handshake_omits_did_change_configuration_when_settings_unset() {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend
        .start("rust-analyzer", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    let events = backend.drain();
    let (_id, ev) = events.into_iter().next().unwrap();
    let actions = client.on_event(ev);

    match &actions[..] {
        [ClientAction::BecameRunning { send }] => {
            assert_eq!(send.len(), 1, "expected only initialized, no config push");
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
}

#[test]
fn did_change_configuration_is_queued_ahead_of_pending_did_open() {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend
        .start("rust-analyzer", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.set_settings(Some(serde_json::json!({"a": 1})));

    // Queued while Starting, before the handshake completes.
    client.send_or_queue(
        &mut backend,
        Message::Notification {
            method: "textDocument/didOpen".to_string(),
            params: serde_json::json!({}),
        },
    );

    client.start_handshake(&mut backend);
    let events = backend.drain();
    let (_id, ev) = events.into_iter().next().unwrap();
    let actions = client.on_event(ev);

    match &actions[..] {
        [ClientAction::BecameRunning { send }] => {
            let methods: Vec<&str> = send
                .iter()
                .map(|m| match m {
                    Message::Notification { method, .. } => method.as_str(),
                    _ => panic!("expected only notifications in send"),
                })
                .collect();
            assert_eq!(
                methods,
                vec![
                    "initialized",
                    "workspace/didChangeConfiguration",
                    "textDocument/didOpen",
                ]
            );
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
}

#[test]
fn initialize_request_carries_initialization_options_when_set() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.set_init_options(Some(serde_json::json!({"check": {"command": "clippy"}})));
    client.start_handshake(&mut backend);

    match &backend.sent[0] {
        (_, Message::Request { params, .. }) => {
            assert_eq!(
                params["initializationOptions"]["check"]["command"],
                "clippy"
            );
        }
        other => panic!("expected the initialize request, got {other:?}"),
    }
}

#[test]
fn initialize_request_omits_initialization_options_when_unset() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);

    match &backend.sent[0] {
        (_, Message::Request { params, .. }) => {
            assert!(
                params.get("initializationOptions").is_none(),
                "expected the key to be absent, not null: {params:?}"
            );
        }
        other => panic!("expected the initialize request, got {other:?}"),
    }
}

#[test]
fn handshake_failure_response_crashes() {
    let mut backend = InlineLspBackend::new();
    backend.fail_with("initialize", -32603, "boom");
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    let (_id, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);

    match &actions[..] {
        [ClientAction::Crashed { error }] => {
            assert!(error.as_ref().unwrap().contains("initialize failed"));
        }
        other => panic!("expected one Crashed action, got {other:?}"),
    }
    assert_eq!(client.state, ServerState::Crashed);
    assert_eq!(
        client.pending_count(),
        0,
        "the error path must also consume the initialize pending entry"
    );
}

/// Pins that the `initialize` response is discriminated by the id stashed
/// in `initialize_id`, not by matching on the method string — a Steel
/// plugin issuing `(lsp-request "initialize" ...)` through the generic
/// bridge must get an ordinary correlated response, never be mistaken
/// for the handshake and hijack the client into `BecameRunning`/`Crashed`.
#[test]
fn generic_initialize_request_is_not_hijacked_by_the_handshake_discriminator() {
    let (mut backend, mut client) = make_running_client();
    backend.respond_to("initialize", serde_json::json!({"ok": true}));

    let meta = RequestMeta {
        method: "initialize".to_string(),
        allow_stale: false,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let sent_id = client.send_request(&mut backend, "initialize", serde_json::Value::Null, meta);

    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    assert!(
        actions.is_empty(),
        "a Steel-issued initialize response must correlate normally, not surface as a lifecycle action"
    );
    assert_eq!(client.state, ServerState::Running);

    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert!(actions.is_empty());
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].0, sent_id);
}

// ── initialize timeout via take_completed's sweep ────────────────────────

#[test]
fn initialize_sweep_is_quiet_before_the_deadline() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(&mut backend);

    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert!(completed.is_empty());
    assert!(actions.is_empty());
    assert_eq!(client.state, ServerState::Starting);
}

#[test]
fn initialize_timeout_crashes_via_the_sweep() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(&mut backend);

    let past_deadline = Instant::now() + std::time::Duration::from_secs(31);
    let (completed, actions) = client.take_completed(&mut backend, past_deadline);
    assert!(
        completed.is_empty(),
        "the internal initialize entry must never appear as a completed request"
    );
    match &actions[..] {
        [ClientAction::Crashed { error }] => {
            assert!(error.as_ref().unwrap().contains("initialize timed out"));
        }
        other => panic!("expected one Crashed action, got {other:?}"),
    }
    assert_eq!(client.state, ServerState::Crashed);

    // A second sweep after already Crashed must not report again.
    let (completed2, actions2) = client.take_completed(&mut backend, past_deadline);
    assert!(completed2.is_empty());
    assert!(actions2.is_empty());
}

#[test]
fn initialize_never_times_out_once_running() {
    let (mut backend, mut client) = make_running_client();
    let far_future = Instant::now() + std::time::Duration::from_secs(1000);
    let (completed, actions) = client.take_completed(&mut backend, far_future);
    assert!(completed.is_empty());
    assert!(actions.is_empty());
}

// ── earliest_deadline ─────────────────────────────────────────────────────

#[test]
fn earliest_deadline_none_when_no_pending() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let client = LspClient::new(sid, PathBuf::from("."));
    assert_eq!(client.earliest_deadline(), None);
}

#[test]
fn earliest_deadline_is_min() {
    let (mut backend, mut client) = make_running_client();
    let now = Instant::now();

    client.send_request(
        &mut backend,
        "foo",
        serde_json::Value::Null,
        RequestMeta {
            method: "foo".to_string(),
            allow_stale: false,
            deadline: now + std::time::Duration::from_secs(5),
        },
    );
    client.send_request(
        &mut backend,
        "bar",
        serde_json::Value::Null,
        RequestMeta {
            method: "bar".to_string(),
            allow_stale: false,
            deadline: now + std::time::Duration::from_secs(1),
        },
    );

    let earliest = client.earliest_deadline().expect("two pending requests");
    assert!(
        earliest < now + std::time::Duration::from_secs(5),
        "must be the nearer (1s) deadline, not the farther (5s) one"
    );
}

#[test]
fn messages_sent_while_starting_are_queued_then_flushed_in_order() {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    client.send_or_queue(
        &mut backend,
        Message::Notification {
            method: "textDocument/didOpen".to_string(),
            params: serde_json::json!({"uri": "file:///a"}),
        },
    );
    // Not sent yet — must not appear in the backend's sent log as a
    // didOpen (only the initialize request should be there).
    assert!(
        backend
            .sent
            .iter()
            .all(|(_, m)| !matches!(m, Message::Notification { method, .. } if method == "textDocument/didOpen"))
    );

    let (_id, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    match &actions[..] {
        [ClientAction::BecameRunning { send }] => {
            assert_eq!(send.len(), 2);
            match &send[0] {
                Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                other => panic!("expected initialized first, got {other:?}"),
            }
            match &send[1] {
                Message::Notification { method, .. } => {
                    assert_eq!(method, "textDocument/didOpen")
                }
                other => panic!("expected the queued didOpen second, got {other:?}"),
            }
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
}

#[test]
fn send_request_while_starting_is_queued_then_flushed_and_still_correlates() {
    let mut backend = InlineLspBackend::with_default_handshake();
    backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let sent_id = client.send_request(
        &mut backend,
        "textDocument/hover",
        serde_json::Value::Null,
        meta,
    );

    // Nothing but the initialize request should be on the wire yet.
    assert!(
        backend.sent.iter().all(
            |(_, m)| !matches!(m, Message::Request { method, .. } if method == "textDocument/hover")
        ),
        "request must be queued, not sent, while Starting"
    );
    assert_eq!(
        client.pending_count(),
        2,
        "pending entry recorded even though queued (plus the in-flight initialize)"
    );

    // Handshake completes: BecameRunning flushes the queued hover request.
    let (_id, ev) = backend.drain().into_iter().next().unwrap();
    let mut actions = client.on_event(ev).into_iter();
    match actions.next() {
        Some(ClientAction::BecameRunning { send }) => {
            assert_eq!(send.len(), 2);
            match &send[1] {
                Message::Request { id, method, .. } => {
                    assert_eq!(*id, sent_id);
                    assert_eq!(method, "textDocument/hover");
                }
                other => panic!("expected the queued hover request second, got {other:?}"),
            }
            // The real editor glue's `BecameRunning` dispatch does this
            // exact send loop (`dispatch_lsp_action`) — replicate it so
            // the flushed hover request actually reaches the backend.
            for msg in send {
                backend.send(sid, msg);
            }
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
    assert!(actions.next().is_none());

    // The response still correlates normally once actually sent.
    let (_sid, ev) = backend
        .drain()
        .into_iter()
        .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
        .expect("hover response");
    client.on_event(ev);
    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].0, sent_id);
    assert!(actions.is_empty());
}

#[test]
fn utf8_negotiated_when_offered() {
    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        canned_result(Some(PositionEncodingKind::UTF8)),
    );
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    let (_id, ev) = backend.drain().into_iter().next().unwrap();
    client.on_event(ev);

    assert_eq!(client.encoding, PositionEncoding::Utf8);
}

#[test]
fn utf16_is_the_default_when_server_omits_the_field() {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", canned_result(None));
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    let (_id, ev) = backend.drain().into_iter().next().unwrap();
    client.on_event(ev);

    assert_eq!(client.encoding, PositionEncoding::Utf16);
}

#[test]
fn eof_transitions_to_crashed_and_further_sends_do_not_panic() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    let actions = client.on_event(InboundEvent::Eof {
        error: Some("server exited".to_string()),
    });
    assert_eq!(client.state, ServerState::Crashed);
    match &actions[..] {
        [ClientAction::Crashed { error }] => {
            assert_eq!(error.as_deref(), Some("server exited"));
        }
        other => panic!("expected one Crashed action, got {other:?}"),
    }

    // A send after Crashed must not panic — silently dropped, matching
    // the transport's own send-after-death discipline.
    client.send_or_queue(
        &mut backend,
        Message::Notification {
            method: "textDocument/didOpen".to_string(),
            params: serde_json::Value::Null,
        },
    );
}

/// A request filed against an already-Crashed client is silently dropped
/// on the wire (see `send_or_queue`) and nothing will ever answer it —
/// its `meta.deadline` must be clamped to now so `take_completed`'s sweep
/// resolves it as `TimedOut` on the very next tick, instead of leaving
/// the caller waiting out the deadline it asked for (routinely tens of
/// seconds) for a request that was doomed the moment it was sent.
#[test]
fn send_request_after_crashed_times_out_immediately_via_the_sweep() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.on_event(InboundEvent::Eof {
        error: Some("server exited".to_string()),
    });
    assert_eq!(client.state, ServerState::Crashed);

    let far_future = Instant::now() + std::time::Duration::from_secs(30);
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: far_future,
    };
    let id = client.send_request(
        &mut backend,
        "textDocument/hover",
        serde_json::Value::Null,
        meta,
    );

    assert!(
        client.earliest_deadline().expect("still pending") <= Instant::now(),
        "deadline must be clamped to now, not left at the caller's far-future value"
    );

    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert!(actions.is_empty());
    assert_eq!(completed.len(), 1);
    let (returned_id, _meta, outcome) = &completed[0];
    assert_eq!(*returned_id, id);
    assert!(
        matches!(outcome, Outcome::TimedOut),
        "expected TimedOut, got {outcome:?}"
    );
}

#[test]
fn shutdown_sends_shutdown_request_then_exit_notification_in_order() {
    let (mut backend, mut client) = make_running_client();
    // `make_running_client` already sent `initialize` — this test only
    // asserts on what `begin_shutdown` adds after it.
    let before = backend.sent.len();

    client.begin_shutdown(&mut backend);

    assert_eq!(client.state, ServerState::Dead);
    assert_eq!(backend.sent.len(), before + 2);
    match &backend.sent[before] {
        (_, Message::Request { method, .. }) => assert_eq!(method, "shutdown"),
        other => panic!("expected the shutdown request first, got {other:?}"),
    }
    match &backend.sent[before + 1] {
        (_, Message::Notification { method, .. }) => assert_eq!(method, "exit"),
        other => panic!("expected the exit notification second, got {other:?}"),
    }
}

/// Regression: nothing but `initialize` is legal on the wire before
/// `initialized` — `begin_shutdown` on a still-Starting client must
/// send neither `shutdown` nor `exit` (it still transitions to `Dead`;
/// transport-level teardown reaps the process regardless).
#[test]
fn begin_shutdown_sends_nothing_while_still_starting() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    assert_eq!(client.state, ServerState::Starting);

    client.begin_shutdown(&mut backend);

    assert_eq!(client.state, ServerState::Dead);
    assert!(
        backend.sent.is_empty(),
        "must not send shutdown/exit before the handshake completed: {:?}",
        backend.sent
    );
}

/// Regression: `begin_shutdown` on a still-`Starting` client jumps
/// straight to `Dead` without cancelling (or waiting for) the in-flight
/// `initialize` — its `pending`/`initialize_id` entries are untouched.
/// A response that lands afterward must not resurrect the client into
/// `Running` via `handle_initialize_response`'s unconditional state
/// overwrite.
#[test]
fn initialize_response_after_shutdown_while_starting_does_not_resurrect_the_client() {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", canned_result(None));
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));

    client.start_handshake(&mut backend);
    client.begin_shutdown(&mut backend);
    assert_eq!(client.state, ServerState::Dead);

    let (_id, ev) = backend
        .drain()
        .into_iter()
        .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
        .expect("initialize response");
    let actions = client.on_event(ev);

    assert!(
        actions.is_empty(),
        "a late initialize response must not surface any action once already Dead: {actions:?}"
    );
    assert_eq!(
        client.state,
        ServerState::Dead,
        "state must stay Dead, not flip back to Running"
    );
}

#[test]
fn shutdown_response_surfaces_through_take_completed() {
    let (mut backend, mut client) = make_running_client();
    backend.respond_to("shutdown", serde_json::Value::Null);

    client.begin_shutdown(&mut backend);
    let (_sid, ev) = backend
        .drain()
        .into_iter()
        .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
        .expect("shutdown response");
    let actions = client.on_event(ev);
    assert!(actions.is_empty());

    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert!(actions.is_empty());
    assert_eq!(completed.len(), 1);
    let (_id, meta, outcome) = &completed[0];
    assert_eq!(meta.method, "shutdown");
    match outcome {
        Outcome::Ok(v) => assert_eq!(*v, serde_json::Value::Null),
        other => panic!("expected Ok(null), got {other:?}"),
    }
    assert_eq!(client.pending_count(), 0);
}

#[test]
fn shutdown_error_surfaces_as_err() {
    let (mut backend, mut client) = make_running_client();
    backend.fail_with("shutdown", -32603, "internal error");

    client.begin_shutdown(&mut backend);
    let (_sid, ev) = backend
        .drain()
        .into_iter()
        .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
        .expect("shutdown response");
    client.on_event(ev);

    let (completed, _actions) = client.take_completed(&mut backend, Instant::now());
    assert_eq!(completed.len(), 1);
    let (_id, meta, outcome) = &completed[0];
    assert_eq!(meta.method, "shutdown");
    match outcome {
        Outcome::Err(e) => assert_eq!(e.message, "internal error"),
        other => panic!("expected Err, got {other:?}"),
    }
}

fn make_running_client() -> (InlineLspBackend, LspClient) {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(&mut backend);
    let (_id, ev) = backend.drain().into_iter().next().unwrap();
    client.on_event(ev);
    assert_eq!(client.state, ServerState::Running);
    (backend, client)
}

#[test]
fn send_request_delivers_response_via_take_completed() {
    let (mut backend, mut client) = make_running_client();
    backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));

    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let sent_id = client.send_request(
        &mut backend,
        "textDocument/hover",
        serde_json::Value::Null,
        meta,
    );

    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    assert!(
        actions.is_empty(),
        "a correlated response produces no ClientAction — it's pulled via take_completed"
    );

    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert_eq!(completed.len(), 1);
    assert!(actions.is_empty());
    let (id, meta_out, outcome) = &completed[0];
    assert_eq!(*id, sent_id);
    assert_eq!(meta_out.method, "textDocument/hover");
    match outcome {
        Outcome::Ok(v) => assert_eq!(*v, serde_json::json!({"contents": "hi"})),
        other => panic!("expected Ok, got {other:?}"),
    }

    // Pulled once — a second call finds nothing left.
    let (completed2, actions2) = client.take_completed(&mut backend, Instant::now());
    assert!(completed2.is_empty());
    assert!(actions2.is_empty());
}

#[test]
fn cancel_removes_pending_and_sends_cancel_notification() {
    let (mut backend, mut client) = make_running_client();

    let meta = RequestMeta {
        method: "textDocument/definition".to_string(),
        allow_stale: false,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let id = client.send_request(
        &mut backend,
        "textDocument/definition",
        serde_json::Value::Null,
        meta,
    );
    client.cancel(&mut backend, id.clone());

    match backend.sent.last() {
        Some((_, Message::Notification { method, params })) => {
            assert_eq!(method, "$/cancelRequest");
            assert_eq!(params, &cancel_request_params(&id));
        }
        other => panic!("expected a $/cancelRequest notification, got {other:?}"),
    }

    // A late response for the already-cancelled id must not resurrect it.
    backend.push_from_server(
        client.id,
        Message::Response {
            id: id.clone(),
            result: Ok(serde_json::Value::Null),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    client.on_event(ev);
    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert!(completed.is_empty());
    assert!(actions.is_empty());
}

/// Minor regression: nothing but `initialize` is legal on the wire
/// before `initialized` — a request cancelled or timed out while still
/// `Starting` must not put `$/cancelRequest` on the wire, since its own
/// send is still sitting in `queued`, unsent, and the server never saw
/// it in the first place.
#[test]
fn cancel_and_timeout_send_no_cancel_request_while_still_starting() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    assert_eq!(client.state, ServerState::Starting);

    let meta = RequestMeta {
        method: "textDocument/definition".to_string(),
        allow_stale: false,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let id = client.send_request(
        &mut backend,
        "textDocument/definition",
        serde_json::Value::Null,
        meta,
    );
    client.cancel(&mut backend, id);
    assert!(
        backend.sent.is_empty(),
        "cancelling while Starting must not send anything"
    );

    let meta2 = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() - std::time::Duration::from_millis(1),
    };
    client.send_request(
        &mut backend,
        "textDocument/hover",
        serde_json::Value::Null,
        meta2,
    );
    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert_eq!(completed.len(), 1);
    assert!(matches!(completed[0].2, Outcome::TimedOut));
    assert!(actions.is_empty());
    assert!(
        backend.sent.is_empty(),
        "a timeout while still Starting must not send $/cancelRequest either"
    );
}

/// Regression: a request cancelled while still `Starting` must not
/// resurface once the handshake completes — its `Message::Request` sat
/// unsent in `queued` (removed from `pending` by `cancel`), and without
/// also stripping it from `queued`, `handle_initialize_response`'s
/// flush would still deliver it to the server with no pending entry
/// left to correlate a response (or send `$/cancelRequest` for).
#[test]
fn cancelled_request_is_not_flushed_after_handshake_completes() {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", canned_result(None));
    let sid = backend
        .start("x", &[], std::path::Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(&mut backend);

    let meta = RequestMeta {
        method: "textDocument/definition".to_string(),
        allow_stale: false,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let id = client.send_request(
        &mut backend,
        "textDocument/definition",
        serde_json::Value::Null,
        meta,
    );
    client.cancel(&mut backend, id);

    let (_id, ev) = backend
        .drain()
        .into_iter()
        .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
        .expect("initialize response");
    let actions = client.on_event(ev);
    match &actions[..] {
        [ClientAction::BecameRunning { send }] => {
            assert_eq!(
                send.len(),
                1,
                "only 'initialized' should flush — the cancelled request must not reappear: {send:?}"
            );
            match &send[0] {
                Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                other => panic!("expected only the initialized notification, got {other:?}"),
            }
        }
        other => panic!("expected one BecameRunning action, got {other:?}"),
    }
}

#[test]
fn take_completed_reports_timeout_and_sends_cancel_request() {
    let (mut backend, mut client) = make_running_client();
    let meta = RequestMeta {
        method: "textDocument/completion".to_string(),
        allow_stale: false,
        deadline: Instant::now() - std::time::Duration::from_millis(1),
    };
    let id = client.send_request(
        &mut backend,
        "textDocument/completion",
        serde_json::Value::Null,
        meta,
    );

    let (completed, actions) = client.take_completed(&mut backend, Instant::now());
    assert_eq!(completed.len(), 1);
    assert!(actions.is_empty());
    let (returned_id, _meta, outcome) = &completed[0];
    assert_eq!(*returned_id, id);
    assert!(matches!(outcome, Outcome::TimedOut));

    match backend.sent.last() {
        Some((_, Message::Notification { method, params })) => {
            assert_eq!(method, "$/cancelRequest");
            assert_eq!(params, &cancel_request_params(&id));
        }
        other => panic!("expected a $/cancelRequest notification, got {other:?}"),
    }

    // Removed from pending — a second call must not report it again.
    let (completed2, actions2) = client.take_completed(&mut backend, Instant::now());
    assert!(completed2.is_empty());
    assert!(actions2.is_empty());
}

#[test]
fn server_initiated_request_becomes_a_client_action() {
    let (mut backend, mut client) = make_running_client();
    backend.push_from_server(
        client.id,
        Message::Request {
            id: RequestId::Int(99),
            method: "workspace/configuration".to_string(),
            params: serde_json::json!({"items": []}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    match &actions[..] {
        [ClientAction::ServerRequest { id, method, .. }] => {
            assert_eq!(*id, RequestId::Int(99));
            assert_eq!(method, "workspace/configuration");
        }
        other => panic!("expected one ServerRequest action, got {other:?}"),
    }
}

#[test]
fn server_notification_becomes_a_client_action() {
    let (mut backend, mut client) = make_running_client();
    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "custom/thing".to_string(),
            params: serde_json::json!({"anything": true}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    match &actions[..] {
        [ClientAction::ServerNotification { method, .. }] => {
            assert_eq!(method, "custom/thing");
        }
        other => panic!("expected one ServerNotification action, got {other:?}"),
    }
}

#[test]
fn publish_diagnostics_notification_classifies_as_typed_diagnostics() {
    let (mut backend, mut client) = make_running_client();
    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({"uri": "file:///a", "diagnostics": []}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    match &actions[..] {
        [ClientAction::Diagnostics(p)] => {
            assert_eq!(p.uri.as_str(), "file:///a");
            assert!(p.diagnostics.is_empty());
        }
        other => panic!("expected one Diagnostics action, got {other:?}"),
    }
}

#[test]
fn progress_log_and_show_message_classify_as_typed_variants() {
    let (mut backend, mut client) = make_running_client();

    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "$/progress".to_string(),
            params: serde_json::json!({
                "token": "t1",
                "value": {"kind": "begin", "title": "Indexing"},
            }),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::Progress(p)] => {
            assert_eq!(p.token, lsp_types::NumberOrString::String("t1".to_string()));
        }
        other => panic!("expected one Progress action, got {other:?}"),
    }

    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "window/logMessage".to_string(),
            params: serde_json::json!({"type": 1, "message": "boom"}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::LogMessage(p)] => {
            assert_eq!(p.message, "boom");
            assert_eq!(p.typ, lsp_types::MessageType::ERROR);
        }
        other => panic!("expected one LogMessage action, got {other:?}"),
    }

    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "window/showMessage".to_string(),
            params: serde_json::json!({"type": 3, "message": "hi"}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::ShowMessage(p)] => {
            assert_eq!(p.message, "hi");
            assert_eq!(p.typ, lsp_types::MessageType::INFO);
        }
        other => panic!("expected one ShowMessage action, got {other:?}"),
    }
}

#[test]
fn malformed_known_method_falls_through_as_server_notification() {
    let (mut backend, mut client) = make_running_client();

    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({"uri": 42}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::ServerNotification { method, params }] => {
            assert_eq!(method, "textDocument/publishDiagnostics");
            assert_eq!(params, &serde_json::json!({"uri": 42}));
        }
        other => panic!("expected fallthrough ServerNotification, got {other:?}"),
    }

    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "$/progress".to_string(),
            params: serde_json::json!({"token": {}}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::ServerNotification { method, params }] => {
            assert_eq!(method, "$/progress");
            assert_eq!(params, &serde_json::json!({"token": {}}));
        }
        other => panic!("expected fallthrough ServerNotification, got {other:?}"),
    }
}

#[test]
fn progress_begin_missing_title_recovers_via_lenient_fallback() {
    // A server that treats `title` as optional in practice, even though
    // `WorkDoneProgressBegin::title` is spec-required — the strict parse
    // fails, `recover_progress` patches in a placeholder and retries.
    let (mut backend, mut client) = make_running_client();
    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "$/progress".to_string(),
            params: serde_json::json!({"token": "t1", "value": {"kind": "begin"}}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::Progress(p)] => {
            assert_eq!(p.token, lsp_types::NumberOrString::String("t1".to_string()));
            match &p.value {
                lsp_types::ProgressParamsValue::WorkDone(lsp_types::WorkDoneProgress::Begin(
                    begin,
                )) => assert!(!begin.title.is_empty(), "must recover a non-empty title"),
                other => panic!("expected a Begin progress value, got {other:?}"),
            }
        }
        other => panic!("expected one recovered Progress action, got {other:?}"),
    }
}

#[test]
fn log_message_missing_type_recovers_via_lenient_fallback() {
    let (mut backend, mut client) = make_running_client();
    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "window/logMessage".to_string(),
            params: serde_json::json!({"message": "boom"}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::LogMessage(p)] => {
            assert_eq!(p.message, "boom");
            assert_eq!(p.typ, lsp_types::MessageType::LOG);
        }
        other => panic!("expected one recovered LogMessage action, got {other:?}"),
    }
}

#[test]
fn show_message_non_integer_type_recovers_via_lenient_fallback() {
    let (mut backend, mut client) = make_running_client();
    backend.push_from_server(
        client.id,
        Message::Notification {
            method: "window/showMessage".to_string(),
            params: serde_json::json!({"type": "info", "message": "hi"}),
        },
    );
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    match &client.on_event(ev)[..] {
        [ClientAction::ShowMessage(p)] => {
            assert_eq!(p.message, "hi");
            assert_eq!(p.typ, lsp_types::MessageType::INFO);
        }
        other => panic!("expected one recovered ShowMessage action, got {other:?}"),
    }
}

#[test]
fn stderr_event_becomes_a_client_action() {
    let (_backend, mut client) = make_running_client();
    let actions = client.on_event(InboundEvent::Stderr("panic: oh no".to_string()));
    match &actions[..] {
        [ClientAction::Stderr(line)] => assert_eq!(line, "panic: oh no"),
        other => panic!("expected one Stderr action, got {other:?}"),
    }
}

#[test]
fn eof_reports_crashed_only_once_even_if_fed_again() {
    let (_backend, mut client) = make_running_client();
    let first = client.on_event(InboundEvent::Eof { error: None });
    assert_eq!(first.len(), 1);
    let second = client.on_event(InboundEvent::Eof { error: None });
    assert!(
        second.is_empty(),
        "a second Eof after already-Crashed must not report again"
    );
}

/// Minor regression: a trailing `Eof` racing a graceful `begin_shutdown`
/// teardown must not report a spurious "server crashed" — `Dead` is as
/// valid a "connection is already known gone, on purpose" state as
/// `Crashed`.
#[test]
fn eof_after_a_graceful_shutdown_does_not_report_crashed() {
    let (mut backend, mut client) = make_running_client();
    client.begin_shutdown(&mut backend);
    assert_eq!(client.state, ServerState::Dead);

    let actions = client.on_event(InboundEvent::Eof { error: None });
    assert!(
        actions.is_empty(),
        "an Eof after a graceful shutdown must not surface a Crashed action"
    );
    assert_eq!(
        client.state,
        ServerState::Dead,
        "state must stay Dead, not flip to Crashed"
    );
}

// ── server_request_response ──────────────────────────────────────────────

#[test]
fn workspace_configuration_answers_null_per_item_with_no_settings() {
    let params =
        serde_json::json!({"items": [{"section": "rust-analyzer"}, {"section": "editor"}]});
    let result = server_request_response("workspace/configuration", &params, None).unwrap();
    assert_eq!(result, serde_json::json!([null, null]));
}

#[test]
fn workspace_configuration_with_no_items_answers_empty_array() {
    let params = serde_json::json!({"items": []});
    let result = server_request_response("workspace/configuration", &params, None).unwrap();
    assert_eq!(result, serde_json::json!([]));
}

#[test]
fn workspace_configuration_resolves_sections_from_settings() {
    let settings = serde_json::json!({"rust-analyzer": {"cargo": {"features": "all"}}});
    let params = serde_json::json!({"items": [
        {"section": "rust-analyzer.cargo.features"},
        {"section": "rust-analyzer"},
        {"section": "nope"},
        {},
    ]});
    let result =
        server_request_response("workspace/configuration", &params, Some(&settings)).unwrap();
    assert_eq!(
        result,
        serde_json::json!([
            "all",
            {"cargo": {"features": "all"}},
            null,
            settings,
        ])
    );
}

// workspace/applyEdit is answered separately (needs `&mut Editor`) — see
// hume-editor's `editor::tests::lsp_edits` for its coverage.

#[test]
fn register_and_unregister_capability_and_progress_create_answer_null() {
    for method in [
        "client/registerCapability",
        "client/unregisterCapability",
        "window/workDoneProgress/create",
    ] {
        let result = server_request_response(method, &serde_json::Value::Null, None).unwrap();
        assert_eq!(result, serde_json::Value::Null, "method {method}");
    }
}

#[test]
fn unknown_server_request_is_method_not_found() {
    let err =
        server_request_response("some/madeUpMethod", &serde_json::Value::Null, None).unwrap_err();
    assert_eq!(err.code, -32601);
    assert!(err.message.contains("some/madeUpMethod"));
}

// ── resolve_config_section ────────────────────────────────────────────────

#[test]
fn resolve_config_section_exact_hit() {
    let settings = serde_json::json!({"a": {"b": 1}});
    assert_eq!(
        resolve_config_section(&settings, Some("a")),
        serde_json::json!({"b": 1})
    );
}

#[test]
fn resolve_config_section_nested_dotted_hit() {
    let settings = serde_json::json!({"a": {"b": {"c": 42}}});
    assert_eq!(
        resolve_config_section(&settings, Some("a.b.c")),
        serde_json::json!(42)
    );
}

#[test]
fn resolve_config_section_missing_key_is_null() {
    let settings = serde_json::json!({"a": {"b": 1}});
    assert_eq!(
        resolve_config_section(&settings, Some("a.missing")),
        serde_json::Value::Null
    );
}

#[test]
fn resolve_config_section_non_object_mid_path_is_null() {
    let settings = serde_json::json!({"a": 1});
    assert_eq!(
        resolve_config_section(&settings, Some("a.b")),
        serde_json::Value::Null
    );
}

#[test]
fn resolve_config_section_absent_returns_whole_blob() {
    let settings = serde_json::json!({"a": 1, "b": 2});
    assert_eq!(resolve_config_section(&settings, None), settings);
}

#[test]
fn resolve_config_section_empty_string_returns_whole_blob() {
    let settings = serde_json::json!({"a": 1});
    assert_eq!(resolve_config_section(&settings, Some("")), settings);
}
