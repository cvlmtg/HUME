//! Loading the lsp-install plugin, and scanning installed servers when it loads.

use super::*;

/// core:plum's own load must never error: a pure Scheme-syntax/logic smoke
/// test for `plugins.scm`/`grammars.scm` (no LSP catalogs touch this plugin
/// anymore; that's `lsp_plugin_loads_with_real_lsp_catalogs`'s job below).
#[test]
fn plum_plugin_loads_cleanly() {
    let _path = path_reader();

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "loading core:plum (plugins + grammars only) must not error: {errors:?}"
    );
}

/// core:lsp-install's catalog load, which reads its own `data/` files: the
/// smoke test for both self-contained module loads.
#[test]
fn lsp_plugin_loads_with_real_lsp_catalogs() {
    let _path = path_reader();

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "loading core:lsp-install against its real data files must not error: {errors:?}"
    );
}

/// The regression test this whole change exists to pin: loading only
/// `core:plum` exposes no LSP commands at all (not even `:lsp-install`) and
/// runs no receipt scan. LSP server install/uninstall/registration is
/// core:lsp-owned end to end.
#[test]
fn plum_alone_does_not_register_installed_servers() {
    let _path = path_reader();
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
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
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
        log.contains("unknown command: lsp-install"),
        "core:plum alone must not expose :lsp-install: {log}"
    );
}

// ── Scan-on-load ─────────────────────────────────────────────────────────────

#[test]
fn scan_registers_installed_server_with_absolute_managed_path() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install_eager(&mut ed, data_tmp.path());

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "scan must register rust-analyzer's command as the absolute managed path, \
         not a bare command name relying on $PATH lookup"
    );
}

/// Registering installed servers needs only `registrations/`: a server's
/// `requirements/` file is read when an install or the discovery hint asks
/// for it, and its `sources/` file only by an install, so unreadable ones do
/// not stop the scan.
#[test]
fn scan_does_not_read_the_install_requirements() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "gopls", "0.20.0", "gopls");
    let runtime = runtime_with_sources(&[]);
    let data = runtime.path().join("plugins/core/lsp-install/data");
    std::fs::write(data.join("sources/gopls.scm"), "(").unwrap();
    std::fs::write(data.join("requirements/gopls.scm"), "(").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_with_init_in_runtime(
        &mut ed,
        runtime.path(),
        data_tmp.path(),
        &format!(
            "(load-plugin! \"core:stdlib\")\n{}",
            hume_scripting::eager_load_scm("core:lsp-install", None)
        ),
    );

    assert!(
        ed.state.lsp.registered_command_for_test("gopls").is_some(),
        "the scan must register an installed server without reading its sources or requirements"
    );
}

/// The expected JSON is transcribed by hand from
/// runtime/plugins/core/lsp-install/data/registrations/'s current text, not derived by calling
/// `lsp/settings->hash`: this is the settings-conversion correctness
/// check, so it must not share logic with the thing it verifies.
#[test]
fn settings_conversion_produces_correct_json_shapes_for_arrays_and_nested_objects() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "svlangserver", "0.4.1", "svlangserver");
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install_eager(&mut ed, data_tmp.path());

    // The seeded catalog's `config` field is registered under BOTH keywords
    // (`core:lsp-install`'s `register.scm` delivers it as init-options and settings, exactly
    // as Helix delivers the same blob), so assert the conversion lands
    // correctly in both, not just one.

    // svlangserver: (systemverilog (includeIndexing . #("*.{v,vh,sv,svh}" "**/*.{v,vh,sv,svh}")))
    let expected_sv = serde_json::json!({
        "systemverilog": {
            "includeIndexing": ["*.{v,vh,sv,svh}", "**/*.{v,vh,sv,svh}"]
        }
    });
    let sv_settings = ed
        .state
        .lsp
        .registered_settings_for_test("svlangserver")
        .expect("svlangserver settings must be registered");
    assert_eq!(
        sv_settings, expected_sv,
        "a #(...) settings array must decode to a JSON array, not an object or a string"
    );
    let sv_init_options = ed
        .state
        .lsp
        .registered_init_options_for_test("svlangserver")
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
        .state
        .lsp
        .registered_settings_for_test("rust-analyzer")
        .expect("rust-analyzer settings must be registered");
    assert_eq!(
        ra_settings, expected_ra,
        "nested-object settings entries must round-trip through the converter exactly"
    );
    let ra_init_options = ed
        .state
        .lsp
        .registered_init_options_for_test("rust-analyzer")
        .expect("rust-analyzer init-options must be registered");
    assert_eq!(
        ra_init_options, expected_ra,
        "catalog config must reach init-options with the same shape as settings"
    );
}

