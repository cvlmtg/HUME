// Editor-level tests for core:lsp's server install pipeline (servers.scm):
// scan-on-load registration, :lsp-install/:lsp-uninstall/:lsp-servers,
// receipts, orphan warnings, and the on-language-set discovery hint.
//
// Fixture servers, chosen from the real runtime/scheme/lsp-{servers,sources}.scm
// catalogs (verified at authoring time, re-checked by these tests every run):
//   rust-analyzer (language "rust"): github, plain .gz, installable
//   svlangserver (language "systemverilog"): npm, settings contain a real
//     JSON array (systemverilog.includeIndexing)
//   gopls (language "go"): golang, installable through `go install`
//   ada-language-server (language "ada"): github, but every platform target
//     is .tar.gz, so it is never installable in v1 regardless of host OS
//   pest-language-server (language "pest"): cargo, crates.io semver,
//     installable
//   nil (language "nix"): cargo-git, a Mason git-tag pin, a stub: not
//     installable
//   ocamllsp (language "ocaml"): opam, a stub: not installable

use std::path::Path;

use super::*;
use crate::editor::Severity;

/// Write a receipt + a dummy binary file for `name` directly into
/// `<data_dir>/servers/<name>/`, matching what `lsp/install-server!` would
/// produce, for scan-time tests that don't need a real network install.
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

/// core:plum's own load must never error: a pure Scheme-syntax/logic smoke
/// test for `plugins.scm`/`grammars.scm` (no LSP catalogs touch this plugin
/// anymore; that's `lsp_plugin_loads_with_real_lsp_catalogs`'s job below).
#[test]
fn plum_plugin_loads_cleanly() {
    let _lock = lock();

    let data_tmp = safe_tempdir();
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
/// lsp-servers.scm catalog, and `servers.scm`'s lsp-sources.scm catalog load.
/// This is the smoke test for both self-contained module loads.
#[test]
fn lsp_plugin_loads_with_real_lsp_catalogs() {
    let _lock = lock();

    let data_tmp = safe_tempdir();
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
/// runs no receipt scan. LSP server install/uninstall/registration is
/// core:lsp-owned end to end.
#[test]
fn plum_alone_does_not_register_installed_servers() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
fn scan_registers_installed_server_with_absolute_managed_path() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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

/// The expected JSON is transcribed by hand from
/// runtime/scheme/lsp-servers.scm's current text, not derived by calling
/// `lsp/settings->hash`: this is the settings-conversion correctness
/// check, so it must not share logic with the thing it verifies.
#[test]
fn settings_conversion_produces_correct_json_shapes_for_arrays_and_nested_objects() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    fabricate_server(data_tmp.path(), "svlangserver", "0.4.1", "svlangserver");
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // The seeded catalog's `config` field is registered under BOTH keywords
    // (`registration.scm` delivers it as init-options and settings, exactly
    // as Helix delivers the same blob), so assert the conversion lands
    // correctly in both, not just one.

    // svlangserver: (systemverilog (includeIndexing . #("*.{v,vh,sv,svh}" "**/*.{v,vh,sv,svh}")))
    let expected_sv = serde_json::json!({
        "systemverilog": {
            "includeIndexing": ["*.{v,vh,sv,svh}", "**/*.{v,vh,sv,svh}"]
        }
    });
    let sv_settings = ed
        .lsp
        .config_settings_for_test("systemverilog")
        .expect("svlangserver settings must be registered");
    assert_eq!(
        sv_settings, expected_sv,
        "a #(...) settings array must decode to a JSON array, not an object or a string"
    );
    let sv_init_options = ed
        .lsp
        .config_init_options_for_test("systemverilog")
        .expect("svlangserver init-options must be registered");
    assert_eq!(
        sv_init_options, expected_sv,
        "catalog config must reach init-options with the same shape as settings"
    );

    // rust-analyzer: nested objects with bool/string/int scalar leaves.
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
    let ra_settings = ed
        .lsp
        .config_settings_for_test("rust")
        .expect("rust-analyzer settings must be registered");
    assert_eq!(
        ra_settings, expected_ra,
        "nested-object settings entries must round-trip through the converter exactly"
    );
    let ra_init_options = ed
        .lsp
        .config_init_options_for_test("rust")
        .expect("rust-analyzer init-options must be registered");
    assert_eq!(
        ra_init_options, expected_ra,
        "catalog config must reach init-options with the same shape as settings"
    );
}

#[test]
fn interrupted_install_is_warned_and_not_registered() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    std::fs::create_dir_all(&dir).unwrap();
    // No receipt.scm written: simulates an install that died mid-flight.
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
fn orphan_server_is_warned_and_not_registered() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
fn install_lock_sentinel_file_is_never_scanned_as_a_server_directory() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    // A file, not a directory, sitting directly under servers/, exactly
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
fn stray_non_directory_file_under_servers_dir_is_never_scanned_as_a_server() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    // A file, not a directory, with no special-cased name (e.g. a
    // Finder-dropped .DS_Store). Must be excluded on being a non-directory,
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
fn lsp_rescan_servers_command_registers_newly_installed() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
/// language the user registered by hand. Only languages nothing has
/// claimed yet get the catalog default. An unconditional re-registration
/// of every catalog language would silently replace a manual
/// `register-lsp-server!` override (documented workflow: a local build, a
/// version the catalog doesn't carry, or a `$PATH` copy the user wants to
/// take precedence; see user-manual/docs/lsp.md) on the next rescan.
#[test]
fn rescan_does_not_clobber_a_manually_registered_language() {
    let _lock = lock();
    let data_tmp = safe_tempdir();

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp\")\n\
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
/// eager `(load-plugin! "core:lsp")` queues a Register op for it from its own
/// startup scan, in the very same eval as anything that follows. The user's
/// own `register-lsp-server!` queued *after* that `load-plugin!` line must
/// win: `register-lsp-server!` is last-wins over queue order. Differs from
/// `rescan_does_not_clobber_a_manually_registered_language` above: there,
/// the receipt is fabricated *after* init.scm's eval, so `load-plugin!`'s own
/// scan queues nothing competing for "rust" in that eval, so it never
/// exercises this same-eval race at all.
#[test]
fn register_lsp_server_after_eager_load_plugin_overrides_the_scans_own_registration() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp\")\n\
         (register-lsp-server! \"rust\" #:command \"my-custom-rust-analyzer\" \
         #:root-markers '(\"Cargo.toml\"))",
    );

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("my-custom-rust-analyzer".to_owned()),
        "register-lsp-server! queued after load-plugin! must win over the scan's \
         own registration of the already-installed catalog server"
    );
}

