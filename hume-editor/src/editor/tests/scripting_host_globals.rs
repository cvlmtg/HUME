//! Drift test for the generated `steel-language-server` host-globals file
//! (`runtime/plugins/core/steel-server/lsp-home/hume-globals.scm`): keeps it
//! in sync with `hume_scripting::ScriptingHost::host_global_names`, the
//! single source of truth for which Steel identifiers HUME's own layers
//! add on top of a pristine engine.

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;

/// `<repo>/runtime/plugins/core/steel-server/lsp-home/`.
fn lsp_home_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("runtime/plugins/core/steel-server/lsp-home")
}

/// Builds a `ScriptingHost` + `Editor` in exactly the state
/// `Editor::init_scripting` reaches right before it would evaluate a user's
/// `init.scm`: native command names pre-registered
/// (`register_command_names`), then `prelude.scm` → `languages.scm` →
/// `grammars.scm` evaluated in production order (mirrors
/// `scripting_setup.rs`'s own `eval_runtime_scheme` sequence, which is
/// private to that module — this re-runs the same two calls per file rather
/// than exposing it). No user `init.scm` runs, so `host.host_global_names()`
/// afterward is exactly HUME's shipped surface, independent of any
/// developer's own config. Also returns the real command-registry names, so
/// a caller that evaluates more `.scm` on top (e.g. `plugin.scm`) can do so
/// with the same `builtin_names` production passes.
///
/// `grammars.scm` calls `(runtime-dir)` to build grammar paths, so
/// `HUME_RUNTIME` must point at the real `runtime/` dir for the duration —
/// `cargo test`'s cwd is the crate dir, not the workspace root, so
/// `hume_platform::dirs::runtime_dir()`'s cwd-relative fallback misses.
/// `HUME_RUNTIME` is process-global; the lock is scoped to just
/// `ScriptingHost::new()` (the only step that reads it), not held for the
/// rest of this function — a caller building on the result (e.g. via
/// `safe_tempdir()`) would otherwise self-deadlock on the same
/// `HUME_RUNTIME_MUTEX`.
fn host_and_editor_after_runtime_layers() -> (ScriptingHost, Editor, rustc_hash::FxHashSet<String>)
{
    let runtime_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("runtime");

    let mut host = {
        let _lock = super::HUME_RUNTIME_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY (not the unsafe-block kind — env vars are just inherently
        // process-global): guarded by the lock above.
        unsafe {
            std::env::set_var("HUME_RUNTIME", &runtime_root);
        }
        let host = ScriptingHost::new();
        unsafe {
            std::env::remove_var("HUME_RUNTIME");
        }
        host
    };

    let mut ed = editor_from("-[a]>b\n");
    let native_names: Vec<&str> = ed.state.config.registry.native_mappable_names().collect();
    host.register_command_names(&native_names);

    let builtin_names: rustc_hash::FxHashSet<String> =
        ed.state.config.registry.names().map(String::from).collect();

    for rel in ["prelude.scm", "languages.scm", "grammars.scm"] {
        let path = runtime_root.join("scheme").join(rel);
        let effects = {
            let mut ih = make_init_host(&mut ed.state, &mut ed.view);
            host.eval_init(&path, 10_000, &mut ih, builtin_names.clone())
                .unwrap_or_else(|e| panic!("evaluating runtime/scheme/{rel}: {}", e.message))
        };
        ed.apply_script_effects(effects);
    }
    (host, ed, builtin_names)
}

/// Renders `hume-globals.scm`'s exact expected content from `names`. Panics
/// on a name containing a character that can't sit inside a Scheme string
/// literal unescaped — none of HUME's builtin/macro/command names ever do
/// (they're all identifier-shaped), so this is a canary against a future
/// name that would otherwise emit malformed Scheme. Malformed Scheme is not
/// a cosmetic bug here: `steel-language-server` compiles every file in its
/// `STEEL_LSP_HOME` directory at startup and `.unwrap()`s the result
/// (`steel-language-server-0.8.2/src/main.rs`), so a bad line panics the
/// server instead of producing a diagnostic.
fn render_hume_globals_scm(names: &[String]) -> String {
    for name in names {
        assert!(
            !name.contains(['"', '\\']) && !name.chars().any(char::is_whitespace),
            "host global name {name:?} cannot be embedded in a Scheme string literal \
             unescaped — update render_hume_globals_scm to escape it"
        );
    }
    let mut out = String::from(
        "\
;;; runtime/plugins/core/steel-server/lsp-home/hume-globals.scm — GENERATED, do not hand-edit.
;;;
;;; Read only by `steel-language-server` (never by HUME's own Steel engine) —
;;; `runtime/plugins/core/steel-server/plugin.scm` points STEEL_LSP_HOME at
;;; this file's directory so the language server stops reporting HUME's own
;;; builtins, bootstrap wrappers, prelude macros, and native command names as
;;; `free identifier` errors while editing init.scm or a plugin file.
;;;
;;; Regenerate after any change to HUME's Steel builtins, bootstrap.scm, or
;;; runtime/scheme/{prelude,languages,grammars}.scm:
;;;
;;;   HUME_WRITE_STEEL_GLOBALS=1 cargo test -p hume-editor hume_globals_scm_matches_generated_host_names
;;;
;;; hume-editor/src/editor/tests/scripting_host_globals.rs's drift test fails
;;; the build if this file falls out of sync.

",
    );
    for name in names {
        out.push_str("(#%register-global \"");
        out.push_str(name);
        out.push_str("\")\n");
    }
    out
}

/// Fail oracle: delete one line from the shipped `hume-globals.scm` (or
/// edit one name) — this test must fail, naming the file as stale.
#[test]
fn hume_globals_scm_matches_generated_host_names() {
    let (host, ..) = host_and_editor_after_runtime_layers();
    let names = host.host_global_names();
    assert!(
        !names.is_empty(),
        "sanity: host_global_names() must not be empty after loading HUME's own runtime layers"
    );
    let expected = render_hume_globals_scm(&names);

    let path = lsp_home_dir().join("hume-globals.scm");

    if std::env::var("HUME_WRITE_STEEL_GLOBALS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("parent dir")).expect("create lsp-home dir");
        std::fs::write(&path, &expected).expect("write hume-globals.scm");
        return;
    }

    let actual = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — generate it with:\n  \
             HUME_WRITE_STEEL_GLOBALS=1 cargo test -p hume-editor \
             hume_globals_scm_matches_generated_host_names",
            path.display()
        )
    });

    assert_eq!(
        actual, expected,
        "\nruntime/plugins/core/steel-server/lsp-home/hume-globals.scm is stale. Regenerate with:\n\
         \n  HUME_WRITE_STEEL_GLOBALS=1 cargo test -p hume-editor hume_globals_scm_matches_generated_host_names\n"
    );
}