/// An unparsable receipt reads as an interrupted install, and the receipt read
/// after it is unaffected.
#[test]
fn an_unparsable_receipt_is_an_interrupted_install() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "gopls", "0.0.1", "gopls");
    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        "2026-07-06",
        "rust-analyzer",
    );
    let receipt = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("gopls")
        .join("receipt.scm");
    std::fs::write(&receipt, "(").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install_eager(&mut ed, data_tmp.path());

    let log = ed.state.message_log.format_for_display();
    assert_eq!(
        ed.state.lsp.registered_command_for_test("gopls"),
        None,
        "{log}"
    );
    assert!(
        log.contains("interrupted install of gopls"),
        "must warn about the interrupted install, naming the server: {log}"
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .is_some(),
        "the next receipt must still read: {log}"
    );
}

#[test]
fn interrupted_install_is_warned_and_not_registered() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    let dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    std::fs::create_dir_all(&dir).unwrap();
    // No receipt.scm written: simulates an install that died mid-flight.
    std::fs::write(dir.join("rust-analyzer"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install_eager(&mut ed, data_tmp.path());

    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
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
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(
        data_tmp.path(),
        "totally-not-a-real-server",
        "1.0.0",
        "totally-not-a-real-server",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install_eager(&mut ed, data_tmp.path());

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
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    // A file, not a directory, sitting directly under servers/, exactly
    // where lsp-install/acquire-lock! puts it and register-installed-servers!
    // scans.
    std::fs::write(servers_dir.join(".install-lock"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains(".install-lock"),
        "the lock sentinel file must never be scanned as an interrupted/orphan server: {log}"
    );
}

#[test]
fn stray_non_directory_file_under_servers_dir_is_never_scanned_as_a_server() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    // A file, not a directory, with no special-cased name (e.g. a
    // Finder-dropped .DS_Store). Must be excluded on being a non-directory,
    // not on matching a specific filename.
    std::fs::write(servers_dir.join(".DS_Store"), b"").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains(".DS_Store") && !log.contains("interrupted install"),
        "a stray non-directory file must never be scanned as an interrupted/orphan server: {log}"
    );
}

/// A mid-session rescan (the one `:lsp-install` runs after a
/// successful/up-to-date install) must never replace a
/// registration the user made under the catalog server's own name: the scan
/// registers only names nothing has registered yet. An unconditional
/// re-registration would replace a manual `register-lsp-server!` override
/// (documented workflow: a local build, a version the catalog doesn't
/// carry, or a `$PATH` copy the user wants to take precedence; see
/// user-manual/docs/lsp.md) on the next rescan.
#[test]
fn rescan_does_not_clobber_a_manual_registration_of_the_same_name() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)\n\
         (register-lsp-server! \"rust-analyzer\" \
         #:command \"my-custom-rust-analyzer\")",
    );
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some("my-custom-rust-analyzer".to_owned()),
        "precondition: the manual registration from init.scm took effect"
    );

    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        &seeded_version("rust-analyzer"),
        "rust-analyzer",
    );
    type_cmd(&mut ed, ":lsp-install rust-analyzer");

    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some("my-custom-rust-analyzer".to_owned()),
        "a rescan must not overwrite a server the user registered manually under the same name"
    );
}

/// A manual registration under a name of its own is a separate server for
/// the language: a rescan registers the catalog server beside it and leaves
/// the manual one as it was.
#[test]
fn rescan_registers_the_catalog_server_beside_a_manual_one_of_another_name() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)\n\
         (register-lsp-server! \"my-rust-server\" \
         #:command \"my-custom-rust-analyzer\")",
    );

    fabricate_server(
        data_tmp.path(),
        "rust-analyzer",
        &seeded_version("rust-analyzer"),
        "rust-analyzer",
    );
    type_cmd(&mut ed, ":lsp-install rust-analyzer");

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "a manual registration under another name must not stop the scan registering \
         the catalog server"
    );
    assert_eq!(
        ed.state.lsp.registered_command_for_test("my-rust-server"),
        Some("my-custom-rust-analyzer".to_owned()),
        "the manual registration under its own name must survive the rescan"
    );
}

/// When a seeded server is *already installed before init.scm even runs*, an
/// eager `(load-plugin! "core:lsp-install")` queues a Register op for it from its own
/// startup scan, in the very same eval as anything that follows. The user's
/// own `register-lsp-server!` queued *after* that `load-plugin!` line must
/// win: `register-lsp-server!` is last-wins over queue order. Differs from
/// `rescan_does_not_clobber_a_manual_registration_of_the_same_name` above:
/// there, the receipt is fabricated *after* init.scm's eval, so
/// `load-plugin!`'s own scan queues nothing competing for "rust-analyzer" in
/// that eval, so it never exercises this same-eval race at all.
#[test]
fn register_lsp_server_after_eager_load_plugin_overrides_the_scans_own_registration() {
    let _path = path_reader();
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
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)\n\
         (register-lsp-server! \"rust-analyzer\" \
         #:command \"my-custom-rust-analyzer\")",
    );

    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some("my-custom-rust-analyzer".to_owned()),
        "register-lsp-server! queued after load-plugin! must win over the scan's \
         own registration of the already-installed catalog server"
    );
}

