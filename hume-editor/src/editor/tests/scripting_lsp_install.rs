// Editor-level tests for LSP-INSTALL step 3 (PLUM `servers.scm`): scan-on-load
// registration, :lsp-install/:lsp-uninstall/:lsp-servers, receipts, orphan
// warnings, and the on-language-set discovery hint. See docs/LSP-INSTALL.md.
//
// Fixture servers, chosen from the real runtime/scheme/lsp-{servers,sources}.scm
// catalogs (verified at authoring time, re-checked by these tests every run):
//   rust-analyzer (language "rust") — github, plain .gz, installable
//   svlangserver (language "systemverilog") — npm, settings contain a real
//     JSON array (systemverilog.includeIndexing)
//   gopls (language "go") — golang purl kind, a stub source: not installable
//   ada-language-server (language "ada") — github, but every platform target
//     is .tar.gz, so it is never installable in v1 regardless of host OS

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::Severity;

/// Canonicalizes `root` (mirrors what `hume_scripting`'s `init_dirs` does
/// internally) and returns `<root>/hume` — the actual directory `(data-dir)`
/// resolves to. macOS temp dirs are symlinks (`/var/folders` ->
/// `/private/var/folders`); comparing against the raw tempdir path would
/// mismatch what a registered command's absolute path actually contains.
fn canonical_data_dir(root: &Path) -> PathBuf {
    root.canonicalize().unwrap().join("hume")
}

/// Write a receipt + a dummy binary file for `name` directly into
/// `<data_dir>/servers/<name>/`, matching what `plum/install-server!` would
/// produce — for scan-time tests that don't need a real network install.
fn fabricate_server(data_dir_root: &Path, name: &str, version: &str, bin: &str) {
    let dir = canonical_data_dir(data_dir_root).join("servers").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("receipt.scm"),
        format!(r#"((name . "{name}") (version . "{version}") (bin . "{bin}"))"#),
    )
    .unwrap();
    std::fs::write(dir.join(bin), b"#!/bin/sh\n").unwrap();
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    super::HUME_RUNTIME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Load `init_src` into `ed`, pointing `HUME_RUNTIME` at the repo's real
/// `runtime/` dir (so the real shipped plugin sources and
/// lsp-servers.scm/lsp-sources.scm catalogs are used) and `XDG_DATA_HOME` at
/// `data_dir`. Env vars are process-global — callers must hold
/// `super::HUME_RUNTIME_MUTEX` for the test's duration. Mirrors
/// `injections_editor.rs`'s `load_plum`.
#[cfg(not(windows))]
fn load_with_init(ed: &mut Editor, data_dir: &std::path::Path, init_src: &str) {
    let repo_runtime_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime");
    let config_tmp = tempfile::tempdir().unwrap();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_src).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", &repo_runtime_dir);
        std::env::set_var("XDG_DATA_HOME", data_dir);
    }
    ed.init_scripting();
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }
}

/// Load the real `core:plum` plugin only — the installer, no registration.
#[cfg(not(windows))]
fn load_plum(ed: &mut Editor, data_dir: &std::path::Path) {
    load_with_init(ed, data_dir, r#"(load-plugin "core:plum")"#);
}

/// Load the real `core:lsp` plugin only (plus its documented `core:stdlib`
/// dependency) — registration, no installer.
#[cfg(not(windows))]
fn load_lsp(ed: &mut Editor, data_dir: &std::path::Path) {
    load_with_init(
        ed,
        data_dir,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:lsp\")",
    );
}

/// Load both real plugins, `core:plum` then `core:lsp` — the ordering a
/// normal init.scm relying on installed servers would use.
#[cfg(not(windows))]
fn load_plum_and_lsp(ed: &mut Editor, data_dir: &std::path::Path) {
    load_with_init(
        ed,
        data_dir,
        "(load-plugin \"core:plum\")\n(load-plugin \"core:stdlib\")\n(load-plugin \"core:lsp\")",
    );
}

/// PLUM's catalog load (lsp-servers.scm/lsp-sources.scm) runs its real body
/// against the real catalogs with an empty data dir. This is a pure
/// Scheme-syntax/logic smoke test for `servers.scm`, not an installation
/// test — it must catch parse errors, unbound identifiers, and
/// context-gating mistakes (e.g. calling a command-only builtin at load
/// time) before any of that reaches a real install.
#[test]
#[cfg(not(windows))]
fn plum_plugin_loads_with_real_lsp_catalogs() {
    let _lock = lock();

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "loading core:plum against the real lsp-servers.scm/lsp-sources.scm catalogs must not error: {errors:?}"
    );
}

