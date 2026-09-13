// `std::env::set_var`/`remove_var` in setup_editor_with_init_scripting and
// setup_lang_lint_editor below mutate process-global XDG_*/HUME_RUNTIME/HOME
// vars — always under a `TEST_GLOBALS` claim. `clippy.toml`'s
// `disallowed-methods` entry exists so a *new* raw call elsewhere in the
// crate gets caught; this file is the sanctioned caller it lists as exempt
// for this test group — the only file in this directory that needs the
// allow, since every raw env call lives in one of the two helpers below and
// no leaf module calls std::env directly.
#![allow(clippy::disallowed_methods)]

use super::*;

mod dispatch_parity;
mod event_and_declare;
mod keymap_and_language_lint;
mod language_activation;
mod lazy_activation;
mod manifest_e2e;
mod stdlib_plugin;

/// Helper: create a user plugin at `plugins/user/tp/plugin.scm`, write
/// `init.scm`, evaluate it, set up the lazy stubs, and wire the host into
/// `ed`.  Caller must keep `TempDir` alive.
fn setup_lazy_editor(init_body: &str, plugin_body: &str) -> (Editor, tempfile::TempDir) {
    let dir = safe_tempdir();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin_body).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, init_body).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed in setup_lazy_editor");

    ed.scripting = Some(host);
    (ed, dir)
}

/// Helper: write `init_scm` to a temporary config dir, set `XDG_CONFIG_HOME`
/// and `HUME_RUNTIME`, call `init_scripting` on a fresh Editor, restore env
/// vars before returning.  Caller must keep the returned `Vec<TempDir>` alive.
///
/// `runtime_dir`: `None` points `HUME_RUNTIME` at a fresh empty tempdir (for
/// synthetic-fixture tests with no shipped plugin sources) and includes it in
/// the returned `Vec`; `Some(path)` points at `path` instead (typically the
/// repo's real `runtime/` tree, for tests exercising a real shipped
/// `manifest.scm`/plugin end to end) and does not add a tempdir for it, since
/// the caller owns that path's lifetime.
fn setup_editor_with_init_scripting(
    init_scm: &str,
    runtime_dir: Option<&std::path::Path>,
) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let config_tmp = safe_tempdir();
    let data_tmp = safe_tempdir();
    let runtime_tmp = runtime_dir.is_none().then(safe_tempdir);
    let runtime_path = runtime_dir.unwrap_or_else(|| runtime_tmp.as_ref().unwrap().path());

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_scm).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", runtime_path);
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    let mut dirs = vec![config_tmp, data_tmp];
    dirs.extend(runtime_tmp);
    (ed, dirs)
}

/// Helper: create a `user/tp` plugin file, write init.scm, run `init_scripting`.
///
/// Parallels `setup_editor_with_init_scripting` but also puts a plugin on disk so
/// `#:languages` activation entries for `"user/tp"` are actually recorded.  (Absent-path
/// plugins early-return in `declare_plugin` and skip activation registration.)
fn setup_lang_lint_editor(init_body: &str) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let config_tmp = safe_tempdir();
    let runtime_tmp = safe_tempdir();
    let data_tmp = safe_tempdir();

    // Trivial plugin body — the lint checks activation entry names, not plugin behaviour.
    let plugin_dir = data_tmp
        .path()
        .join("hume")
        .join("plugins")
        .join("user")
        .join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), "(+ 1 0)").unwrap();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_body).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", runtime_tmp.path());
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}
