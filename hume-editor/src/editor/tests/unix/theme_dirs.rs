//! `theme_search_paths` tier ordering: config dir, then data dir, then
//! runtime dir, each shadowing the next by stem.

use super::*;

/// Three distinct tempdirs standing in for the config, data and runtime
/// directories, so all three tiers resolve to known, isolated paths.
struct ThemeDirsFixture {
    dirs: hume_platform::dirs::Dirs,
    _tmps: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
}

impl ThemeDirsFixture {
    fn new() -> Self {
        let config_tmp = safe_tempdir();
        let data_tmp = safe_tempdir();
        let runtime_tmp = safe_tempdir();
        Self {
            dirs: hume_platform::dirs::Dirs {
                config: Some(config_tmp.path().join("hume")),
                data: Some(data_tmp.path().join("hume")),
                runtime: Some(runtime_tmp.path().to_path_buf()),
            },
            _tmps: (config_tmp, data_tmp, runtime_tmp),
        }
    }

    fn config_dir(&self) -> &Path {
        self.dirs.config.as_deref().expect("fixture sets config")
    }

    fn data_dir(&self) -> &Path {
        self.dirs.data.as_deref().expect("fixture sets data")
    }

    fn runtime_dir(&self) -> &Path {
        self.dirs.runtime.as_deref().expect("fixture sets runtime")
    }
}

#[test]
fn theme_search_paths_orders_config_then_data_then_runtime() {
    let fixture = ThemeDirsFixture::new();

    let paths = crate::editor::scripting_setup::theme_search_paths(&fixture.dirs);

    assert_eq!(
        paths,
        vec![
            fixture.config_dir().join("themes"),
            fixture.data_dir().join("themes"),
            fixture.runtime_dir().join("themes"),
        ],
        "expected config, then data, then runtime themes dirs, in that order"
    );
}

/// A theme present in both the data dir and the runtime dir must resolve to
/// the data-dir copy: installed third-party themes shadow bundled ones,
/// same as config-dir themes already do.
#[test]
fn data_dir_theme_shadows_bundled_theme_of_same_name() {
    let fixture = ThemeDirsFixture::new();

    let data_themes = fixture.data_dir().join("themes");
    std::fs::create_dir_all(&data_themes).unwrap();
    std::fs::write(
        data_themes.join("sand.toml"),
        br##""ui.cursor.primary" = { fg = "#ff00ff" }"##,
    )
    .unwrap();

    let runtime_themes = fixture.runtime_dir().join("themes");
    std::fs::create_dir_all(&runtime_themes).unwrap();
    std::fs::write(
        runtime_themes.join("sand.toml"),
        br##""ui.cursor.primary" = { fg = "#000000" }"##,
    )
    .unwrap();

    let theme = hume_engine::theme::loader::load_theme(
        "sand",
        &crate::editor::scripting_setup::theme_search_paths(&fixture.dirs),
    )
    .expect("sand theme should load from the data dir")
    .theme;
    let style = theme.resolve_by_name(hume_engine::types::Scope("ui.cursor.primary"));
    assert_eq!(
        style.fg,
        Some(hume_grid::Rgb(0xff, 0x00, 0xff)),
        "expected the data-dir theme to shadow the runtime-dir one"
    );
}

/// A theme with one malformed key still loads. This is the whole point of
/// warning instead of failing: the user isn't left on their old theme over
/// one bad line, and the warning still reaches them.
#[test]
fn load_theme_by_name_loads_despite_a_malformed_key_and_warns() {
    let fixture = ThemeDirsFixture::new();
    let runtime_themes = fixture.runtime_dir().join("themes");
    std::fs::create_dir_all(&runtime_themes).unwrap();
    std::fs::write(
        runtime_themes.join("flawed.toml"),
        br##""ui.text" = { fg = "#123456" }
"keyword" = "nonexistent_color"
"##,
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = fixture.dirs.clone();
    let ok = crate::editor::theme::load_theme_by_name(
        &mut ed.view,
        &mut ed.state.message_log,
        &mut ed.state.status_msg,
        ed.state.input.popup_mut(),
        &ed.state.dirs,
        "flawed",
    );
    assert!(ok, "a theme with a malformed key still loads");
    assert!(
        ed.state.message_log.has_unseen(),
        "the malformed key's warning must reach the message log"
    );
    // The status line is the half the user actually reads without opening
    // `:messages`, and the one bad key here makes it the singular branch.
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("theme 'flawed' loaded with 1 warning"),
        "a partial load must say so on the status line"
    );
    // The malformed entry lands as a default style rather than being dropped,
    // so it still blocks the dot-notation chain the way a real entry would.
    assert!(
        ed.view.theme.raw_contains("keyword"),
        "a malformed entry stays present, blocking fallback"
    );
    let text = ed
        .view
        .theme
        .resolve_by_name(hume_engine::types::Scope("ui.text"));
    assert_eq!(text.fg, Some(hume_grid::Rgb(0x12, 0x34, 0x56)));
}