/// End-to-end check of the real shipped `core:steel-server` plugin body
/// (`runtime/plugins/core/steel-server/plugin.scm`): loading it and calling
/// `steel-server/register!` directly (bypassing the `which
/// "steel-language-server"` gate, since the binary need not be installed to
/// run this suite) must register `"scheme"` with `#:env` pointing
/// `STEEL_LSP_HOME` at the real, existing `lsp-home/` directory this same
/// module's drift test keeps in sync.
///
/// Flip: rename `lsp-home/` on disk (or point `steel-server/lsp-home` at a
/// nonexistent directory) — this test starts failing on the `is_some`
/// assertion, since the plugin's own `path-exists?` guard falls through to
/// the `#f`/no-`#:env` branch.
#[test]
fn steel_server_plugin_registers_scheme_with_generated_globals_env() {
    let (mut host, mut ed, builtin_names) = host_and_editor_after_runtime_layers();
    let runtime_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("runtime");

    let plugin_path = runtime_root.join("plugins/core/steel-server/plugin.scm");
    let effects = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&plugin_path, 10_000, &mut ih, builtin_names.clone())
            .unwrap_or_else(|e| panic!("evaluating plugin.scm: {}", e.message))
    };
    ed.apply_script_effects(effects);

    // `plugin.scm`'s own load-time tail already calls `steel-server/register!`
    // when `steel-language-server` happens to be on this machine's `$PATH`
    // (harmless no-op below, guarded by `unless (lsp-registered-for-language?
    // "scheme")`) — call it explicitly too so the test also passes in CI,
    // where the binary is never installed.
    let src = r#"(define-command! "probe-steel-server-register" "" (lambda () (steel-server/register!)))"#;
    let cmd_tmp = safe_tempdir();
    eval_with_real_host(&mut ed, &mut host, src, cmd_tmp.path());
    type_cmd(&mut ed, ":probe-steel-server-register");

    assert_eq!(
        ed.lsp.config_command_for_test("scheme").as_deref(),
        Some("steel-language-server"),
        "steel-server/register! must register the scheme language"
    );
    let env = ed
        .lsp
        .config_env_for_test("scheme")
        .expect("scheme must be registered");
    let lsp_home = env
        .iter()
        .find(|(k, _)| k == "STEEL_LSP_HOME")
        .map(|(_, v)| v.clone())
        .expect("#:env must carry a STEEL_LSP_HOME entry");

    // Compare canonicalized forms, not raw strings: `(runtime-dir)` returns
    // a canonicalized display path (`ScriptDirs::new`), which need not be
    // byte-identical to this test's own `<CARGO_MANIFEST_DIR>/../runtime`
    // construction (e.g. macOS's `/var` → `/private/var` symlink).
    let expected = std::fs::canonicalize(runtime_root.join("plugins/core/steel-server/lsp-home"))
        .expect("lsp-home dir exists on disk");
    let actual = std::fs::canonicalize(&lsp_home)
        .unwrap_or_else(|e| panic!("STEEL_LSP_HOME {lsp_home:?} does not exist: {e}"));
    assert_eq!(
        actual, expected,
        "STEEL_LSP_HOME must point at the generated host-globals directory"
    );
    assert!(
        actual.join("hume-globals.scm").is_file(),
        "STEEL_LSP_HOME must contain the generated hume-globals.scm"
    );
}
