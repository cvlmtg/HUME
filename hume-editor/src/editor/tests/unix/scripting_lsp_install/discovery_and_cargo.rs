//! The discovery hint and the cargo installer.

use super::*;

// ── Discovery hint ────────────────────────────────────────────────────────────
//
// `ed.set_buffer_language` + `ed.settle()` is not a `:`-typed command
// dispatch: it is the same path a buffer opened via a CLI argument at
// startup takes. These tests therefore also cover that the hook body's
// `lsp-language-servers` call is safe outside a typed
// command's dispatch, not only after one.

#[test]
fn discovery_hint_fires_once_for_an_installable_unregistered_language() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert_eq!(
        log.matches("run :lsp-install").count(),
        1,
        "the hint must fire exactly once: {log}"
    );
    assert!(
        log.contains("rust-analyzer"),
        "the hint must name the seeded server: {log}"
    );

    // Revisit the same language later in the session: must not repeat.
    ed.set_buffer_language(bid, None);
    ed.settle();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();
    let log2 = ed.state.message_log.format_for_display();
    assert_eq!(
        log2.matches("run :lsp-install").count(),
        1,
        "the hint must not repeat for a language already evaluated this session: {log2}"
    );
}

#[test]
fn discovery_hint_does_not_fire_for_a_blocked_server() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    // nil (cargo-git stub) is never installable, so the hint must never
    // suggest a command that would fail.
    let lang = ed.state.config.languages.intern("nix");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint a suggestion that would fail: {log}"
    );
}

#[test]
fn discovery_hint_does_not_fire_for_npm_kind_when_npm_missing_from_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // svlangserver (npm-kind, language "systemverilog") must not report
    // installable unconditionally. Reporting installable regardless of npm
    // availability could suggest a :lsp-install that immediately fails the
    // npm-on-$PATH check. Force $PATH to a directory
    // with no npm binary in it.
    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("systemverilog");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint an npm-kind install when npm is not on $PATH: {log}"
    );
}

#[test]
fn discovery_hint_fires_for_cargo_kind() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // pest-language-server (cargo-kind, language "pest") must report
    // installable: core:lsp-install has a cargo installer, and cargo is
    // guaranteed on $PATH since this test suite itself runs under cargo.
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("pest");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("run :lsp-install"),
        "a now-installable cargo-kind server must hint: {log}"
    );
    assert!(
        log.contains("pest-language-server"),
        "the hint must name the seeded server: {log}"
    );
}

#[test]
fn discovery_hint_does_not_fire_for_cargo_kind_when_cargo_missing_from_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // Force $PATH to a directory with no cargo binary in it: must not hint
    // an install that would immediately fail the cargo-on-$PATH check.
    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("pest");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint a cargo-kind install when cargo is not on $PATH: {log}"
    );
}

#[test]
fn discovery_hint_does_not_fire_for_download_kind_when_download_tools_missing_from_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    // rust-analyzer is a github-kind download: it needs curl and gzip.
    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint a download install when curl is not on $PATH: {log}"
    );
}

#[test]
fn discovery_hint_does_not_fire_when_already_registered() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path()); // core:lsp-install's own scan registers rust-analyzer for "rust"

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must not hint when the language is already registered: {log}"
    );
}

/// An installed server later in the language's default list serves it, so
/// the primary's hint stays quiet. python lists ty before ruff.
#[test]
fn discovery_hint_does_not_fire_when_a_later_default_server_is_installed() {
    let python_hints = |installed: &[&str]| {
        let data_tmp = tempfile::tempdir().unwrap();
        for server in installed {
            fabricate_server(data_tmp.path(), server, "0.0.1", server);
        }
        let mut ed = editor_from("-[x]>\n");
        load_lsp_install(&mut ed, data_tmp.path());
        let bid = ed.focused_buffer_id();
        let lang = ed.state.config.languages.intern("python");
        ed.set_buffer_language(bid, Some(lang));
        ed.settle();
        ed.state.message_log.format_for_display()
    };

    let log = python_hints(&[]);
    assert!(
        log.contains("'ty' is available for python"),
        "precondition: ty is hinted with nothing installed: {log}"
    );
    let log = python_hints(&["ruff"]);
    assert!(
        !log.contains("run :lsp-install"),
        "ruff serves python, so ty must not be hinted: {log}"
    );
}

// ── cargo installer (fake shim, no network/compile) ─────────────────────────────
//
// A real `cargo install` compiles a full crate graph (multi-minute,
// toolchain+network dependent), a poor fit even for a manual live-e2e gate;
// no live e2e exists for cargo- or npm-kind installs at all, so these tests
// are the entire coverage of HUME's side of the contract (argv, `--root`
// layout, receipt, registration, and the failure path) against a fake
// `cargo` executable that does no real work.

/// Layout of a real `cargo install --root <dir>`: `<dir>/bin/<bin_name>`.
fn cargo_layout(bin_name: &str) -> String {
    format!(
        "root=\"\"; prev=\"\"\n\
         for a in \"$@\"; do [ \"$prev\" = \"--root\" ] && root=\"$a\"; prev=\"$a\"; done\n\
         mkdir -p \"$root/bin\"\n\
         printf '#!/bin/sh\\n' > \"$root/bin/{bin_name}\"\n\
         chmod +x \"$root/bin/{bin_name}\""
    )
}

#[test]
fn lsp_install_cargo_runs_cargo_install_with_locked_root_and_registers() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("cargo", &args_file, &cargo_layout("pest-language-server"));

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install pest");

    let argv: Vec<String> = std::fs::read_to_string(&args_file)
        .expect("shim must have run and recorded argv")
        .lines()
        .map(str::to_string)
        .collect();
    let server_dir = server_dir(data_tmp.path(), "pest-language-server");
    assert_eq!(
        argv[..5],
        [
            "install",
            "--locked",
            "--root",
            server_dir.to_str().unwrap(),
            "--"
        ],
        "argv up to the crate spec must match exactly: {argv:?}"
    );
    assert!(
        argv[5].starts_with("pest-language-server@"),
        "final argv token must be the crate@version spec: {argv:?}"
    );
    let seeded_version = argv[5].strip_prefix("pest-language-server@").unwrap();

    let receipt = std::fs::read_to_string(server_dir.join("receipt.scm"))
        .expect("receipt.scm must be written on success");
    assert!(
        receipt.contains(&format!("(version . \"{seeded_version}\")")),
        "receipt version must match the version cargo was told to install: {receipt}"
    );

    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("pest-language-server")
        .expect("pest must be registered after a successful cargo install");
    assert_eq!(
        Path::new(&cmd),
        server_dir.join("bin").join("pest-language-server"),
        "the registered command must be the absolute bin/ path cargo --root produces"
    );
}

#[test]
fn lsp_install_cargo_missing_binary_after_install_fails_loudly() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    // The shim exits 0 but leaves no bin/ behind: the failure the installer's
    // own post-check must catch.
    let shim_dir = write_fake_tool_shim("cargo", &args_file, "");

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install pest");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after cargo install"),
        "a cargo install that produces no binary must fail loudly: {log}"
    );

    let server_dir = server_dir(data_tmp.path(), "pest-language-server");
    assert!(
        !server_dir.join("receipt.scm").exists(),
        "no receipt must be committed when the post-install binary check fails"
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("pest-language-server")
            .is_none(),
        "pest must not be registered when the install failed"
    );
}