/// Unlike the sibling test above, this queues the override *before* the
/// eager `load-plugin!` line. `lsp-registered-for-language?` reads through
/// the pending op queue, so the scan's no-clobber filter sees this
/// earlier-queued registration and skips "rust" entirely regardless of
/// call order.
#[test]
fn register_lsp_server_before_eager_load_plugin_also_survives_the_scan() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
        "(load-plugin! \"core:stdlib\")\n\
         (register-lsp-server! \"rust\" #:command \"my-custom-rust-analyzer\" \
         #:root-markers '(\"Cargo.toml\"))\n\
         (load-plugin! \"core:lsp\")",
    );

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("my-custom-rust-analyzer".to_owned()),
        "register-lsp-server! queued before load-plugin! must survive the scan's \
         no-clobber filter, which now reads through the same-eval pending queue"
    );
}

/// A lazily-declared core:lsp (`#:languages`) still registers an installed
/// server once activated (the startup scan runs at activation time, not
/// only at eager `(load-plugin! "core:lsp")`), and the very buffer whose
/// language-set triggered the activation attaches to that server in the
/// same call, with no need to wait for a later effects-applying drain.
///
/// `activate_lazy_language_plugins` (called from `set_buffer_language`,
/// before `lsp_attach_buffer`) evaluates the plugin inline via
/// `activate_and_register` (mappings/lazy.rs), which applies the activating
/// body's queued side effects (including any `register-lsp-server!`)
/// through `apply_script_effects` before returning.
///
/// The buffer is given a real path (`lsp_attach_buffer` no-ops on a pathless
/// buffer) so the attach assertions below actually exercise the attach path,
/// not just the registration.
#[test]
fn lazy_lsp_plugin_registers_installed_servers_on_language_activation() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    let src_tmp = safe_tempdir();
    let file = src_tmp.path().join("main.rs");
    std::fs::write(&file, b"fn main() {}\n").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(declare-plugin! \"core:lsp\" #:languages '(\"rust\"))",
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
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

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
/// command name is listed in the declaration's `#:commands` manifest:
/// dispatch runs `activate_lazy_plugin` before arity marshalling (see
/// input_stack/command.rs), so `:lsp-install` on a plugin that hasn't
/// loaded yet still works, no eager `(load-plugin! "core:lsp")` required.
#[test]
fn lazy_lsp_plugin_activates_on_typed_lsp_install_command() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (declare-plugin! \"core:lsp\" #:typed-commands '(\"lsp-install\"))",
    );

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("no language server is seeded"),
        "dispatching :lsp-install must activate the lazily-declared plugin \
         and then run normally: {msg}"
    );
}

// ── :lsp-install failure paths ────────────────────────────────────────────────

#[test]
fn lsp_install_stub_kind_names_the_unsupported_kind() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

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
fn lsp_install_unknown_language_warns() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.contains("no language server is seeded"),
        "an unseeded language must report, not silently no-op: {msg}"
    );
}

/// Tab on `:lsp-install`'s argument completes against the seeded catalog's
/// own languages, read from the real `runtime/scheme/lsp-servers.scm` (not
/// a fixture subset): "rus" matches only "rust" (unlike "ru", which also
/// matches "ruby").
#[test]
fn lsp_install_tab_completes_a_seeded_language() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    ed.handle_key(key(':'));
    type_chars(&mut ed, "lsp-install rus");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "lsp-install rust");
}

#[test]
fn lsp_install_no_language_buffer_and_no_arg_warns() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // ada-language-server ships only .tar.gz on every platform.
    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        type_cmd(&mut ed, ":lsp-install ada");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'tar' on $PATH"),
        "a tar-archive server must preflight tar before downloading: {log}"
    );
}

/// `lsp/with-install-lock!`'s failure branch (`thunk` raised) must still
/// release the lock: a failed install must not permanently wedge every
/// later `:lsp-install`/`:lsp-uninstall` behind a lock nothing will ever
/// release.
#[test]
fn install_lock_is_released_after_a_failed_install() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // Fails inside lsp/install-server! (lsp/install-blocker), i.e. inside
    // lsp/with-install-lock!'s thunk. Exercises the release-on-failure path,
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
    assert!(
        !lock_path.exists(),
        "a failed install must release the cross-process lock, not leave it \
         held forever: {}",
        lock_path.display()
    );

    // A second, unrelated install must be able to acquire the lock. Proves
    // release actually happened, not just that the sentinel file is
    // (coincidentally) absent.
    type_cmd(&mut ed, ":lsp-install ocaml");
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already in progress"),
        "a later install must not find the lock still held: {log}"
    );
}