/// Twin of the above for `core:lsp`'s own catalog load (`registration.scm`),
/// which independently reads the seeded lsp-servers.scm catalog — this is
/// the smoke test for that self-contained module load.
#[test]
#[cfg(not(windows))]
fn lsp_plugin_loads_with_real_lsp_catalogs() {
    let _lock = lock();

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "loading core:lsp against the real lsp-servers.scm catalog must not error: {errors:?}"
    );
}

/// The regression test this whole change exists to pin: loading only
/// `core:plum` must never register or attach an installed server — PLUM is
/// installer-only now. See docs/LSP-INSTALL.md "Registration model".
#[test]
#[cfg(not(windows))]
fn plum_alone_does_not_register_installed_servers() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "loading core:plum alone must never register an installed server"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("interrupted install") && !log.contains("orphan server"),
        "core:plum must not run any receipt scan at all: {log}"
    );
}

// ── Scan-on-load ─────────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn scan_registers_installed_server_with_absolute_managed_path() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "scan must register rust-analyzer's command as the absolute managed path, \
         not a bare command name relying on $PATH lookup"
    );
}

/// Independent oracle: the expected JSON is transcribed by hand from
/// runtime/scheme/lsp-servers.scm's current text, not derived by calling
/// `plum/settings->hash` — this is the settings-conversion correctness
/// check, so it must not share logic with the thing it verifies.
#[test]
#[cfg(not(windows))]
fn settings_conversion_produces_correct_json_shapes_for_arrays_and_nested_objects() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "svlangserver", "0.4.1", "svlangserver");
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // svlangserver: (systemverilog (includeIndexing . #("*.{v,vh,sv,svh}" "**/*.{v,vh,sv,svh}")))
    let sv_settings = ed
        .lsp
        .config_settings_for_test("systemverilog")
        .expect("svlangserver settings must be registered");
    let expected_sv = serde_json::json!({
        "systemverilog": {
            "includeIndexing": ["*.{v,vh,sv,svh}", "**/*.{v,vh,sv,svh}"]
        }
    });
    assert_eq!(
        sv_settings, expected_sv,
        "a #(...) settings array must decode to a JSON array, not an object or a string"
    );

    // rust-analyzer: nested objects with bool/string/int scalar leaves.
    let ra_settings = ed
        .lsp
        .config_settings_for_test("rust")
        .expect("rust-analyzer settings must be registered");
    let expected_ra = serde_json::json!({
        "files": {"watcher": "server"},
        "inlayHints": {
            "bindingModeHints": {"enable": false},
            "closingBraceHints": {"minLines": 10},
            "closureReturnTypeHints": {"enable": "with_block"},
            "discriminantHints": {"enable": "fieldless"},
            "lifetimeElisionHints": {"enable": "skip_trivial"},
            "typeHints": {"hideClosureInitialization": false}
        }
    });
    assert_eq!(
        ra_settings, expected_ra,
        "nested-object settings entries must round-trip through the converter exactly"
    );
}

#[test]
#[cfg(not(windows))]
fn interrupted_install_is_warned_and_not_registered() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    std::fs::create_dir_all(&dir).unwrap();
    // No receipt.scm written — simulates an install that died mid-flight.
    std::fs::write(dir.join("rust-analyzer"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "a server dir without a readable receipt must never be registered"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("interrupted install") && log.contains("rust-analyzer"),
        "must warn about the interrupted install, naming the server: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn orphan_server_is_warned_and_not_registered() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "totally-not-a-real-server",
        "1.0.0",
        "totally-not-a-real-server",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("orphan")
            && log.contains("totally-not-a-real-server")
            && log.contains(":lsp-uninstall"),
        "must warn about the orphan server, naming it and suggesting :lsp-uninstall: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn install_lock_sentinel_file_is_never_scanned_as_a_server_directory() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    // A file, not a directory — sitting directly under servers/, exactly
    // where acquire-install-lock! puts it and register-installed-servers!
    // scans.
    std::fs::write(servers_dir.join(".install-lock"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains(".install-lock"),
        "the lock sentinel file must never be scanned as an interrupted/orphan server: {log}"
    );
}

/// Pins the notify path PLUM uses after a successful install: core:lsp
/// exposes `lsp-rescan-servers` precisely so PLUM can trigger a rescan
/// without requiring anything from core:lsp's module.
#[test]
#[cfg(not(windows))]
fn lsp_rescan_servers_command_registers_newly_installed() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "precondition: nothing installed yet"
    );

    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    type_cmd(&mut ed, ":lsp-rescan-servers");

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        ":lsp-rescan-servers must pick up a receipt written after the initial load-time scan"
    );
}

