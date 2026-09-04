//! `theme_search_paths` tier ordering: config dir, then data dir, then
//! runtime dir — each shadowing the next by stem. XDG env vars are
//! unix-only (`dirs.rs`'s `config_dir_with`/`data_dir_with`), hence gated
//! here rather than in the portable `tests/theme_loading.rs`.

use super::*;

/// Points `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`HUME_RUNTIME` at three distinct
/// tempdirs so all three tiers resolve to known, isolated paths.
struct ThemeDirsFixture {
    config_dir: PathBuf,
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    _tmps: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
    _lock: ClaimGuard,
}

impl ThemeDirsFixture {
    fn new() -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);

        let config_tmp = safe_tempdir();
        let data_tmp = safe_tempdir();
        let runtime_tmp = safe_tempdir();

        let config_dir = config_tmp.path().join("hume");
        let data_dir = data_tmp.path().join("hume");

        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
            std::env::set_var("XDG_DATA_HOME", data_tmp.path());
            std::env::set_var("HUME_RUNTIME", runtime_tmp.path());
        }

        Self {
            config_dir,
            data_dir,
            runtime_dir: runtime_tmp.path().to_path_buf(),
            _tmps: (config_tmp, data_tmp, runtime_tmp),
            _lock: lock,
        }
    }
}

impl Drop for ThemeDirsFixture {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("HUME_RUNTIME");
        }
    }
}

#[test]
fn theme_search_paths_orders_config_then_data_then_runtime() {
    let fixture = ThemeDirsFixture::new();

    let paths = crate::editor::scripting_setup::theme_search_paths();

    assert_eq!(
        paths,
        vec![
            fixture.config_dir.join("themes"),
            fixture.data_dir.join("themes"),
            fixture.runtime_dir.join("themes"),
        ],
        "expected config, then data, then runtime themes dirs, in that order"
    );
}

/// A theme present in both the data dir and the runtime dir must resolve to
/// the data-dir copy — installed third-party themes shadow bundled ones,
/// same as config-dir themes already do.
#[test]
fn data_dir_theme_shadows_bundled_theme_of_same_name() {
    let fixture = ThemeDirsFixture::new();

    let data_themes = fixture.data_dir.join("themes");
    std::fs::create_dir_all(&data_themes).unwrap();
    std::fs::write(
        data_themes.join("sand.toml"),
        br##""ui.cursor.primary" = { fg = "#ff00ff" }"##,
    )
    .unwrap();

    let runtime_themes = fixture.runtime_dir.join("themes");
    std::fs::create_dir_all(&runtime_themes).unwrap();
    std::fs::write(
        runtime_themes.join("sand.toml"),
        br##""ui.cursor.primary" = { fg = "#000000" }"##,
    )
    .unwrap();

    let theme = hume_engine::theme::loader::load_theme(
        "sand",
        &crate::editor::scripting_setup::theme_search_paths(),
    )
    .expect("sand theme should load from the data dir");
    let style = theme.resolve_by_name(hume_engine::types::Scope("ui.cursor.primary"));
    assert_eq!(
        style.fg,
        Some(hume_grid::Rgb(0xff, 0x00, 0xff)),
        "expected the data-dir theme to shadow the runtime-dir one"
    );
}