/// A live `.install-lock` (as another HUME process mid-install would leave)
/// must refuse the install loudly, before any network activity, and never
/// interleave with a concurrent install/uninstall. `acquire-install-lock!`
/// fails first, so this never actually reaches rust-analyzer's real
/// download path.
#[test]
fn lsp_install_refuses_when_the_cross_process_lock_is_already_held() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
/// fallback branch, not the "no argument given" branch: a made-up language
/// name only this test's buffer has makes the distinction unambiguous.
#[test]
fn lsp_install_no_arg_falls_back_to_buffer_language_not_the_count_sentinel() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

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
/// load-time scan already ran (e.g. installed out-of-band) must be picked
/// up by `:lsp-install`'s own post-check rescan, not just reported as
/// up-to-date and left unregistered.
#[test]
fn lsp_install_up_to_date_registers_a_late_fabricated_receipt() {
    let _lock = lock();
    let data_tmp = safe_tempdir();

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());
    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        None,
        "precondition: nothing installed at load time, so core:lsp's load-time scan \
         registered nothing"
    );
    // Fabricate the receipt only now, after the load-time scan already ran
    // against an empty data dir, so the final assertion below can only pass
    // if :lsp-install's own up-to-date rescan registers it, not the
    // load-time scan.
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-13",
        "rust-analyzer",
    );

    type_cmd(&mut ed, ":lsp-install rust");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("LSP: rust-analyzer already installed (v2026-07-13) — up to date"),
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
/// must still exclude them: `:plum-install-plugins`/`:plum-list-plugins` must never treat a
/// bundled core plugin as something to `git clone`. `:plum-list-plugins`'s trailing
/// "PLUM missing:" status is the safe way to observe `plum/missing-plugins`'s
/// output without ever touching the network.
#[test]
fn plum_missing_plugins_excludes_declared_core_plugins() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:plum\")\n\
         (declare-plugin! \"core:lsp\" #:languages '(\"rust\"))",
    );

    type_cmd(&mut ed, ":plum-list-plugins");

    let status = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        status.starts_with("PLUM missing:"),
        "expected the trailing 'PLUM missing:' status line, got: {status}"
    );
    assert!(
        !status.contains("core:lsp"),
        "a bundled core plugin must never appear as 'missing': PLUM would try to \
         git-clone it: {status}"
    );
}

// ── :lsp-uninstall ────────────────────────────────────────────────────────────

#[test]
fn lsp_uninstall_removes_registration_and_directory() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
    ed.settle();

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
        "uninstall must remove the server directory once the deferred (after! 0 ...) fires"
    );
}

/// Tab on `:lsp-uninstall`'s argument completes against every server with
/// an install dir on disk: the same set the command itself accepts,
/// including an orphan no longer in the seeded catalog (uninstall's own
/// `(path-exists? dir)` check, not the catalog, decides what's removable).
#[test]
fn lsp_uninstall_tab_completes_an_installed_server() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    ed.handle_key(key(':'));
    type_chars(&mut ed, "lsp-uninstall rust-a");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "lsp-uninstall rust-analyzer");
}

/// The uninstall delete is guarded by the same cross-process lock: a live
/// `.install-lock` at the moment the deferred `(after! 0 ...)` callback fires
/// must refuse the delete loudly, leaving the directory intact.
#[test]
fn lsp_uninstall_refuses_the_delete_when_the_cross_process_lock_is_already_held() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
    ed.settle();

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
fn lsp_uninstall_of_never_installed_server_is_silent() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall rust-analyzer");
    ed.drain_async_sources();
    ed.settle();

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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    // A sibling write-sandbox dir `../plugins` would canonicalize into:
    // it must survive untouched.
    let plugins_dir = canonical_data_dir(data_tmp.path()).join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(plugins_dir.join("sentinel"), b"do not delete me").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall ../plugins");
    ed.drain_async_sources();
    ed.settle();

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

/// `stdlib/safe-path-segment?` (used by `lsp-uninstall`) must reject `:` and
/// `"`, not just `.`/`..`/path separators: the drive-relative-root escape
/// `hume_platform::path::is_safe_segment` exists to block.
#[test]
fn lsp_uninstall_rejects_colon_and_quote_in_name() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-uninstall c:evil");
    ed.drain_async_sources();
    ed.settle();
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("invalid server name") && log.contains("c:evil"),
        "must warn loudly about a drive-relative-root name: {log}"
    );

    type_cmd(&mut ed, ":lsp-uninstall a\"b");
    ed.drain_async_sources();
    ed.settle();
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("invalid server name") && log.contains("a\"b"),
        "must warn loudly about a quote-embedded name: {log}"
    );
}

// ── :lsp-servers ──────────────────────────────────────────────────────────────

#[test]
fn lsp_servers_command_runs_without_error() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
    // Severity routing). An empty command body would leave this `None`,
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
fn lsp_status_opens_a_read_only_view_when_no_servers_are_registered() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-status");

    assert_eq!(ed.doc().display_name(), "[lsp-status]");
    assert_eq!(ed.doc().text().to_string(), "No LSP servers registered.\n");
}

#[test]
fn lsp_stop_with_no_matching_server_reports_nothing_to_stop() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-restart");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("lsp: no matching server to restart")
    );
}

#[test]
fn plum_alone_does_not_expose_lsp_status_stop_restart() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
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
// `ed.set_buffer_language` + `ed.settle()` is not a `:`-typed command
// dispatch: it is the same path a buffer opened via a CLI argument at
// startup takes. These tests therefore also cover that the hook body's
// ctx-gated `lsp-registered-for-language?` call is safe outside a typed
// command's dispatch, not only after one.