/// A mid-session rescan (`:lsp-rescan-servers`, or the notify PLUM sends
/// after `:lsp-install`) must never clobber a language the user registered
/// by hand — only languages nothing has claimed yet get the catalog
/// default. Before this test, the scan re-registered every catalog
/// language unconditionally, which would silently replace a manual
/// `register-lsp-server!` override (documented workflow: a local build, a
/// version PLUM doesn't carry, or a `$PATH` copy the user wants to take
/// precedence — see user-manual/docs/lsp.md) the next time anything
/// triggered a rescan.
#[test]
#[cfg(not(windows))]
fn rescan_does_not_clobber_a_manually_registered_language() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin \"core:stdlib\")\n\
         (load-plugin \"core:lsp\")\n\
         (register-lsp-server! \"rust\" #:command \"my-custom-rust-analyzer\" \
         #:root-markers '(\"Cargo.toml\"))",
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("my-custom-rust-analyzer".to_owned()),
        "precondition: the manual registration from init.scm took effect"
    );

    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    type_cmd(&mut ed, ":lsp-rescan-servers");

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("my-custom-rust-analyzer".to_owned()),
        "a rescan must not overwrite a language the user registered manually"
    );
}

/// A lazily-declared core:lsp (`#:languages`) still registers a
/// PLUM-installed server once activated — the startup scan runs at
/// activation time, not only at eager `(load-plugin "core:lsp")`.
///
/// `activate_lazy_language_plugins` (called from `set_buffer_language`,
/// before `lsp_attach_buffer`) evaluates the plugin inline but only flushes
/// *messages* (`activate_pending_plugins`, mappings/lazy.rs) — unlike the
/// eager `(load-plugin ...)` path, which flushes queued
/// `PendingLspServerOp`s at the end of init.scm
/// (`flush_pending_lsp_server_ops`, scripting_setup.rs). So the buffer whose
/// language-set *caused* the activation does not attach in that same call;
/// registration only becomes visible at the next effects-applying drain
/// (`apply_script_effects`, run by `drain_hooks` for a hook with a handler,
/// by `drain_pending_steel_calls`, or by the next command dispatch) — mirror
/// that explicitly here, matching the `flush_pending_lsp_server_ops`
/// test-only flush idiom other lsp_*.rs tests already use.
#[test]
#[cfg(not(windows))]
fn lazy_lsp_plugin_registers_installed_servers_on_language_activation() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin \"core:stdlib\")\n(declare-plugin \"core:lsp\" #:languages '(\"rust\"))",
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "precondition: core:lsp must not have activated yet"
    );

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".to_owned()));
    ed.drain_hooks();
    ed.apply_script_effects(hume_scripting::HookResult::default());

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "activating core:lsp via a language-set trigger must eventually run its startup scan"
    );
}

// ── :lsp-install failure paths ────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn lsp_install_stub_kind_names_the_unsupported_kind() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    // gopls's Mason source is purl kind `golang` — a stub, never installable in v1.
    type_cmd(&mut ed, ":lsp-install go");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("golang"),
        "a stub-kind server must fail naming the unsupported purl kind: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn lsp_install_unknown_language_warns() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("no language server is seeded"),
        "an unseeded language must warn, not silently no-op: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn lsp_install_no_language_buffer_and_no_arg_warns() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    // Fresh test buffer has no language set.
    type_cmd(&mut ed, ":lsp-install");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("no language given") || log.contains("no language set"),
        "no arg + no buffer language must warn: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn lsp_install_unsupported_asset_format_fails_loudly() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    // ada-language-server ships only .tar.gz on every platform — unsupported
    // in v1 (step 2 shipped plain-.gz and .zip unpacking only) regardless of
    // which host OS runs this test.
    type_cmd(&mut ed, ":lsp-install ada");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("unsupported asset format"),
        "a tar.gz-only server must fail naming the format, not silently skip: {log}"
    );
}

