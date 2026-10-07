//! The two `:` completion sources that read the runtime's `themes/` dir:
//! `:theme <Tab>` (`THEME_SOURCE`, a `String` source offering
//! `theme_name_candidates`'s whole universe) and `:set global theme=<Tab>`
//! (`complete_set`'s `Delegated` value phase reaching the same list).

use super::*;

/// A runtime dir holding two themes, so the popup opens (a single candidate
/// completes without one).
struct TwoThemesRuntime {
    dir: tempfile::TempDir,
}

impl TwoThemesRuntime {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let themes_dir = dir.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(themes_dir.join("zorro.toml"), b"").unwrap();
        std::fs::write(themes_dir.join("alpha.toml"), b"").unwrap();
        Self { dir }
    }
}

/// `:` + `input` + Tab, then every candidate's `insert_text`.
fn theme_candidates_for(runtime: &TwoThemesRuntime, input: &str) -> Vec<String> {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = Dirs {
        runtime: Some(runtime.dir.path().to_path_buf()),
        ..Dirs::none()
    };
    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());
    let session = ed
        .state
        .input
        .minibuf_completion()
        .expect("two themes must open a popup");
    (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect()
}

#[test]
fn tab_completes_set_global_theme_value() {
    let runtime = TwoThemesRuntime::new();
    let names = theme_candidates_for(&runtime, "set global theme=");
    assert!(names.iter().any(|n| n == "zorro"), "{names:?}");
    assert!(names.iter().any(|n| n == "alpha"), "{names:?}");
}

#[test]
fn tab_on_theme_arg_completes_theme_names() {
    let runtime = TwoThemesRuntime::new();
    let names = theme_candidates_for(&runtime, "theme ");
    assert!(names.iter().any(|n| n == "zorro"), "{names:?}");
    assert!(names.iter().any(|n| n == "alpha"), "{names:?}");
}