#[test]
fn discovery_hint_fires_once_for_an_installable_unregistered_language() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let bid = ed.focused_buffer_id();
    // gopls (golang stub) is never installable, so the hint must never
    // suggest a command that would fail.
    let lang = ed.state.config.languages.intern("go");
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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // svlangserver (npm-kind, language "systemverilog") must not report
    // installable unconditionally. Reporting installable regardless of npm
    // availability could suggest a :lsp-install that immediately fails
    // `lsp/preflight!`'s own npm-on-$PATH check. Force $PATH to a directory
    // with no npm binary in it.
    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        let bid = ed.focused_buffer_id();
        let lang = ed.state.config.languages.intern("systemverilog");
        ed.set_buffer_language(bid, Some(lang));
        ed.settle();
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint an npm-kind install when npm is not on $PATH: {log}"
    );
}

#[test]
fn discovery_hint_fires_for_cargo_kind_now_installable() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // pest-language-server (cargo-kind, language "pest") must report
    // installable: core:lsp has a cargo installer, and cargo is
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
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    // Force $PATH to a directory with no cargo binary in it: must not hint
    // an install that would immediately fail `lsp/preflight!`'s cargo check.
    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        let bid = ed.focused_buffer_id();
        let lang = ed.state.config.languages.intern("pest");
        ed.set_buffer_language(bid, Some(lang));
        ed.settle();
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("run :lsp-install"),
        "must never hint a cargo-kind install when cargo is not on $PATH: {log}"
    );
}

#[test]
fn discovery_hint_does_not_fire_when_already_registered() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path()); // core:lsp's own scan registers rust-analyzer for "rust"

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

// ── cargo installer (fake shim, no network/compile) ─────────────────────────────
//
// A real `cargo install` compiles a full crate graph (multi-minute,
// toolchain+network dependent), a poor fit even for a manual live-e2e gate;
// no live e2e exists for cargo- or npm-kind installs at all, so these tests
// are the entire coverage of HUME's side of the contract (argv, `--root`
// layout, receipt, registration, and the failure path) against a fake
// `cargo` executable that does no real work.

/// Write an executable fake `tool` shim into a fresh tempdir and return that
/// dir. The shim records its argv (one token per line) to `args_file`, then
/// runs `layout`, a shell fragment that creates what the real tool would
/// leave behind. It resets its own `$PATH` first so `mkdir`/`chmod` can still
/// be found, even though the *test*'s `$PATH` is pinned to the shim directory
/// alone.
fn write_fake_tool_shim(tool: &str, args_file: &Path, layout: &str) -> tempfile::TempDir {
    let shim_dir = safe_tempdir();
    let body = format!(
        "#!/bin/sh\n\
         PATH=/usr/bin:/bin\n\
         printf '%s\\n' \"$@\" > {args_file}\n\
         {layout}\n",
        args_file = args_file.display(),
    );
    let shim_path = shim_dir.path().join(tool);
    std::fs::write(&shim_path, body).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim_dir
}

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

/// Layout of a real `go install` with `GOBIN` set: `$GOBIN/<bin_name>`.
fn go_layout(bin_name: &str) -> String {
    format!(
        "mkdir -p \"$GOBIN\"\n\
         printf '#!/bin/sh\\n' > \"$GOBIN/{bin_name}\"\n\
         chmod +x \"$GOBIN/{bin_name}\""
    )
}

#[test]
fn lsp_install_cargo_runs_cargo_install_with_locked_root_and_registers() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("cargo", &args_file, &cargo_layout("pest-language-server"));

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install pest");
    }

    let argv: Vec<String> = std::fs::read_to_string(&args_file)
        .expect("shim must have run and recorded argv")
        .lines()
        .map(str::to_string)
        .collect();
    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("pest-language-server");
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
        .lsp
        .config_command_for_test("pest")
        .expect("pest must be registered after a successful cargo install");
    assert_eq!(
        Path::new(&cmd),
        server_dir.join("bin").join("pest-language-server"),
        "the registered command must be the absolute bin/ path cargo --root produces"
    );
}

#[test]
fn lsp_install_cargo_missing_binary_after_install_fails_loudly() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    // The shim exits 0 but leaves no bin/ behind: the failure the installer's
    // own post-check must catch.
    let shim_dir = write_fake_tool_shim("cargo", &args_file, "");

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install pest");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after cargo install"),
        "a cargo install that produces no binary must fail loudly: {log}"
    );

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("pest-language-server");
    assert!(
        !server_dir.join("receipt.scm").exists(),
        "no receipt must be committed when the post-install binary check fails"
    );
    assert!(
        ed.lsp.config_command_for_test("pest").is_none(),
        "pest must not be registered when the install failed"
    );
}

// ── github-kind download pipeline (fabricated sources, fake curl) ───────────────
//
// The real catalog pins each asset's sha256, so a fabricated archive can't
// pass it. These tests run the shipped plugin sources against a runtime copy
// whose `scheme/lsp-sources.scm` names a fabricated asset with its true
// digest, and a `curl` shim that copies that asset into place.

/// Copy of the repo's `runtime/` whose `scheme/lsp-sources.scm` is `sources`.
fn runtime_with_sources(sources: &str) -> tempfile::TempDir {
    let runtime = safe_tempdir();
    let status = std::process::Command::new("cp")
        .arg("-R")
        .arg(format!("{}/.", repo_runtime_dir().display()))
        .arg(runtime.path())
        .status()
        .expect("spawn cp");
    assert!(status.success());
    std::fs::write(
        runtime.path().join("scheme").join("lsp-sources.scm"),
        sources,
    )
    .unwrap();
    runtime
}