/// A live `.install-lock` (as another HUME process mid-install would leave)
/// must refuse the install loudly, before any network activity — never
/// interleave with a concurrent install/uninstall. `acquire-install-lock!`
/// fails first, so this never actually reaches rust-analyzer's real
/// download path (no `HUME_REQUIRE_LIVE_LSP_INSTALL_E2E` gate needed).
#[test]
#[cfg(not(windows))]
fn lsp_install_refuses_when_the_cross_process_lock_is_already_held() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    std::fs::write(servers_dir.join(".install-lock"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install rust");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("already in progress"),
        "a live cross-process lock must refuse the install loudly: {log}"
    );
}

/// Proves the minibuffer's `IntV(1)` no-arg sentinel (see
/// `command_mode.rs`'s arity marshalling) takes the buffer-language
/// fallback branch, not the "no argument given" branch — a made-up language
/// name only this test's buffer has makes the distinction unambiguous.
#[test]
#[cfg(not(windows))]
fn lsp_install_no_arg_falls_back_to_buffer_language_not_the_count_sentinel() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("definitely-not-seeded".to_owned()));

    type_cmd(&mut ed, ":lsp-install");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("definitely-not-seeded"),
        "no-arg :lsp-install must resolve to the buffer's language, not misread the \
         minibuffer's IntV(1) count sentinel as a string argument or as 'no language': {log}"
    );
}

// ── :lsp-install up-to-date path ──────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn lsp_install_up_to_date_hints_when_lsp_not_loaded() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    // Seeded version from runtime/scheme/lsp-sources.scm's rust-analyzer entry —
    // an exact match takes the "already installed" branch.
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install rust");

    // The up-to-date `'info` status is superseded by the `'warn` hint that
    // follows it in the same command — `'warn` sets both `message_log` and
    // `status_msg` (see message_log.rs's Severity table), so the hint is
    // what's left standing in `status_msg`, and it's the only thing that
    // must survive in `message_log` too (`'info` never reaches it).
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(
            "PLUM: server installed but core:lsp is not loaded — \
             add (load-plugin \"core:lsp\") to init.scm for LSP features"
        ),
        "must hint that core:lsp isn't loaded when it's the only thing missing"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("core:lsp"),
        "the hint must also be written to :messages, not just the status line: {log}"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "PLUM alone must never register, even on the up-to-date path"
    );
}

#[test]
#[cfg(not(windows))]
fn lsp_install_up_to_date_rescans_when_lsp_loaded() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum_and_lsp(&mut ed, data_tmp.path());
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "precondition: nothing installed at load time, so core:lsp's load-time scan \
         registered nothing"
    );
    // Fabricate the receipt only now, after the load-time scan already ran against
    // an empty data dir — so the final assertion below can only pass if the
    // up-to-date notify path (`plum/notify-lsp!` → `:lsp-rescan-servers`) itself
    // registers it, not the load-time scan.
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    type_cmd(&mut ed, ":lsp-install rust");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("PLUM: rust-analyzer already installed (v2026-07-06) — up to date"),
        "must report the up-to-date status"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("add (load-plugin \"core:lsp\")"),
        "must not hint to load core:lsp when it's already loaded: {log}"
    );
    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "registration must survive the up-to-date :lsp-install notify path"
    );
}

/// A lazily-declared (not yet activated) core:lsp — the manual's recommended
/// setup — must get the activation note from `plum/notify-lsp!`'s declared
/// branch, not the "not loaded" warn meant for a truly absent core:lsp. Before
/// this test, `declared-plugins` filtered out every `core:*` name, so
/// `plum/notify-lsp!` had no way to distinguish "declared but inactive" from
/// "never configured" and always fell through to the misleading warn.
#[test]
#[cfg(not(windows))]
fn lsp_install_up_to_date_notes_lazily_declared_lsp() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin \"core:plum\")\n\
         (load-plugin \"core:stdlib\")\n\
         (declare-plugin \"core:lsp\" #:languages '(\"rust\"))",
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "precondition: core:lsp is declared but not yet activated, so nothing is registered"
    );

    type_cmd(&mut ed, ":lsp-install rust");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(
            "PLUM: server installed — core:lsp will register it once it activates \
             (e.g. when a matching file opens)"
        ),
        "a declared-but-inactive core:lsp must get the activation note, not the load-plugin warn"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("add (load-plugin \"core:lsp\")"),
        "must not tell a correctly-configured user to change their config: {log}"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "must not register — core:lsp still hasn't activated"
    );
}

