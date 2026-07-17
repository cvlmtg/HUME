// Editor-level tests for core:lsp's server install pipeline (servers.scm):
// scan-on-load registration, :lsp-install/:lsp-uninstall/:lsp-servers,
// receipts, orphan warnings, and the on-language-set discovery hint. See
// docs/LSP-INSTALL.md.
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

/// Canonicalizes `root` (mirrors what `hume_scripting`'s `ScriptDirs::new`
/// does internally) and returns `<root>/hume` — the actual directory `(data-dir)`
/// resolves to. macOS temp dirs are symlinks (`/var/folders` ->
/// `/private/var/folders`); comparing against the raw tempdir path would
/// mismatch what a registered command's absolute path actually contains.
fn canonical_data_dir(root: &Path) -> PathBuf {
    root.canonicalize().unwrap().join("hume")
}

/// Write a receipt + a dummy binary file for `name` directly into
/// `<data_dir>/servers/<name>/`, matching what `lsp/install-server!` would
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

/// Load the real `core:plum` plugin only — plugin/grammar management, no
/// LSP awareness at all (servers.scm lives entirely in core:lsp now).
#[cfg(not(windows))]
fn load_plum(ed: &mut Editor, data_dir: &std::path::Path) {
    load_with_init(ed, data_dir, r#"(load-plugin "core:plum")"#);
}

/// Load the real `core:lsp` plugin only (plus its documented `core:stdlib`
/// dependency) — the entire LSP server lifecycle: install, uninstall,
/// listing, and scan-on-load registration.
#[cfg(not(windows))]
fn load_lsp(ed: &mut Editor, data_dir: &std::path::Path) {
    load_with_init(
        ed,
        data_dir,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:lsp\")",
    );
}

/// core:plum's own load must never error — a pure Scheme-syntax/logic smoke
/// test for `plugins.scm`/`grammars.scm` (no LSP catalogs touch this plugin
/// anymore; that's `lsp_plugin_loads_with_real_lsp_catalogs`'s job below).
#[test]
#[cfg(not(windows))]
fn plum_plugin_loads_cleanly() {
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
        "loading core:plum (plugins + grammars only) must not error: {errors:?}"
    );
}

/// core:lsp's own catalog load (`registration.scm`), which reads the seeded
/// lsp-servers.scm catalog, and `servers.scm`'s lsp-sources.scm catalog load
/// — this is the smoke test for both self-contained module loads.
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
        "loading core:lsp against the real lsp-servers.scm/lsp-sources.scm catalogs must not error: {errors:?}"
    );
}

/// The regression test this whole change exists to pin: loading only
/// `core:plum` exposes no LSP commands at all (not even `:lsp-install`) and
/// runs no receipt scan — LSP server install/uninstall/registration is
/// core:lsp-owned end to end. See docs/LSP-INSTALL.md "Registration model".
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

    type_cmd(&mut ed, ":lsp-install rust");
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("Unknown command: lsp-install"),
        "core:plum alone must not expose :lsp-install: {log}"
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
/// `lsp/settings->hash` — this is the settings-conversion correctness
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

#[test]
#[cfg(not(windows))]
fn stray_non_directory_file_under_servers_dir_is_never_scanned_as_a_server() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    // A file, not a directory, with no special-cased name — e.g. a
    // Finder-dropped .DS_Store. Must be excluded on being a non-directory,
    // not on matching a specific filename.
    std::fs::write(servers_dir.join(".DS_Store"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains(".DS_Store") && !log.contains("interrupted install"),
        "a stray non-directory file must never be scanned as an interrupted/orphan server: {log}"
    );
}

/// Exposed for out-of-band installs (a server installed outside
/// `:lsp-install`), and used internally by `servers.scm`'s own install and
/// uninstall commands to pick up what they just wrote to disk.
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

/// A mid-session rescan (`:lsp-rescan-servers`, or the one `:lsp-install`
/// runs after a successful/up-to-date install) must never clobber a
/// language the user registered by hand — only languages nothing has
/// claimed yet get the catalog default. An unconditional re-registration
/// of every catalog language would silently replace a manual
/// `register-lsp-server!` override (documented workflow: a local build, a
/// version the catalog doesn't carry, or a `$PATH` copy the user wants to
/// take precedence — see user-manual/docs/lsp.md) on the next rescan.
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

