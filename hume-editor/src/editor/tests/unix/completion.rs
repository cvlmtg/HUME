use super::*;

/// `:set global theme=<name>` must surface installed themes — verifying the
/// `complete_set` dispatch reaches `theme_name_candidates` (shared with
/// `:theme`) and that the value phase for `theme` is wired end-to-end.
///
/// Sets only `HUME_RUNTIME` (not `TMPDIR`) so it cannot race with the
/// unguarded path-completion tests, whose `tempfile::tempdir()` respects
/// `TMPDIR`. The shared `TEST_GLOBALS` claim still serializes against other
/// `HUME_RUNTIME`-sensitive tests.
// `std::env::set_var`/`remove_var` below mutate the process-global
// `HUME_RUNTIME` var — sound only under the `TEST_GLOBALS.claim(Global::Env)`
// taken just below, held for the rest of this test via `HumeRuntimeOnly`'s
// `_lock` field, which is what makes this the sanctioned caller
// `clippy.toml`'s `disallowed-methods` entry lists as exempt.
#[test]
#[allow(clippy::disallowed_methods)]
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

    let session = ed
        .state
        .input
        .completion()
        .expect("theme value should open a popup (>=2 candidates)");
    let names: Vec<String> = (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect();
    assert!(
        names.iter().any(|n| n == "zorro"),
        "theme candidate missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "alpha"),
        "theme candidate missing: {names:?}"
    );
}

/// `:theme <Tab>` — `complete_theme`'s own `THEME_SOURCE`, a `MatchKind::
/// String` source that offers `theme_name_candidates`'s full universe for
/// the session to narrow. Distinct from `tab_completes_set_global_theme_value`
/// above, which reaches the same candidate list through `complete_set`'s
/// `Delegated` value phase instead — this test is `:theme`'s own, and was
/// previously the only registered source with no coverage at all.
#[test]
#[allow(clippy::disallowed_methods)]
fn tab_on_theme_arg_completes_theme_names() {
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
    for ch in "theme ".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    let session = ed
        .state
        .input
        .completion()
        .expect(":theme <Tab> should open a popup (>=2 candidates)");
    let names: Vec<String> = (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect();
    assert!(
        names.iter().any(|n| n == "zorro"),
        "theme candidate missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "alpha"),
        "theme candidate missing: {names:?}"
    );
}
