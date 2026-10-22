use super::*;
use hume_platform::dirs::Dirs;

mod dispatch_parity;
mod event_and_declare;
mod keymap_and_language_lint;
mod language_activation;
mod lazy_activation;
mod manifest_e2e;
mod stdlib_plugin;

/// Helper: write `init_scm` to a temporary config dir, point a fresh Editor's
/// config, data and runtime directories at it and fresh tempdirs, and call
/// `init_scripting`.  Caller must keep the returned `Vec<TempDir>` alive.
///
/// `runtime_dir`: `None` points the runtime directory at a fresh empty tempdir (for
/// synthetic-fixture tests with no shipped plugin sources) and includes it in
/// the returned `Vec`; `Some(path)` points at `path` instead (typically the
/// repo's real `runtime/` tree, for tests exercising a real shipped
/// `manifest.scm`/plugin end to end) and does not add a tempdir for it, since
/// the caller owns that path's lifetime.
fn setup_editor_with_init_scripting(
    init_scm: &str,
    runtime_dir: Option<&std::path::Path>,
) -> (Editor, Vec<tempfile::TempDir>) {
    setup_editor_with_init_files(init_scm, &[], runtime_dir)
}

/// [`setup_editor_with_init_scripting`] with extra files (relative path,
/// source) written beside `init.scm`, for local plugins.
fn setup_editor_with_init_files(
    init_scm: &str,
    files: &[(&str, &str)],
    runtime_dir: Option<&std::path::Path>,
) -> (Editor, Vec<tempfile::TempDir>) {
    let config_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = runtime_dir.is_none().then(|| tempfile::tempdir().unwrap());
    let runtime_path = runtime_dir.unwrap_or_else(|| runtime_tmp.as_ref().unwrap().path());

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_scm).unwrap();
    for (name, src) in files {
        let path = hume_config.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, src).unwrap();
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_tmp.path().join("hume")),
        runtime: Some(runtime_path.to_path_buf()),
        ..process_dirs()
    };
    ed.init_scripting(&mut Default::default());

    let mut tmps = vec![config_tmp, data_tmp];
    tmps.extend(runtime_tmp);
    (ed, tmps)
}

/// Helper: create a `user/tp` plugin file, write init.scm, run `init_scripting`.
///
/// Parallels `setup_editor_with_init_scripting` but also puts a plugin on disk so
/// `#:languages` activation entries for `"user/tp"` are actually recorded.  (Absent-path
/// plugins early-return in `declare_plugin` and skip activation registration.)
fn setup_lang_lint_editor(init_body: &str) -> (Editor, Vec<tempfile::TempDir>) {
    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

    // Trivial plugin body: the lint checks activation entry names, not plugin behaviour.
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

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_tmp.path().join("hume")),
        runtime: Some(runtime_tmp.path().to_path_buf()),
        ..Dirs::none()
    };
    ed.init_scripting(&mut Default::default());

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}