/// When a seeded server is *already installed before init.scm even runs*, an
/// eager `(load-plugin "core:lsp")` queues a Register op for it from its own
/// startup scan, in the very same eval as anything that follows. The user's
/// own `register-lsp-server!` queued *after* that `load-plugin` line must
/// win — `register-lsp-server!` is last-wins over queue order. Differs from
/// `rescan_does_not_clobber_a_manually_registered_language` above: there,
/// the receipt is fabricated *after* init.scm's eval, so `load-plugin`'s own
/// scan queues nothing competing for "rust" in that eval — it never
/// exercises this same-eval race at all.
#[test]
#[cfg(not(windows))]
fn register_lsp_server_after_eager_load_plugin_overrides_the_scans_own_registration() {
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
        "(load-plugin \"core:stdlib\")\n\
         (load-plugin \"core:lsp\")\n\
         (register-lsp-server! \"rust\" #:command \"my-custom-rust-analyzer\" \
         #:root-markers '(\"Cargo.toml\"))",
    );

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("my-custom-rust-analyzer".to_owned()),
        "register-lsp-server! queued after load-plugin must win over the scan's \
         own registration of the already-installed catalog server"
    );
}

/// Unlike the sibling test above, this queues the override *before* the
/// eager `load-plugin` line. `lsp-registered-for-language?` reads through
/// the pending op queue, so the scan's no-clobber filter sees this
/// earlier-queued registration and skips "rust" entirely regardless of
/// call order.
#[test]
#[cfg(not(windows))]
fn register_lsp_server_before_eager_load_plugin_also_survives_the_scan() {
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
        "(load-plugin \"core:stdlib\")\n\
         (register-lsp-server! \"rust\" #:command \"my-custom-rust-analyzer\" \
         #:root-markers '(\"Cargo.toml\"))\n\
         (load-plugin \"core:lsp\")",
    );

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("my-custom-rust-analyzer".to_owned()),
        "register-lsp-server! queued before load-plugin must survive the scan's \
         no-clobber filter, which now reads through the same-eval pending queue"
    );
}

/// A lazily-declared core:lsp (`#:languages`) still registers an installed
/// server once activated — the startup scan runs at activation time, not
/// only at eager `(load-plugin "core:lsp")` — and the very buffer whose
/// language-set triggered the activation attaches to that server in the
/// same call, with no need to wait for a later effects-applying drain.
///
/// `activate_lazy_language_plugins` (called from `set_buffer_language`,
/// before `lsp_attach_buffer`) evaluates the plugin inline via
/// `activate_and_register` (mappings/lazy.rs), which applies the activating
/// body's queued side effects — including any `register-lsp-server!` —
/// through `apply_script_effects` before returning.
///
/// The buffer is given a real path (`lsp_attach_buffer` no-ops on a pathless
/// buffer) so the attach assertions below actually exercise the attach path,
/// not just the registration.
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
    let src_tmp = tempfile::tempdir().unwrap();
    let file = src_tmp.path().join("main.rs");
    std::fs::write(&file, b"fn main() {}\n").unwrap();

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
    // Set the path *after* init_scripting (which re-detects language for
    // every already-open buffer and would otherwise activate core:lsp early,
    // before this test's own explicit `set_buffer_language` call below).
    ed.doc_mut().set_path(Some(file));

    let bid = ed.focused_buffer_id();
    assert!(
        ed.state.buffers.get(bid).lsp_server.is_none(),
        "precondition: buffer must be unattached before core:lsp activates"
    );
    ed.set_buffer_language(bid, Some("rust".to_owned()));

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "activating core:lsp via a language-set trigger must apply its startup scan \
         immediately, in the same set_buffer_language call"
    );
    assert!(
        ed.state.buffers.get(bid).lsp_server.is_some(),
        "the buffer whose language-set triggered activation must attach in that same \
         call, not wait for a later effects-applying drain"
    );
    assert_eq!(ed.lsp.server_count_for_test(), 1);
}