fn sha256_hex(path: &Path) -> String {
    let out = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .expect("spawn shasum");
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// A github-kind source record serving `asset` (digest of `fixture`) for
/// every Unix install target, with `bin` as the binary's path in the server
/// directory.
fn github_source(name: &str, asset: &str, fixture: &Path, bin: &str) -> String {
    let sha = sha256_hex(fixture);
    let targets: String = ["darwin-arm64", "darwin-x64", "linux-x64"]
        .iter()
        .map(|t| format!("({t} \"{asset}\" \"sha256:{sha}\" \"{bin}\")"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "((\"{name}\" (kind . github) (version . \"9.9.9\") (repo . \"o/r\") (targets {targets})))"
    )
}

/// Fake `curl` that copies `fixture` to its `-o` argument and records its
/// argv to `args_file`. The real `tar`/`gzip`/`unzip` stay reachable through
/// the returned `$PATH` value.
fn write_fake_curl_shim(fixture: &Path, args_file: &Path) -> (tempfile::TempDir, String) {
    let shim_dir = safe_tempdir();
    let body = format!(
        "#!/bin/sh\n\
         PATH=/usr/bin:/bin\n\
         printf '%s\\n' \"$@\" > {args_file}\n\
         out=\"\"; prev=\"\"\n\
         for a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; done\n\
         cp {fixture} \"$out\"\n",
        args_file = args_file.display(),
        fixture = fixture.display(),
    );
    let shim_path = shim_dir.path().join("curl");
    std::fs::write(&shim_path, body).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:/usr/bin:/bin", shim_dir.path().display());
    (shim_dir, path)
}

#[test]
fn lsp_install_tar_gz_unpacks_a_nested_binary_and_registers() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = lock();
    let work = safe_tempdir();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/lua-language-server"), b"#!/bin/sh\n").unwrap();
    let archive = work.path().join("lls.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    let runtime = runtime_with_sources(&github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/lua-language-server",
    ));
    let args_file = work.path().join("curl-argv.txt");
    let (_shim, path) = write_fake_curl_shim(&archive, &args_file);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_with_init_in_runtime(
        &mut ed,
        runtime.path(),
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp\")",
    );
    {
        let _path = EnvVarGuard::set("PATH", &path);
        type_cmd(&mut ed, ":lsp-install lua");
    }

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("lua-language-server");
    let bin = server_dir.join("pkg/bin/lua-language-server");
    assert_eq!(
        std::fs::metadata(&bin)
            .expect("binary must be unpacked")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(
        !server_dir.join("lls.tar.gz").exists(),
        "the downloaded archive must be deleted after unpacking"
    );
    let cmd = ed
        .lsp
        .config_command_for_test("lua")
        .expect("lua must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

#[test]
fn lsp_install_raw_binary_is_marked_executable_and_kept() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = lock();
    let work = safe_tempdir();
    let asset = work.path().join("marksman-fake");
    std::fs::write(&asset, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&asset, std::fs::Permissions::from_mode(0o644)).unwrap();
    let runtime = runtime_with_sources(&github_source(
        "marksman",
        "marksman-fake",
        &asset,
        "marksman-fake",
    ));
    let args_file = work.path().join("curl-argv.txt");
    let (_shim, path) = write_fake_curl_shim(&asset, &args_file);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_with_init_in_runtime(
        &mut ed,
        runtime.path(),
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp\")",
    );
    {
        let _path = EnvVarGuard::set("PATH", &path);
        type_cmd(&mut ed, ":lsp-install markdown");
    }

    let bin = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("marksman")
        .join("marksman-fake");
    assert_eq!(
        std::fs::metadata(&bin)
            .expect("the raw download is the binary and must be kept")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let cmd = ed
        .lsp
        .config_command_for_test("markdown")
        .expect("markdown must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

#[test]
fn lsp_install_generic_kind_downloads_the_recorded_url_and_registers() {
    let _lock = lock();
    let work = safe_tempdir();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("terraform-ls"), b"#!/bin/sh\n").unwrap();
    let archive = work.path().join("tfls.zip");
    assert!(
        std::process::Command::new("zip")
            .arg("-j")
            .arg(&archive)
            .arg(payload.join("terraform-ls"))
            .status()
            .unwrap()
            .success()
    );
    let sha = sha256_hex(&archive);
    let url = "https://example.test/terraform-ls/9.9.9/tfls.zip";
    let targets: String = ["darwin-arm64", "darwin-x64", "linux-x64"]
        .iter()
        .map(|t| format!("({t} \"tfls.zip\" \"{url}\" \"sha256:{sha}\" \"terraform-ls\")"))
        .collect::<Vec<_>>()
        .join(" ");
    let runtime = runtime_with_sources(&format!(
        "((\"terraform-ls\" (kind . generic) (version . \"9.9.9\") (targets {targets})))"
    ));
    let args_file = work.path().join("curl-argv.txt");
    let (_shim, path) = write_fake_curl_shim(&archive, &args_file);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_with_init_in_runtime(
        &mut ed,
        runtime.path(),
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp\")",
    );
    {
        let _path = EnvVarGuard::set("PATH", &path);
        type_cmd(&mut ed, ":lsp-install hcl");
    }

    let argv = std::fs::read_to_string(&args_file).expect("curl shim must have run");
    assert_eq!(
        argv.lines().last(),
        Some(url),
        "curl must fetch the url recorded in the row verbatim: {argv}"
    );
    let bin = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("terraform-ls")
        .join("terraform-ls");
    let cmd = ed
        .lsp
        .config_command_for_test("hcl")
        .expect("hcl must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

/// Load `core:lsp` against a runtime whose catalog is `sources`, with a curl
/// shim serving `fixture`, and run `:lsp-install <lang>`. Returns the data dir
/// and the editor for post-install assertions.
fn install_from_fixture(
    sources: &str,
    fixture: &Path,
    lang: &str,
) -> (tempfile::TempDir, Editor, tempfile::TempDir) {
    let runtime = runtime_with_sources(sources);
    let args_file = fixture.with_extension("curl-argv");
    let (_shim, path) = write_fake_curl_shim(fixture, &args_file);
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_with_init_in_runtime(
        &mut ed,
        runtime.path(),
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp\")",
    );
    {
        let _path = EnvVarGuard::set("PATH", &path);
        type_cmd(&mut ed, &format!(":lsp-install {lang}"));
    }
    (data_tmp, ed, runtime)
}

#[test]
fn lsp_install_gz_asset_is_decoded_and_marked_executable() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = lock();
    let work = safe_tempdir();
    let plain = work.path().join("rust-analyzer-fake");
    std::fs::write(&plain, b"#!/bin/sh\necho decoded\n").unwrap();
    assert!(
        std::process::Command::new("gzip")
            .arg("-k")
            .arg(&plain)
            .status()
            .unwrap()
            .success()
    );
    let archive = work.path().join("rust-analyzer-fake.gz");
    let sources = github_source(
        "rust-analyzer",
        "rust-analyzer-fake.gz",
        &archive,
        "rust-analyzer",
    );

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "rust");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    let bin = server_dir.join("rust-analyzer");
    assert_eq!(std::fs::read(&bin).unwrap(), b"#!/bin/sh\necho decoded\n");
    assert_eq!(
        std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!server_dir.join("rust-analyzer-fake.gz").exists());
    let cmd = ed
        .lsp
        .config_command_for_test("rust")
        .expect("rust must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

#[test]
fn lsp_install_zip_marks_every_regular_file_executable_and_never_follows_symlinks() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = lock();
    let work = safe_tempdir();
    let outside = work.path().join("outside-target");
    std::fs::write(&outside, b"not part of the archive").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o644)).unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    for name in ["server", "helper"] {
        let f = payload.join("pkg/bin").join(name);
        std::fs::write(&f, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::os::unix::fs::symlink(&outside, payload.join("pkg/link")).unwrap();
    let archive = work.path().join("tfls.zip");
    assert!(
        std::process::Command::new("zip")
            .current_dir(&payload)
            .arg("-ry")
            .arg(&archive)
            .arg("pkg")
            .status()
            .unwrap()
            .success()
    );
    let sources = github_source("terraform-ls", "tfls.zip", &archive, "pkg/bin/server");

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "hcl");

    let pkg = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("terraform-ls")
        .join("pkg");
    for name in ["server", "helper"] {
        assert_eq!(
            std::fs::metadata(pkg.join("bin").join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "{name} must be made executable"
        );
    }
    assert!(
        std::fs::symlink_metadata(pkg.join("link"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the symlink entry must stay a symlink"
    );
    assert_eq!(
        std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
        0o644,
        "a symlink's target must never be chmod'd"
    );
    assert!(ed.lsp.config_command_for_test("hcl").is_some());
}

#[test]
fn lsp_install_missing_binary_after_unpack_fails_loudly() {
    let _lock = lock();
    let work = safe_tempdir();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("other"), b"x").unwrap();
    let archive = work.path().join("lls.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    let sources = github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "bin/lua-language-server",
    );

    let (data_tmp, mut ed, _runtime) = install_from_fixture(&sources, &archive, "lua");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary"),
        "an archive without the recorded binary must fail loudly: {log}"
    );
    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("lua-language-server");
    assert!(!server_dir.join("receipt.scm").exists());
    assert!(ed.lsp.config_command_for_test("lua").is_none());
}

#[test]
fn lsp_install_sha256_mismatch_deletes_the_archive_and_fails() {
    let _lock = lock();
    let work = safe_tempdir();
    let archive = work.path().join("marksman-fake");
    std::fs::write(&archive, b"served bytes").unwrap();
    let pinned = work.path().join("pinned-bytes");
    std::fs::write(&pinned, b"different bytes").unwrap();
    let sources = github_source("marksman", "marksman-fake", &pinned, "marksman-fake");

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "markdown");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("sha256 mismatch"),
        "a digest that differs from the pin must fail loudly: {log}"
    );
    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("marksman");
    assert!(!server_dir.join("marksman-fake").exists());
    assert!(!server_dir.join("receipt.scm").exists());
    assert!(ed.lsp.config_command_for_test("markdown").is_none());
}

fn set_lock_mtime(lock_path: &Path, offset_secs: i64) {
    let now = std::time::SystemTime::now();
    let delta = std::time::Duration::from_secs(offset_secs.unsigned_abs());
    let when = if offset_secs < 0 {
        now - delta
    } else {
        now + delta
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(lock_path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn lsp_install_replaces_a_lock_older_than_an_hour() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    let lock_path = servers_dir.join(".install-lock");
    std::fs::write(&lock_path, b"").unwrap();
    set_lock_mtime(&lock_path, -2 * 3600);

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());
    // ocamllsp fails inside the locked section, after the lock was acquired.
    type_cmd(&mut ed, ":lsp-install ocaml");

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already in progress"),
        "a stale lock must be replaced, not honoured: {log}"
    );
    assert!(log.contains("opam"), "the install must have run: {log}");
    assert!(
        !lock_path.exists(),
        "the replaced lock must be released afterwards"
    );
}

#[test]
fn lsp_install_treats_a_lock_dated_in_the_future_as_live() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    let lock_path = servers_dir.join(".install-lock");
    std::fs::write(&lock_path, b"").unwrap();
    set_lock_mtime(&lock_path, 2 * 3600);

    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":lsp-install ocaml");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("already in progress"),
        "a lock with a future mtime must count as live: {log}"
    );
    assert!(lock_path.exists(), "a live lock must be left in place");
}

