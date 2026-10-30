//! `:lsp-uninstall`, `:lsp-catalog`, and status, stop and restart.

use super::*;

// ── :lsp-uninstall ────────────────────────────────────────────────────────────

#[test]
fn lsp_uninstall_removes_registration_and_directory() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        &seeded_version("rust-analyzer"),
        "rust-analyzer",
    );
    stop_an_install(data_tmp.path(), "rust-analyzer");

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp-install\")",
    );
    type_cmd(&mut ed, ":lsp-install rust-analyzer");
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .is_some(),
        "precondition: scan must have registered the fabricated install"
    );

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        None,
        "uninstall must unregister the server"
    );
    let dir = server_dir(data_tmp.path(), "rust-analyzer");
    assert!(
        !dir.exists(),
        "uninstall must remove the server directory, with every install folder in it, once the deferred (after! 0 ...) fires"
    );
}

/// Tab on `:lsp-uninstall`'s argument completes against every server with
/// an install dir on disk: the same set the command itself accepts,
/// including an orphan no longer in the seeded catalog (uninstall's own
/// `(path-exists? dir)` check, not the catalog, decides what's removable).
#[test]
fn lsp_uninstall_tab_completes_an_installed_server() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    ed.handle_key(key(':'));
    type_chars(&mut ed, "lsp-uninstall rust-a");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "lsp-uninstall rust-analyzer");
}

/// The uninstall delete is guarded by the same cross-process lock: a lock
/// another process holds when the deferred `(after! 0 ...)` callback fires
/// must refuse the delete loudly, leaving the directory intact.
#[test]
fn lsp_uninstall_refuses_the_delete_when_the_cross_process_lock_is_already_held() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    let _other = hold_install_lock(data_tmp.path());

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("already in progress"),
        "a live cross-process lock must refuse the delete loudly: {log}"
    );
    assert!(
        server_dir(data_tmp.path(), "rust-analyzer").exists(),
        "the server directory must survive when the lock can't be acquired"
    );
}

#[test]
fn lsp_uninstall_of_never_installed_server_is_silent() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.settle();

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "uninstalling an already-absent/never-installed server must succeed silently: {errors:?}"
    );
    // "nothing to uninstall" is `'info`, display-only (status line), never
    // written to `:messages` (see message_log.rs's Severity table), so the
    // confirmation shows up in `status_msg`, not `message_log`.
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("LSP: nothing to uninstall for rust-analyzer"),
        "must report there was nothing to do"
    );
}

#[test]
fn lsp_uninstall_rejects_path_traversal_name() {
    let data_tmp = tempfile::tempdir().unwrap();
    // A sibling write-sandbox dir `../plugins` would canonicalize into:
    // it must survive untouched.
    let plugins_dir = canonical_data_dir(data_tmp.path()).join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(plugins_dir.join("sentinel"), b"do not delete me").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall ../plugins");
    ed.drain_async_sources();
    ed.settle();

    assert!(
        plugins_dir.join("sentinel").exists(),
        "path-traversal uninstall must never reach a sibling sandbox directory"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-uninstall: invalid server name: ../plugins"),
        "must warn loudly about the rejected name: {log}"
    );
}

/// `safe-path-segment?` (used by `lsp-uninstall`) must reject `:` and
/// `"`, not just `.`/`..`/path separators: the drive-relative-root escape
/// `hume_platform::path::is_safe_segment` exists to block.
#[test]
fn lsp_uninstall_rejects_colon_and_quote_in_name() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall c:evil");
    ed.drain_async_sources();
    ed.settle();
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-uninstall: invalid server name: c:evil"),
        "must warn loudly about a drive-relative-root name: {log}"
    );

    type_cmd(&mut ed, ":lsp-uninstall a\"b");
    ed.drain_async_sources();
    ed.settle();
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-uninstall: invalid server name: a\"b"),
        "must warn loudly about a quote-embedded name: {log}"
    );
}

#[test]
fn lsp_uninstall_without_a_name_reports_an_error() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-uninstall: requires a server name"),
        "{log}"
    );
}

// ── :lsp-catalog ──────────────────────────────────────────────────────────────

/// `:lsp-catalog` shows the seeded servers in the read-only `[lsp-catalog]`
/// buffer: a counted header, then one line per server sorted by name, an
/// installed one with its version.
#[test]
fn lsp_catalog_shows_the_servers_in_a_read_only_buffer() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-catalog");

    assert!(
        !has_error(&ed, ""),
        "{}",
        ed.state.message_log.format_for_display()
    );
    assert_eq!(ed.doc().display_name(), "[lsp-catalog]");
    assert!(ed.doc().is_read_only());
    let text = ed.doc().text().to_string();
    let mut lines = text.lines();
    let header = lines.next().expect("a header line");
    let items: Vec<&str> = lines.map(|l| l.trim_start()).collect();
    assert_eq!(header, format!("LSP servers ({})", items.len()));
    let mut sorted = items.clone();
    sorted.sort_unstable();
    assert_eq!(items, sorted, "servers are listed by name");
    assert!(
        items
            .iter()
            .any(|l| l.starts_with("rust-analyzer [") && l.contains("]: installed v2026-07-06")),
        "{text}"
    );
}

// ── :lsp-status / :lsp-stop / :lsp-restart ─────────────────────────────────────
//
// These live in core:lsp, dispatched through Steel to the
// `lsp-show-status!`/`lsp-stop!`/`lsp-restart!` builtins.

#[test]
fn lsp_status_opens_a_read_only_view_when_no_servers_are_running() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-status");

    assert_eq!(ed.doc().display_name(), "[lsp-status]");
    assert_eq!(ed.doc().text().to_string(), "No LSP servers running.\n");
}

#[test]
fn lsp_stop_with_no_matching_server_reports_nothing_to_stop() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-stop");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("lsp: no matching server to stop")
    );
}

#[test]
fn lsp_restart_with_no_matching_server_reports_nothing_to_restart() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-restart");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("lsp: no matching server to restart")
    );
}

#[test]
fn lsp_client_alone_exposes_no_install_commands_and_registers_nothing() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "rust-analyzer", "1.0.0", "rust-analyzer");
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    for cmd in [":lsp-install", ":lsp-uninstall", ":lsp-catalog"] {
        type_cmd(&mut ed, cmd);
        let log = ed.state.message_log.format_for_display();
        assert!(
            log.contains(&format!("unknown command: {}", &cmd[1..])),
            "core:lsp alone must not expose {cmd}: {log}"
        );
    }
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .is_none(),
        "core:lsp alone must not scan installed servers"
    );
}

#[test]
fn plum_alone_does_not_expose_lsp_status_stop_restart() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    for cmd in [":lsp-status", ":lsp-stop", ":lsp-restart"] {
        type_cmd(&mut ed, cmd);
        let log = ed.state.message_log.format_for_display();
        assert!(
            log.contains(&format!("unknown command: {}", &cmd[1..])),
            "core:plum alone must not expose {cmd}: {log}"
        );
    }
}