/// A `:`-typed command can activate a lazily-declared core:lsp when the
/// command name is listed in the declaration's `#:commands` manifest —
/// dispatch runs `activate_lazy_plugin` before arity marshalling (see
/// mappings/command_mode.rs), so `:lsp-install` on a plugin that hasn't
/// loaded yet still works, no eager `(load-plugin "core:lsp")` required.
#[test]
#[cfg(not(windows))]
fn lazy_lsp_plugin_activates_on_typed_lsp_install_command() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin \"core:stdlib\")\n\
         (declare-plugin \"core:lsp\" #:commands '(\"lsp-install\"))",
    );

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("no language server is seeded"),
        "dispatching :lsp-install must activate the lazily-declared plugin \
         and then run normally: {log}"
    );
}

// ── :lsp-install failure paths ────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn lsp_install_stub_kind_names_the_unsupported_kind() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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

/// `lsp/with-install-lock!`'s failure branch (`thunk` raised) must still
/// release the lock — a failed install must not permanently wedge every
/// later `:lsp-install`/`:lsp-uninstall` behind a lock nothing will ever
/// release.
#[test]
#[cfg(not(windows))]
fn install_lock_is_released_after_a_failed_install() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // Fails inside lsp/install-server! (lsp/install-blocker), i.e. inside
    // lsp/with-install-lock!'s thunk — exercises the release-on-failure path,
    // not the release-on-success path every other install test hits.
    type_cmd(&mut ed, ":lsp-install ada");
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("unsupported asset format"),
        "sanity: the install must actually have failed: {log}"
    );

    let lock_path = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join(".install-lock");
    assert!(
        !lock_path.exists(),
        "a failed install must release the cross-process lock, not leave it \
         held forever: {}",
        lock_path.display()
    );

    // A second, unrelated install must be able to acquire the lock — proves
    // release actually happened, not just that the sentinel file is
    // (coincidentally) absent.
    type_cmd(&mut ed, ":lsp-install ada");
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already in progress"),
        "a later install must not find the lock still held: {log}"
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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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

/// The up-to-date path still re-registers: a receipt written after the
/// load-time scan already ran (e.g. installed out-of-band) must be picked
/// up by `:lsp-install`'s own post-check rescan, not just reported as
/// up-to-date and left unregistered.
#[test]
#[cfg(not(windows))]
fn lsp_install_up_to_date_registers_a_late_fabricated_receipt() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "precondition: nothing installed at load time, so core:lsp's load-time scan \
         registered nothing"
    );
    // Fabricate the receipt only now, after the load-time scan already ran
    // against an empty data dir — so the final assertion below can only pass
    // if :lsp-install's own up-to-date rescan registers it, not the
    // load-time scan.
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    type_cmd(&mut ed, ":lsp-install rust");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("LSP: rust-analyzer already installed (v2026-07-06) — up to date"),
        "must report the up-to-date status"
    );
    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "registration must survive the up-to-date :lsp-install rescan"
    );
}

/// `declared-plugins` includes `core:*` names. PLUM's own install-list logic
/// must still exclude them — `:plum-install`/`:plum-list` must never treat a
/// bundled core plugin as something to `git clone`. `:plum-list`'s trailing
/// "PLUM missing:" status is the safe way to observe `plum/missing-plugins`'s
/// output without ever touching the network.
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
    load_lsp(&mut ed, data_tmp.path());
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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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
        Some("LSP: nothing to uninstall for rust-analyzer"),
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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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
        status.starts_with("LSP: ") && status.ends_with(" seeded servers"),
        "unexpected status message: {status}"
    );
    let count: usize = status
        .trim_start_matches("LSP: ")
        .trim_end_matches(" seeded servers")
        .parse()
        .unwrap_or_else(|_| panic!("status message count is not a number: {status}"));
    assert!(
        count > 0,
        "expected a non-zero seeded-server count: {status}"
    );
}

// ── :lsp-status / :lsp-stop / :lsp-restart ─────────────────────────────────────
//
// These live in core:lsp, dispatched through Steel to the
// `lsp-show-status!`/`lsp-stop!`/`lsp-restart!` builtins.