/// `declared-plugins` now includes `core:*` names (needed so
/// `plum/notify-lsp!` can tell "declared but inactive" apart from "never
/// configured"). PLUM's own install-list logic must still exclude them —
/// `:plum-install`/`:plum-list` must never treat a bundled core plugin as
/// something to `git clone`. `:plum-list`'s trailing "PLUM missing:" status
/// is the safe way to observe `plum/missing-plugins`'s output without ever
/// touching the network.
#[test]
#[cfg(not(windows))]
fn plum_missing_plugins_excludes_declared_core_plugins() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin \"core:plum\")\n\
         (load-plugin \"core:stdlib\")\n\
         (declare-plugin \"core:lsp\" #:languages '(\"rust\"))",
    );

    type_cmd(&mut ed, ":plum-list");

    let status = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        status.starts_with("PLUM missing:"),
        "expected the trailing 'PLUM missing:' status line, got: {status}"
    );
    assert!(
        !status.contains("core:lsp"),
        "a bundled core plugin must never appear as 'missing' — PLUM would try to \
         git-clone it: {status}"
    );
}

// ── :lsp-uninstall ────────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn lsp_uninstall_removes_registration_and_directory() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_plum_and_lsp(&mut ed, data_tmp.path());
    assert!(
        ed.lsp.config_command_for_test("rust").is_some(),
        "precondition: scan must have registered the fabricated install"
    );

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "uninstall must unregister every language the server served"
    );
    let dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    assert!(
        !dir.exists(),
        "uninstall must remove the server directory once the deferred (after 0 ...) fires"
    );
}

/// The uninstall delete is guarded by the same cross-process lock — a live
/// `.install-lock` at the moment the deferred `(after 0 ...)` callback fires
/// must refuse the delete loudly, leaving the directory intact.
#[test]
#[cfg(not(windows))]
fn lsp_uninstall_refuses_the_delete_when_the_cross_process_lock_is_already_held() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::write(servers_dir.join(".install-lock"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("already in progress"),
        "a live cross-process lock must refuse the delete loudly: {log}"
    );
    assert!(
        servers_dir.join("rust-analyzer").exists(),
        "the server directory must survive when the lock can't be acquired"
    );
}

#[test]
#[cfg(not(windows))]
fn lsp_uninstall_of_never_installed_server_is_silent() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "uninstalling an already-absent/never-installed server must succeed silently: {errors:?}"
    );
    // "nothing to uninstall" is `'info` — display-only (status line), never
    // written to `:messages` (see message_log.rs's Severity table) — so the
    // confirmation shows up in `status_msg`, not `message_log`.
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("PLUM: nothing to uninstall for rust-analyzer"),
        "must report there was nothing to do"
    );
}

#[test]
#[cfg(not(windows))]
fn lsp_uninstall_rejects_path_traversal_name() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    // A sibling write-sandbox dir `../plugins` would canonicalize into —
    // it must survive untouched.
    let plugins_dir = canonical_data_dir(data_tmp.path()).join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(plugins_dir.join("sentinel"), b"do not delete me").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall ../plugins");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert!(
        plugins_dir.join("sentinel").exists(),
        "path-traversal uninstall must never reach a sibling sandbox directory"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("invalid server name") && log.contains("../plugins"),
        "must warn loudly about the rejected name: {log}"
    );
}

// ── :lsp-servers ──────────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn lsp_servers_command_runs_without_error() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-servers");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(errors.is_empty(), ":lsp-servers must not error: {errors:?}");

    // The trailing `'info` summary lands in `status_msg` (see the
    // `lsp_uninstall_of_never_installed_server_is_silent` comment on
    // Severity routing) — an empty command body would leave this `None`,
    // so this pins that the catalog walk actually ran against real data.
    let status = ed
        .state
        .status_msg
        .as_deref()
        .expect("lsp-servers must report a seeded-server count");
    assert!(
        status.starts_with("PLUM: ") && status.ends_with(" seeded servers"),
        "unexpected status message: {status}"
    );
    let count: usize = status
        .trim_start_matches("PLUM: ")
        .trim_end_matches(" seeded servers")
        .parse()
        .unwrap_or_else(|_| panic!("status message count is not a number: {status}"));
    assert!(
        count > 0,
        "expected a non-zero seeded-server count: {status}"
    );
}

// ── Discovery hint ────────────────────────────────────────────────────────────
//
// `ed.set_buffer_language` + `ed.drain_hooks()` is not a `:`-typed command
// dispatch — it is the same path a buffer opened via a CLI argument at
// startup takes. These tests therefore also cover that the hook body's
// ctx-gated `lsp-registered-for-language?` call is safe outside a typed
// command's dispatch, not only after one.

