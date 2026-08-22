use super::*;

/// `:set global theme=<name>` must surface installed themes — verifying the
/// SetCompleter dispatch reaches `theme_name_candidates` (shared with
/// `:theme`) and that the value phase for `theme` is wired end-to-end.
///
/// Sets only `HUME_RUNTIME` (not `TMPDIR`) so it cannot race with the
/// unguarded path-completion tests, whose `tempfile::tempdir()` respects
/// `TMPDIR`. The shared `TEST_GLOBALS` claim still serializes against other
/// `HUME_RUNTIME`-sensitive tests.
#[test]
fn tab_completes_set_global_theme_value() {
    struct HumeRuntimeOnly {
        _dir: tempfile::TempDir,
        _lock: ClaimGuard,
    }
    impl Drop for HumeRuntimeOnly {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("HUME_RUNTIME") }
        }
    }
    let lock = TEST_GLOBALS.claim(Global::Env);
    let dir = safe_tempdir();
    // Two themes so the popup opens (a single candidate completes silently).
    let themes_dir = dir.path().join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    std::fs::write(themes_dir.join("zorro.toml"), b"").unwrap();
    std::fs::write(themes_dir.join("alpha.toml"), b"").unwrap();
    unsafe { std::env::set_var("HUME_RUNTIME", dir.path()) }
    let _guard = HumeRuntimeOnly {
        _dir: dir,
        _lock: lock,
    };

    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "set global theme=".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    let state = ed
        .state
        .minibuf_completion
        .as_ref()
        .expect("theme value should open a popup (>=2 candidates)");
    let names: Vec<&str> = state
        .candidates
        .iter()
        .map(|c| c.replacement.as_str())
        .collect();
    assert!(
        names.contains(&"zorro"),
        "theme candidate missing: {names:?}"
    );
    assert!(
        names.contains(&"alpha"),
        "theme candidate missing: {names:?}"
    );
}