#[test]
#[cfg(not(windows))]
fn lsp_status_opens_a_read_only_view_when_no_servers_are_registered() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-status");

    assert_eq!(ed.doc().display_name(), "[lsp-status]");
    assert_eq!(ed.doc().text().to_string(), "No LSP servers registered.\n");
}

#[test]
#[cfg(not(windows))]
fn lsp_stop_with_no_matching_server_reports_nothing_to_stop() {
    let _lock = lock();
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
#[cfg(not(windows))]
fn lsp_restart_with_no_matching_server_reports_nothing_to_restart() {
    let _lock = lock();
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
#[cfg(not(windows))]
fn plum_alone_does_not_expose_lsp_status_stop_restart() {
    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    for cmd in [":lsp-status", ":lsp-stop", ":lsp-restart"] {
        type_cmd(&mut ed, cmd);
        let log = ed.state.message_log.format_for_display();
        assert!(
            log.contains(&format!("Unknown command: {}", &cmd[1..])),
            "core:plum alone must not expose {cmd}: {log}"
        );
    }
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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

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
    load_lsp(&mut ed, data_tmp.path());

    // svlangserver (npm-kind, language "systemverilog") must not report
    // installable unconditionally — reporting installable regardless of npm
    // availability could suggest a :lsp-install that immediately fails
    // `lsp/preflight!`'s own npm-on-$PATH check. Force $PATH to a directory
    // with no npm binary in it.
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
    load_lsp(&mut ed, data_tmp.path()); // core:lsp's own scan registers rust-analyzer for "rust"

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".to_owned()));
    ed.drain_hooks();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must not hint when the language is already registered: {log}"
    );
}

// ── Live e2e (env-gated) ───────────────────────────────────────────────────────

/// End-to-end: real `:lsp-install rust` (rust-analyzer, github, plain .gz) —
/// download, sha256 verification, unpack, receipt, and registration all
/// exercised for real against the live GitHub release.
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
    load_lsp(&mut ed, data_tmp.path());

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
}

/// Regression for the reinstall/upgrade path: `lsp/install-server!` queues
/// `unregister-lsp-server!` for every one of the server's languages before
/// reinstalling, and that op applies only at end-of-eval. The post-install
/// rescan (same eval) filters on `lsp-registered-for-language?`, which reads
/// through the pending op queue — so it sees that queued unregister and
/// correctly re-admits the language for re-registration, rather than
/// skipping it because the live (pre-drain) registry still shows it
/// registered. This drives that exact path for real (a receipt whose
/// version no longer matches the seeded catalog) and asserts `rust` is
/// still registered afterward.
///
/// Gated by `HUME_REQUIRE_LIVE_LSP_INSTALL_E2E=1`; skipped otherwise.
#[test]
#[cfg(not(windows))]
fn lsp_install_real_rust_analyzer_reinstall_after_version_bump_e2e() {
    let require_live = std::env::var("HUME_REQUIRE_LIVE_LSP_INSTALL_E2E")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !require_live {
        eprintln!(
            "lsp_install_real_rust_analyzer_reinstall_after_version_bump_e2e: skipping \
             (set HUME_REQUIRE_LIVE_LSP_INSTALL_E2E=1 to run live e2e)"
        );
        return;
    }

    let _lock = lock();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install rust");
    assert!(
        ed.lsp.config_command_for_test("rust").is_some(),
        "rust must be registered after the first install"
    );

    // Force the mismatch branch on the next :lsp-install: rewrite the
    // receipt's version so it no longer matches the seeded catalog version,
    // without touching the already-downloaded binary or its path.
    let receipt_path = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("receipt.scm");
    let receipt = std::fs::read_to_string(&receipt_path).unwrap();
    let downgraded = receipt.replacen("(version . \"", "(version . \"0000-00-00-superseded-", 1);
    std::fs::write(&receipt_path, downgraded).unwrap();

    type_cmd(&mut ed, ":lsp-install rust");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("installing rust-analyzer"),
        "the version mismatch must take the reinstall path, not \"up to date\": {log}"
    );
    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "reinstall-after-version-bump must not error: {errors:?}"
    );
    assert!(
        ed.lsp.config_command_for_test("rust").is_some(),
        "rust must still be registered after a version-mismatch reinstall — \
         this is exactly what the rescan's same-eval read-through guarantees"
    );
}