#[test]
#[cfg(not(windows))]
fn discovery_hint_fires_once_for_an_installable_unregistered_language() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".to_owned()));
    ed.drain_hooks();

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

    // Revisit the same language later in the session — must not repeat.
    ed.set_buffer_language(bid, None);
    ed.drain_hooks();
    ed.set_buffer_language(bid, Some("rust".to_owned()));
    ed.drain_hooks();
    let log2 = ed.state.message_log.format_for_display();
    assert_eq!(
        log2.matches("run :lsp-install").count(),
        1,
        "the hint must not repeat for a language already evaluated this session: {log2}"
    );
}

#[test]
#[cfg(not(windows))]
fn discovery_hint_does_not_fire_for_a_blocked_server() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    // gopls (golang stub) is never installable — the hint must never
    // suggest a command that would fail.
    ed.set_buffer_language(bid, Some("go".to_owned()));
    ed.drain_hooks();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint a suggestion that would fail: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn discovery_hint_does_not_fire_for_npm_kind_when_npm_missing_from_path() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    // svlangserver (npm-kind, language "systemverilog") used to report
    // installable unconditionally — the hint could suggest a :lsp-install
    // that immediately fails `plum/preflight!`'s own npm-on-$PATH check.
    // Force $PATH to a directory with no npm binary in it.
    let empty_path_dir = tempfile::tempdir().unwrap();
    let original_path = std::env::var("PATH").unwrap();
    unsafe {
        std::env::set_var("PATH", empty_path_dir.path());
    }

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("systemverilog".to_owned()));
    ed.drain_hooks();

    unsafe {
        std::env::set_var("PATH", original_path);
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint an npm-kind install when npm is not on $PATH: {log}"
    );
}

#[test]
#[cfg(not(windows))]
fn discovery_hint_does_not_fire_when_already_registered() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_plum_and_lsp(&mut ed, data_tmp.path()); // core:lsp's scan registers rust-analyzer for "rust"

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".to_owned()));
    ed.drain_hooks();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must not hint when the language is already registered: {log}"
    );
}

/// D6: an installed-but-unregistered server (PLUM loaded, core:lsp not)
/// gets a different hint than an uninstalled one — running `:lsp-install`
/// again would be a no-op, so the hint must point at loading core:lsp
/// instead.
#[test]
#[cfg(not(windows))]
fn discovery_hint_suggests_loading_lsp_for_installed_server() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".to_owned()));
    ed.drain_hooks();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must not suggest re-running :lsp-install for an already-installed server: {log}"
    );
    assert!(
        log.contains("load-plugin \"core:lsp\"") && log.contains("rust-analyzer"),
        "must hint to load core:lsp, naming the installed server: {log}"
    );
}

// ── Live e2e (env-gated) ───────────────────────────────────────────────────────

/// End-to-end: real `:lsp-install rust` (rust-analyzer, github, plain .gz) —
/// download, sha256 verification, unpack, receipt, and registration all
/// exercised for real against the live GitHub release. Loads both plugins
/// (PLUM installs, core:lsp registers) — the realistic setup for someone
/// who actually wants the server to work, not just download.
///
/// Gated by `HUME_REQUIRE_LIVE_LSP_INSTALL_E2E=1`; skipped otherwise.
#[test]
#[cfg(not(windows))]
fn lsp_install_real_rust_analyzer_e2e() {
    let require_live = std::env::var("HUME_REQUIRE_LIVE_LSP_INSTALL_E2E")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !require_live {
        eprintln!(
            "lsp_install_real_rust_analyzer_e2e: skipping \
             (set HUME_REQUIRE_LIVE_LSP_INSTALL_E2E=1 to run live e2e)"
        );
        return;
    }

    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum_and_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install rust");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "live rust-analyzer install must not error: {errors:?}"
    );

    let dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    let receipt =
        std::fs::read_to_string(dir.join("receipt.scm")).expect("receipt.scm must be written");
    assert!(
        receipt.contains("rust-analyzer"),
        "receipt must name the server: {receipt}"
    );

    let cmd = ed
        .lsp
        .config_command_for_test("rust")
        .expect("rust must be registered after a successful install");
    let cmd_path = Path::new(&cmd);
    assert!(
        cmd_path.exists(),
        "the registered command must point at a real, existing binary: {cmd}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(cmd_path).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "the installed binary must be executable: {cmd}"
        );
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("add (load-plugin \"core:lsp\")"),
        "must not hint to load core:lsp when it's already loaded: {log}"
    );
}