/// Unlike the sibling test above, this queues the override *before* the
/// eager `load-plugin!` line. `lsp-server-registered?` reads through the
/// pending op queue, so the scan sees this earlier-queued registration and
/// skips "rust-analyzer" regardless of call order.
#[test]
fn register_lsp_server_before_eager_load_plugin_also_survives_the_scan() {
    let _path = path_reader();
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
        "(load-plugin! \"core:stdlib\")\n\
         (register-lsp-server! \"rust-analyzer\" \
         #:command \"my-custom-rust-analyzer\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)",
    );

    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some("my-custom-rust-analyzer".to_owned()),
        "register-lsp-server! queued before load-plugin! must survive the scan's \
         no-clobber filter, which now reads through the same-eval pending queue"
    );
}

/// A lazily-declared core:lsp-install (`#:languages`) still registers an installed
/// server once activated (the startup scan runs at activation time, not
/// only at eager `(load-plugin! "core:lsp-install")`), and the very buffer whose
/// language-set triggered the activation attaches to that server in the
/// same call, with no need to wait for a later effects-applying drain.
///
/// `activate_lazy_language_plugins` (called from `set_buffer_language`,
/// before `lsp_reconcile_buffer`) evaluates the plugin inline via
/// `activate_and_register` (`scripting/lazy.rs`), which applies the activating
/// body's queued side effects (including any `register-lsp-server!`)
/// through `apply_script_effects` before returning.
///
/// The buffer is given a real path (a pathless buffer attaches to no
/// server) so the attach assertions below exercise the attach path, not
/// just the registration.
#[test]
fn lazy_lsp_plugin_registers_installed_servers_on_language_activation() {
    let _path = path_reader();
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
        "(load-plugin! \"core:stdlib\")\n(declare-plugin! \"core:lsp-install\" #:languages '(\"rust\"))",
    );
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        None,
        "precondition: core:lsp-install must not have activated yet"
    );
    // Set the path *after* init_scripting (which re-detects language for
    // every already-open buffer and would otherwise activate core:lsp-install early,
    // before this test's own explicit `set_buffer_language` call below).
    ed.doc_mut().set_path(Some(file));

    let bid = ed.focused_buffer_id();
    assert!(
        !ed.state.buffer_positions.lsp.has_doc(bid),
        "precondition: buffer must be unattached before core:lsp-install activates"
    );
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

    let expected_cmd = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer")
        .join("rust-analyzer");
    assert_eq!(
        ed.state.lsp.registered_command_for_test("rust-analyzer"),
        Some(expected_cmd.to_string_lossy().into_owned()),
        "activating core:lsp-install via a language-set trigger must apply its startup scan \
         immediately, in the same set_buffer_language call"
    );
    let attached: Vec<_> = ed.state.buffer_positions.lsp.servers(bid).collect();
    assert_eq!(
        attached,
        ed.state.lsp.instances_named_for_test("rust-analyzer"),
        "the buffer whose language-set triggered activation must attach to rust-analyzer \
         in that same call, not wait for a later effects-applying drain"
    );
    assert_eq!(attached.len(), 1);
    assert_eq!(ed.state.lsp.instance_count_for_test(), 1);
}

/// Setting a language activates only the registration entry; the install
/// commands' entry stays declared until a command runs.
#[test]
fn language_set_loads_the_registration_entry_and_leaves_the_commands_entry_lazy() {
    use hume_scripting::PluginStatus;
    use hume_scripting::attribution::{EntryFile, EntryId, PluginId};

    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp-install\")",
    );
    let plugin = PluginId::parse("core:lsp-install").unwrap();
    let registration = EntryId::main(plugin.clone());
    let commands = EntryId::new(plugin, EntryFile::parse("commands.scm").unwrap());
    let status = |ed: &Editor, id: &EntryId| ed.scripting.as_ref().unwrap().plugin_status(id);
    assert_eq!(status(&ed, &registration), Some(PluginStatus::Declared));
    assert_eq!(status(&ed, &commands), Some(PluginStatus::Declared));

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

    assert_eq!(status(&ed, &registration), Some(PluginStatus::Loaded));
    assert_eq!(status(&ed, &commands), Some(PluginStatus::Declared));

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    assert_eq!(status(&ed, &commands), Some(PluginStatus::Loaded));
}

/// A `:`-typed command can activate a lazily-declared core:lsp-install when the
/// command name is listed in the plugin's manifest:
/// dispatch runs `activate_lazy_plugin` before arity marshalling (see
/// input_stack/command.rs), so `:lsp-install` on a plugin that hasn't
/// loaded yet still works, no eager `(load-plugin! "core:lsp-install")` required.
#[test]
fn lazy_lsp_plugin_activates_on_typed_lsp_install_command() {
    let _path = path_reader();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp-install\")",
    );

    type_cmd(&mut ed, ":lsp-install not-a-real-language-xyz");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-install: no language or server named"),
        "dispatching :lsp-install must activate the lazily-declared plugin \
         and then run normally: {log}"
    );
}