// ── golang installer (fake shim) ────────────────────────────────────────────────

#[test]
fn lsp_install_golang_runs_go_install_into_gobin_and_registers() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("go", &args_file, &go_layout("gopls"));

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install go");
    }

    let argv: Vec<String> = std::fs::read_to_string(&args_file)
        .expect("shim must have run and recorded argv")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(argv[..2], ["install", "--"], "argv: {argv:?}");
    assert!(
        argv[2].starts_with("golang.org/x/tools/gopls@v"),
        "final argv token must be the module@version spec: {argv:?}"
    );

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("gopls");
    let cmd = ed
        .lsp
        .config_command_for_test("go")
        .expect("go must be registered after a successful go install");
    assert_eq!(
        Path::new(&cmd),
        server_dir.join("bin").join("gopls"),
        "GOBIN must point at the server dir's bin/"
    );
}

#[test]
fn lsp_install_golang_missing_binary_after_install_fails_loudly() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("go", &args_file, "");

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install go");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after go install"),
        "a go install that produces no binary must fail loudly: {log}"
    );
    assert!(
        ed.lsp.config_command_for_test("go").is_none(),
        "go must not be registered when the install failed"
    );
}

#[test]
fn lsp_install_golang_requires_go_on_path() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        type_cmd(&mut ed, ":lsp-install go");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'go' on $PATH"),
        "a golang-kind install must name the missing toolchain: {log}"
    );
}

