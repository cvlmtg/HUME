//! The two `:` completion sources that read the runtime's `themes/` dir —
//! `:theme <Tab>` (`THEME_SOURCE`, a `String` source offering
//! `theme_name_candidates`'s whole universe) and `:set global theme=<Tab>`
//! (`complete_set`'s `Delegated` value phase reaching the same list).
//!
//! Sets only `HUME_RUNTIME` (not `TMPDIR`) so neither test can race the
//! unguarded path-completion tests, whose `tempfile::tempdir()` respects
//! `TMPDIR`. The shared `TEST_GLOBALS` claim still serializes against other
//! `HUME_RUNTIME`-sensitive tests.

use super::*;

/// A runtime dir holding two themes (so the popup opens — a single
/// candidate completes silently), installed as `HUME_RUNTIME` for the
/// guard's lifetime. Fields drop in order: the var is cleared, then the dir
/// is deleted, then the claim is released.
struct TwoThemesRuntime {
    _dir: tempfile::TempDir,
    _lock: ClaimGuard,
}

impl TwoThemesRuntime {
    // `set_var` below mutates process-global `HUME_RUNTIME`, always under the
    // `Global::Env` claim taken first and held for the guard's lifetime —
    // the sanctioned-caller shape `clippy.toml`'s `disallowed-methods` entry
    // exists to protect.
    #[allow(clippy::disallowed_methods)]
    fn new() -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);
        let dir = safe_tempdir();
        let themes_dir = dir.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(themes_dir.join("zorro.toml"), b"").unwrap();
        std::fs::write(themes_dir.join("alpha.toml"), b"").unwrap();
        unsafe { std::env::set_var("HUME_RUNTIME", dir.path()) }
        Self {
            _dir: dir,
            _lock: lock,
        }
    }
}

impl Drop for TwoThemesRuntime {
    // Sanctioned caller — see `Self::new`.
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        unsafe { std::env::remove_var("HUME_RUNTIME") }
    }
}

/// `:` + `input` + Tab, then every candidate's `insert_text`.
fn theme_candidates_for(input: &str) -> Vec<String> {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());
    let session = ed
        .state
        .input
        .completion()
        .expect("two themes must open a popup");
    (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect()
}

#[test]
fn tab_completes_set_global_theme_value() {
    let _runtime = TwoThemesRuntime::new();
    let names = theme_candidates_for("set global theme=");
    assert!(names.iter().any(|n| n == "zorro"), "{names:?}");
    assert!(names.iter().any(|n| n == "alpha"), "{names:?}");
}

#[test]
fn tab_on_theme_arg_completes_theme_names() {
    let _runtime = TwoThemesRuntime::new();
    let names = theme_candidates_for("theme ");
    assert!(names.iter().any(|n| n == "zorro"), "{names:?}");
    assert!(names.iter().any(|n| n == "alpha"), "{names:?}");
}
