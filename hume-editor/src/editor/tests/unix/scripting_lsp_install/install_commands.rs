//! `:lsp-install` failures and the up-to-date path.

use super::*;

// ── :lsp-install failure paths ────────────────────────────────────────────────

#[test]
fn lsp_install_stub_kind_names_the_unsupported_kind() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // ocamllsp's Mason source is purl kind `opam`, a stub, never installable in v1.
    type_cmd(&mut ed, ":lsp-install ocaml");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("opam"),
        "a stub-kind server must fail naming the unsupported purl kind: {log}"
    );
}

#[test]
fn lsp_install_cargo_git_stub_kind_names_the_kind() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // nil's Mason source pins a git tag (2025-06-13), not a crates.io
    // version, so it's downgraded to stub kind `cargo-git`, never installable.
    type_cmd(&mut ed, ":lsp-install nix");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("cargo-git"),
        "a cargo-git stub-kind server must fail naming the kind: {log}"
    );
}

#[test]
fn lsp_install_unknown_language_reports_an_error() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-install: no language or server named"),
        "an unseeded language must report, not no-op: {log}"
    );
}

/// Tab on `:lsp-install`'s argument completes against the seeded catalog's
/// own languages, read from the real `runtime/plugins/core/lsp-install/data/language-servers.scm` (not
/// a fixture subset): "rus" matches only "rust" (unlike "ru", which also
/// matches "ruby").
#[test]
fn lsp_install_tab_completes_a_seeded_language() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    ed.handle_key(key(':'));
    type_chars(&mut ed, "lsp-install rus");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "lsp-install rust");
}

#[test]
fn lsp_install_no_language_buffer_and_no_arg_warns() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // Fresh test buffer has no language set.
    type_cmd(&mut ed, ":lsp-install");

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("no language given") || msg.contains("no language set"),
        "no arg + no buffer language must report a status message: {msg}"
    );
}

#[test]
fn lsp_install_tar_archive_requires_tar_on_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // ada-language-server ships only .tar.gz on every platform.
    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    type_cmd(&mut ed, ":lsp-install ada");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'tar' on $PATH"),
        "a tar-archive server must require tar before downloading: {log}"
    );
}

/// `lsp-install/with-lock!`'s failure branch (`thunk` raised) must still
/// release the lock: a failed install must not permanently wedge every
/// later `:lsp-install`/`:lsp-uninstall` behind a lock nothing will ever
/// release.
#[test]
fn install_lock_is_released_after_a_failed_install() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // Fails inside lsp-install/install-server! (lsp-install/install-blocker), i.e. inside
    // lsp-install/with-lock!'s thunk. Exercises the release-on-failure path,
    // not the release-on-success path every other install test hits.
    type_cmd(&mut ed, ":lsp-install ocaml");
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("opam"),
        "sanity: the install must actually have failed: {log}"
    );

    let lock_path = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join(".install-lock");
    std::fs::File::open(&lock_path)
        .unwrap()
        .try_lock()
        .expect("a failed install must release the cross-process lock");

    // A second, unrelated install must be able to acquire the lock.
    type_cmd(&mut ed, ":lsp-install ocaml");
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already in progress"),
        "a later install must not find the lock still held: {log}"
    );
}

/// A lock another process holds (as another HUME mid-install would) must
/// refuse the install loudly, before any network activity.
/// `lsp-install/acquire-lock!` fails first, so this never reaches
/// rust-analyzer's real download path.
#[test]
fn lsp_install_refuses_when_the_cross_process_lock_is_already_held() {
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    let other = std::fs::File::create(servers_dir.join(".install-lock")).unwrap();
    other.try_lock().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install rust");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("already in progress"),
        "a held cross-process lock must refuse the install loudly: {log}"
    );
}

/// A lock file nobody holds does not block: the file an install the budget
/// stopped leaves behind, here naming this very process, is not a lock.
#[test]
fn lsp_install_ignores_a_lock_file_nobody_holds() {
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    std::fs::write(
        servers_dir.join(".install-lock"),
        std::process::id().to_string(),
    )
    .unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());
    // ocamllsp fails inside the lock (unsupported `opam` kind), never
    // reaching the network.
    type_cmd(&mut ed, ":lsp-install ocaml");

    let log = ed.state.message_log.format_for_display();
    assert!(log.contains("opam"), "the install must run: {log}");
    assert!(
        !log.contains("already in progress"),
        "a lock file nobody holds must not refuse the install: {log}"
    );
}

/// Proves the minibuffer's `IntV(1)` no-arg sentinel (see
/// `dispatch.rs`'s `marshal_leading_args`) takes the buffer-language
/// fallback branch, not the "no argument given" branch: a made-up language
/// name only this test's buffer has makes the distinction unambiguous.
#[test]
fn lsp_install_no_arg_falls_back_to_buffer_language_not_the_count_sentinel() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("definitely-not-seeded");
    ed.set_buffer_language(bid, Some(lang));

    type_cmd(&mut ed, ":lsp-install");

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("definitely-not-seeded"),
        "no-arg :lsp-install must resolve to the buffer's language, not misread the \
         minibuffer's IntV(1) count sentinel as a string argument or as 'no language': {msg}"
    );
}

// ── :lsp-install up-to-date path ──────────────────────────────────────────────

/// The up-to-date path still re-registers: a receipt written after the
/// load-time scan already ran must be picked
/// up by `:lsp-install`'s own post-check rescan, not just reported as
/// up-to-date and left unregistered.
#[test]
fn lsp_install_up_to_date_registers_a_late_fabricated_receipt() {
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        None,
        "precondition: nothing installed at load time, so core:lsp-install's load-time scan \
         registered nothing"
    );
    // Fabricate the receipt only now, after the load-time scan already ran
    // against an empty data dir, so the final assertion below can only pass
    // if :lsp-install's own up-to-date rescan registers it, not the
    // load-time scan.
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-10-05",
        "rust-analyzer",
    );

    type_cmd(&mut ed, ":lsp-install rust");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("LSP: rust-analyzer already installed (v2026-10-05) — up to date"),
        "must report the up-to-date status"
    );
    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "registration must survive the up-to-date :lsp-install rescan"
    );
}

/// `declared-plugins` includes `core:*` names. PLUM's own install-list logic
/// must still exclude them: `:plum-install-plugins`/`:plum-list-plugins` must never treat a
/// bundled core plugin as something to `git clone`. `:plum-list-plugins`'s
/// "Missing plugins" section is the safe way to observe `plum/missing-plugins`'s
/// output without ever touching the network.
#[test]
fn plum_missing_plugins_excludes_declared_core_plugins() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:plum\") (%activate-plugin-inline! \"core:plum\" #f)\n\
         (declare-plugin! \"core:lsp-install\" #:languages '(\"rust\"))",
    );

    type_cmd(&mut ed, ":plum-list-plugins");

    insta::assert_snapshot!(ed.doc().text().to_string(), @"
    Declared plugins (3)
      core:stdlib
      core:plum
      core:lsp-install

    Installed plugins (0)
      none

    Orphan plugins (0)
      none

    Missing plugins (0)
      none
    ");
}