// ── pypi installer (fake python3, fake venv python) ─────────────────────────────

/// Layout of `python3 -m venv <dir>`: a `<dir>/bin/python` that records its
/// own argv to `pip_args_file` and, when `create_binary`, leaves
/// `<dir>/bin/<bin_name>` behind like a real `pip install` would.
fn venv_layout(pip_args_file: &Path, bin_name: &str, create_binary: bool) -> String {
    let create = if create_binary {
        format!(
            "printf '#!/bin/sh\\n' > \"$(dirname \"$0\")/{bin_name}\"\n\
             chmod +x \"$(dirname \"$0\")/{bin_name}\"\n"
        )
    } else {
        String::new()
    };
    format!(
        "mkdir -p \"$3/bin\"\n\
         cat > \"$3/bin/python\" <<'EOS'\n\
         #!/bin/sh\n\
         PATH=/usr/bin:/bin\n\
         printf '%s\\n' \"$@\" > {pip_args_file}\n\
         {create}EOS\n\
         chmod +x \"$3/bin/python\"",
        pip_args_file = pip_args_file.display(),
    )
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("shim must have run and recorded argv")
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn lsp_install_pypi_creates_a_venv_pip_installs_and_registers() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", true));

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install python");
    }

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("ty");
    assert_eq!(
        read_lines(&venv_args),
        ["-m", "venv", server_dir.join("venv").to_str().unwrap()]
    );
    let pip = read_lines(&pip_args);
    assert_eq!(
        pip[..5],
        ["-m", "pip", "install", "--disable-pip-version-check", "--"],
        "pip argv: {pip:?}"
    );
    assert!(
        pip[5].starts_with("ty=="),
        "final token must be pkg==version: {pip:?}"
    );

    let cmd = ed
        .lsp
        .config_command_for_test("python")
        .expect("python must be registered after a successful pip install");
    assert_eq!(Path::new(&cmd), server_dir.join("venv/bin/ty"));
}

#[test]
fn lsp_install_pypi_passes_extras_in_the_requirement() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim(
        "python3",
        &venv_args,
        &venv_layout(&pip_args, "pylsp", true),
    );

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install snakemake");
    }

    let pip = read_lines(&pip_args);
    assert!(
        pip[5].starts_with("python-lsp-server[all]=="),
        "an `extra=all` purl qualifier must reach pip as [all]: {pip:?}"
    );
}

#[test]
fn lsp_install_pypi_missing_binary_after_install_fails_loudly() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir =
        write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", false));

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install python");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after pip install"),
        "a pip install that produces no binary must fail loudly: {log}"
    );
    assert!(ed.lsp.config_command_for_test("python").is_none());
}

#[test]
fn lsp_install_pypi_requires_python3_on_path() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        type_cmd(&mut ed, ":lsp-install python");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'python3' on $PATH"),
        "a pypi-kind install must name the missing interpreter: {log}"
    );
}

// ── per-platform restriction ────────────────────────────────────────────────────

fn load_lsp_with_sources(ed: &mut Editor, sources: &str, data_dir: &Path) -> tempfile::TempDir {
    let runtime = runtime_with_sources(sources);
    load_with_init_in_runtime(
        ed,
        runtime.path(),
        data_dir,
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp\")",
    );
    runtime
}

#[test]
fn lsp_install_refuses_a_server_restricted_to_other_platforms() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    let _runtime = load_lsp_with_sources(
        &mut ed,
        "((\"ty\" (kind . pypi) (version . \"1.0.0\") (package . \"ty\") (extras) (bin . \"ty\") \
          (platforms windows-x64)))",
        data_tmp.path(),
    );

    let args_tmp = safe_tempdir();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", true));
    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install python");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("not supported on this platform"),
        "a platform-restricted server must refuse loudly: {log}"
    );
    assert!(
        !venv_args.exists(),
        "the toolchain must not run for a refused install"
    );
}

#[test]
fn lsp_install_allows_a_server_whose_platform_list_includes_this_one() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    let _runtime = load_lsp_with_sources(
        &mut ed,
        "((\"ty\" (kind . pypi) (version . \"1.0.0\") (package . \"ty\") (extras) (bin . \"ty\") \
          (platforms darwin-arm64 darwin-x64 linux-x64)))",
        data_tmp.path(),
    );

    let args_tmp = safe_tempdir();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", true));
    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install python");
    }

    assert!(
        ed.lsp.config_command_for_test("python").is_some(),
        "an install on a listed platform must register"
    );
}

// ── gem installer (fake shim) ───────────────────────────────────────────────────

/// Layout of a real `gem install --bindir <dir>`: `<dir>/<bin_name>`.
fn gem_layout(bin_name: &str) -> String {
    format!(
        "bindir=\"\"; prev=\"\"\n\
         for a in \"$@\"; do [ \"$prev\" = \"--bindir\" ] && bindir=\"$a\"; prev=\"$a\"; done\n\
         mkdir -p \"$bindir\"\n\
         printf '#!/bin/sh\\n' > \"$bindir/{bin_name}\"\n\
         chmod +x \"$bindir/{bin_name}\""
    )
}

#[test]
fn lsp_install_gem_installs_into_the_server_dir_and_registers_gem_env() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("gem", &args_file, &gem_layout("ruby-lsp"));

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install ruby");
    }

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("ruby-lsp");
    let argv = read_lines(&args_file);
    assert_eq!(
        argv[..6],
        [
            "install",
            "--no-document",
            "--install-dir",
            server_dir.to_str().unwrap(),
            "--bindir",
            server_dir.join("bin").to_str().unwrap(),
        ],
        "argv: {argv:?}"
    );
    assert!(
        argv[6].starts_with("ruby-lsp:"),
        "the main package must be pinned as name:version: {argv:?}"
    );
    assert_eq!(argv[7], "ruby-lsp-rails", "extra packages follow: {argv:?}");

    let cmd = ed
        .lsp
        .config_command_for_test("ruby")
        .expect("ruby must be registered after a successful gem install");
    assert_eq!(Path::new(&cmd), server_dir.join("bin").join("ruby-lsp"));

    let env = ed
        .lsp
        .config_env_for_test("ruby")
        .expect("ruby is registered");
    for key in ["GEM_HOME", "GEM_PATH"] {
        let value = env
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("{key} must be registered for the server: {env:?}"));
        assert_eq!(
            Path::new(&value.1),
            server_dir,
            "{key} must be the server dir"
        );
    }
}

#[test]
fn lsp_install_gem_missing_binary_after_install_fails_loudly() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("gem", &args_file, "");

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install ruby");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after gem install"),
        "a gem install that produces no binary must fail loudly: {log}"
    );
    assert!(ed.lsp.config_command_for_test("ruby").is_none());
}

#[test]
fn lsp_install_gem_requires_gem_on_path() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        type_cmd(&mut ed, ":lsp-install ruby");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'gem' on $PATH"),
        "a gem-kind install must name the missing toolchain: {log}"
    );
}

// ── nuget installer (fake shim) ─────────────────────────────────────────────────

/// Layout of a real `dotnet tool install --tool-path <dir>`: `<dir>/<bin_name>`.
fn dotnet_layout(bin_name: &str) -> String {
    format!(
        "tp=\"\"; prev=\"\"\n\
         for a in \"$@\"; do [ \"$prev\" = \"--tool-path\" ] && tp=\"$a\"; prev=\"$a\"; done\n\
         mkdir -p \"$tp\"\n\
         printf '#!/bin/sh\\n' > \"$tp/{bin_name}\"\n\
         chmod +x \"$tp/{bin_name}\""
    )
}

#[test]
fn lsp_install_nuget_installs_a_dotnet_tool_and_registers() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim(
        "dotnet",
        &args_file,
        &dotnet_layout("roslyn-language-server"),
    );

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install c-sharp");
    }

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("roslyn-language-server");
    let argv = read_lines(&args_file);
    assert_eq!(
        argv[..5],
        [
            "tool",
            "install",
            "roslyn-language-server",
            "--tool-path",
            server_dir.join("bin").to_str().unwrap(),
        ],
        "argv: {argv:?}"
    );
    assert_eq!(argv[5], "--version", "argv: {argv:?}");
    assert!(
        !argv[6].is_empty(),
        "the seeded version must follow --version: {argv:?}"
    );

    let cmd = ed
        .lsp
        .config_command_for_test("c-sharp")
        .expect("c-sharp must be registered after a successful dotnet tool install");
    assert_eq!(
        Path::new(&cmd),
        server_dir.join("bin").join("roslyn-language-server")
    );
}

#[test]
fn lsp_install_nuget_missing_binary_after_install_fails_loudly() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let args_tmp = safe_tempdir();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("dotnet", &args_file, "");

    {
        let _path = EnvVarGuard::set("PATH", shim_dir.path());
        type_cmd(&mut ed, ":lsp-install c-sharp");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after dotnet install"),
        "a dotnet tool install that produces no binary must fail loudly: {log}"
    );
    assert!(ed.lsp.config_command_for_test("c-sharp").is_none());
}

#[test]
fn lsp_install_nuget_requires_dotnet_on_path() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    let empty_path_dir = safe_tempdir();
    {
        let _path = EnvVarGuard::set("PATH", empty_path_dir.path());
        type_cmd(&mut ed, ":lsp-install c-sharp");
    }

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'dotnet' on $PATH"),
        "a nuget-kind install must name the missing toolchain: {log}"
    );
}
