use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::*;
use crate::editor::dispatch::ArgSource;
use hume_scripting::ScriptingHost;

// ── core:pickers — end-to-end plugin tests ────────────────────────────────────
//
// Loads the *real* runtime/plugins/core/pickers/plugin.scm (via
// `include_str!`) into an isolated HUME_RUNTIME dir, then evaluates an
// init.scm that eagerly loads it — exercising the actual shipped file.
//
// Coverage note: the git-repo detection branch (`pickers/git-repo?`) is
// proven only via full integration in a real git repo (`files_picker_...`
// below) — its predicate is a plain (non-command) Steel function, not
// reachable from a test's own init.scm via `call!`, so there is no clean,
// hermetic seam to unit-test its `#f` case directly. The two non-git
// branches (fd found / fd absent) are covered hermetically below via the
// `pickers/files-picker-with` internal command, which *is* a registered
// command and so dispatchable by name via `call!` regardless of which
// Steel environment defined it.

const PICKERS_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/pickers/plugin.scm"
));

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false, ArgSource::Keymap);
}

/// Load the real `core:pickers` plugin, plus `extra_source` appended to the
/// same init.scm (evaluated after the plugin, so `call!`-dispatchable
/// commands the plugin registered are reachable by name — see the module
/// doc comment on why plain, non-command helpers are not used this way).
fn setup(guard: &HumeRuntimeGuard, tmp: &Path, input: &str, extra_source: &str) -> Editor {
    setup_with_config(guard, tmp, input, None, extra_source)
}

/// Like `setup`, but passes `config_expr` (a Scheme expression, e.g.
/// `(hash "untracked" #f)`) as `core:pickers`'s `#:config`.
fn setup_with_config(
    guard: &HumeRuntimeGuard,
    tmp: &Path,
    input: &str,
    config_expr: Option<&str>,
    extra_source: &str,
) -> Editor {
    write_core_plugin(guard, "pickers", PICKERS_PLUGIN);
    // core:pickers' config validation depends on core:stdlib (see
    // plugin.scm's header) — stage and load it first, same as core:git-diff.
    write_core_plugin(guard, "stdlib", STDLIB_PLUGIN);
    let mut ed = editor_from(input);
    let mut host = ScriptingHost::new();
    let load_pickers = match config_expr {
        Some(cfg) => format!("(load-plugin \"core:pickers\" #:config {cfg})"),
        None => "(load-plugin \"core:pickers\")".to_string(),
    };
    let source = format!("(load-plugin \"core:stdlib\")\n{load_pickers}\n{extra_source}");
    eval_with_real_host(&mut ed, &mut host, &source, tmp);
    ed.scripting = Some(host);
    ed
}

// ── Files picker — git branch (full integration) ──────────────────────────────

#[test]
fn files_picker_in_git_repo_uses_git_index_and_opens_selection() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git(sandbox.raw(), &["init", "-q"]);
    std::fs::write(sandbox.raw().join("alpha.txt"), "").unwrap();
    std::fs::write(sandbox.raw().join("beta.txt"), "").unwrap();
    std::fs::write(sandbox.raw().join("cached.txt"), "").unwrap();
    git(sandbox.raw(), &["add", "cached.txt"]);
    std::fs::remove_file(sandbox.raw().join("cached.txt")).unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('f'));
    drain_until_picker_total(&mut ed, 3);

    let picker = ed.state.config.picker.as_ref().expect("picker open");
    assert_eq!(picker.prompt(), "files: ");
    let rows: Vec<&str> = picker.window(10).collect();
    assert!(
        rows.contains(&"cached.txt"),
        "only `git ls-files --cached` can list a file deleted from disk — \
         proves the git branch (not fd) ran; got {rows:?}"
    );

    for ch in "beta".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_enter());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert!(
        path.ends_with("beta.txt"),
        "Enter must open the selected file; got {path:?}"
    );
}

#[test]
fn files_picker_esc_dismisses_cleanly() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git(sandbox.raw(), &["init", "-q"]);
    std::fs::write(sandbox.raw().join("alpha.txt"), "").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();
    let starting_bid = ed.focused_buffer_id();

    ed.feed_key(key('g'));
    ed.feed_key(key('f'));
    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            >= 1
    });

    ed.feed_key(key_esc());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    assert_eq!(
        ed.focused_buffer_id(),
        starting_bid,
        "Esc must not switch buffers"
    );

    // Keep interacting past the terminal action.
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    let text = ed.state.buffers.get(starting_bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after Esc must behave as plain input"
    );
}

// ── Files picker — non-git branches (hermetic, via the internal seam) ─────────

#[test]
fn files_picker_fd_branch_spawns_given_binary() {
    let guard = HumeRuntimeGuard::new();
    let tmp = safe_tempdir();

    let fake_fd = tmp.path().join("fake-fd");
    std::fs::write(&fake_fd, "#!/bin/sh\nprintf 'one.txt\\0two.txt'\n").unwrap();
    std::fs::set_permissions(&fake_fd, std::fs::Permissions::from_mode(0o755)).unwrap();

    let sandbox = CwdSandbox::new();
    std::fs::write(sandbox.raw().join("one.txt"), "").unwrap();
    std::fs::write(sandbox.raw().join("two.txt"), "").unwrap();

    let extra = format!(
        r#"(define-command! "test-fd-branch" "" (lambda ()
             (call! "pickers/files-picker-with" #f "{}")))"#,
        fake_fd.display()
    );
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", &extra);
    ed.set_cwd(&sandbox.path()).unwrap();

    call(&mut ed, "test-fd-branch");
    drain_until_picker_total(&mut ed, 2);
    let picker = ed.state.config.picker.as_ref().expect("picker open");
    assert_eq!(
        picker.window(10).collect::<Vec<_>>(),
        vec!["one.txt", "two.txt"]
    );

    ed.feed_key(key_enter());
    ed.settle();
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert!(path.ends_with("one.txt"), "got {path:?}");
}

#[test]
fn files_picker_error_path_names_fd() {
    let guard = HumeRuntimeGuard::new();
    let tmp = safe_tempdir();
    let extra = r#"(define-command! "test-error-branch" "" (lambda ()
                     (call! "pickers/files-picker-with" #f #f)))"#;
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", extra);

    ed.state.status_msg = None;
    call(&mut ed, "test-error-branch");

    let msg = ed
        .state
        .status_msg
        .clone()
        .expect("error must surface as a status message");
    assert!(msg.contains("fd"), "error must name fd; got: {msg}");
    assert!(ed.state.config.picker.is_none());
}

// ── Git-modified-files picker ──────────────────────────────────────────────────

#[test]
fn git_modified_picker_lists_changed_files_with_status_codes() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("a.txt"), "hello\n").unwrap();
    git(sandbox.raw(), &["add", "a.txt"]);
    git(sandbox.raw(), &["commit", "-q", "-m", "init"]);
    // Unstaged modification to a committed, tracked file.
    std::fs::write(sandbox.raw().join("a.txt"), "hello\nworld\n").unwrap();
    // Staged, never-committed addition.
    std::fs::write(sandbox.raw().join("b.txt"), "").unwrap();
    git(sandbox.raw(), &["add", "b.txt"]);
    // Untracked.
    std::fs::write(sandbox.raw().join("c.txt"), "").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    drain_until_picker_total(&mut ed, 3);

    let picker = ed.state.config.picker.as_ref().expect("picker open");
    assert_eq!(picker.prompt(), "git: ");
    assert_eq!(picker.total_len(), 3);
    let rows: Vec<&str> = picker.window(10).collect();
    assert!(
        rows.contains(&" M a.txt"),
        "unstaged modification must show ' M'; got {rows:?}"
    );
    assert!(
        rows.contains(&"A  b.txt"),
        "staged addition must show 'A '; got {rows:?}"
    );
    assert!(
        rows.contains(&"?? c.txt"),
        "untracked file must show '??'; got {rows:?}"
    );
}

// Fail oracle for this test: `PickerSession::seed` only clears `pending`
// when the seed is non-empty. `git status` hasn't run yet when this picker
// opens, so `picker!` seeds it with an empty list — if `seed` cleared
// `pending` unconditionally, this session would read as "already populated"
// from frame one, and `is_pending()` would be `#f` before `git status` has
// returned anything.
#[test]
fn git_modified_picker_is_pending_until_git_status_returns() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("a.txt"), "hello\n").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    assert!(
        ed.state
            .config
            .picker
            .as_ref()
            .expect("picker open")
            .is_pending(),
        "must be pending the instant it opens, before `git status` has had a \
         chance to run"
    );

    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| !p.is_pending())
            .unwrap_or(false)
    });

    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .expect("picker open")
            .total_len(),
        1,
        "the result that arrived must actually be there once pending clears"
    );
}

#[test]
fn git_modified_picker_accept_resolves_relative_to_repo_root_from_subdirectory() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("root.txt"), "hello\n").unwrap();
    git(sandbox.raw(), &["add", "root.txt"]);
    git(sandbox.raw(), &["commit", "-q", "-m", "init"]);
    std::fs::write(sandbox.raw().join("root.txt"), "hello\nworld\n").unwrap();
    std::fs::create_dir_all(sandbox.raw().join("sub")).unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    // :pwd is a subdirectory, not the repo root — git prints the entry as
    // "root.txt" (repo-root-relative); accept must not open it relative to
    // :pwd (which has no such file).
    ed.set_cwd(&sandbox.path().join("sub")).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    drain_until_picker_total(&mut ed, 1);
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .expect("picker open")
            .total_len(),
        1
    );

    ed.feed_key(key_enter());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert_eq!(
        path,
        sandbox.path().join("root.txt"),
        "accept must resolve the repo-root-relative path against the repo root, \
         not against :pwd (a subdirectory)"
    );
}

#[test]
fn git_modified_picker_row_and_accept_handle_path_with_space() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("has space.txt"), "hello\n").unwrap();
    git(sandbox.raw(), &["add", "has space.txt"]);
    git(sandbox.raw(), &["commit", "-q", "-m", "init"]);
    std::fs::write(sandbox.raw().join("has space.txt"), "hello\nworld\n").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    drain_until_picker_total(&mut ed, 1);

    let picker = ed.state.config.picker.as_ref().expect("picker open");
    let rows: Vec<&str> = picker.window(10).collect();
    assert_eq!(
        rows,
        vec![" M has space.txt"],
        "-z must yield the path verbatim, not git's default C-quoted form; got {rows:?}"
    );

    ed.feed_key(key_enter());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert_eq!(
        path,
        sandbox.path().join("has space.txt"),
        "accept must open the exact path parsed from the -z entry"
    );
}

#[test]
fn git_modified_picker_accept_resolves_nested_relative_path() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::create_dir_all(sandbox.raw().join("sub")).unwrap();
    std::fs::write(sandbox.raw().join("sub/file.txt"), "hello\n").unwrap();
    git(sandbox.raw(), &["add", "sub/file.txt"]);
    git(sandbox.raw(), &["commit", "-q", "-m", "init"]);
    std::fs::write(sandbox.raw().join("sub/file.txt"), "hello\nworld\n").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    // :pwd *is* the repo root here — the subdirectory case is covered by
    // `git_modified_picker_accept_resolves_relative_to_repo_root_from_subdirectory`
    // above; this test isolates `path-join`'s handling of a multi-segment
    // relative path instead.
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    drain_until_picker_total(&mut ed, 1);
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .expect("picker open")
            .total_len(),
        1
    );

    ed.feed_key(key_enter());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert_eq!(
        path,
        sandbox.path().join("sub/file.txt"),
        "accept must join a multi-segment repo-relative path against the root correctly"
    );
}

#[test]
fn git_modified_picker_untracked_false_config_hides_untracked_files() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("a.txt"), "hello\n").unwrap();
    git(sandbox.raw(), &["add", "a.txt"]);
    git(sandbox.raw(), &["commit", "-q", "-m", "init"]);
    std::fs::write(sandbox.raw().join("a.txt"), "hello\nworld\n").unwrap();
    std::fs::write(sandbox.raw().join("untracked.txt"), "").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup_with_config(
        &guard,
        tmp.path(),
        "-[h]>ello\n",
        Some("(hash \"untracked\" #f)"),
        "",
    );
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    drain_until_picker_total(&mut ed, 1);

    let picker = ed.state.config.picker.as_ref().expect("picker open");
    let rows: Vec<&str> = picker.window(10).collect();
    assert_eq!(
        rows,
        vec![" M a.txt"],
        "#f must hide the untracked file while keeping the modified one; got {rows:?}"
    );
}

#[test]
fn git_modified_picker_untracked_default_lists_files_inside_untracked_directory() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::create_dir_all(sandbox.raw().join("newdir")).unwrap();
    std::fs::write(sandbox.raw().join("newdir/file.txt"), "").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();
    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    drain_until_picker_total(&mut ed, 1);
    let rows: Vec<&str> = ed
        .state
        .config
        .picker
        .as_ref()
        .expect("picker open")
        .window(10)
        .collect();
    assert_eq!(
        rows,
        vec!["?? newdir/file.txt"],
        "default (untracked on) must expand into the individual file, not a bare \
         directory row a file picker can't open; got {rows:?}"
    );
}

#[test]
fn git_modified_picker_invalid_untracked_config_fails_load() {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "pickers", PICKERS_PLUGIN);
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);
    let tmp = safe_tempdir();
    let init_path = tmp.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:pickers\" #:config (hash \"untracked\" 'bogus))",
    )
    .unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    let result = {
        let mut ih = crate::editor::scripting_setup::make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    };
    let err = result
        .expect_err("a non-boolean \"untracked\" value must fail the load, not silently default");
    assert!(
        err.message.contains("untracked"),
        "error must name the offending config key, not just fail generically; got: {}",
        err.message
    );
}

/// Loading `core:pickers` without `core:stdlib` declared or loaded first
/// must fail `eval_init` at load time, naming `core:stdlib` —
/// `core:pickers`'s `(declared-plugins)` guard rejects it before its
/// `pickers/untracked` config read ever reaches `call!`.
#[test]
fn missing_stdlib_errors_at_load() {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "pickers", PICKERS_PLUGIN);
    // Deliberately no `write_core_plugin(&guard, "stdlib", ...)`.
    let tmp = safe_tempdir();
    let init_path = tmp.path().join("init.scm");
    std::fs::write(&init_path, "(load-plugin \"core:pickers\")").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    let result = {
        let mut ih = crate::editor::scripting_setup::make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    };
    let err = result.expect_err("core:pickers without core:stdlib must fail eval_init");
    assert!(
        err.message.contains("core:stdlib"),
        "error must name the missing dependency; got: {}",
        err.message
    );
}

#[test]
fn git_modified_picker_clean_tree_opens_empty_picker() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("a.txt"), "hello\n").unwrap();
    git(sandbox.raw(), &["add", "a.txt"]);
    git(sandbox.raw(), &["commit", "-q", "-m", "init"]);

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    // total_len() stays 0 whether the async `git status` callback has run
    // yet or not, so it can't be the drain predicate here — wait for the
    // job to leave the registry instead, so the assertion below proves the
    // callback actually ran and produced zero rows, not just that nothing
    // has happened yet.
    drain_until(&mut ed, |ed| ed.state.config.async_jobs.is_empty());

    let picker = ed
        .state
        .config
        .picker
        .as_ref()
        .expect("a clean tree still opens the picker, just with no rows");
    assert_eq!(picker.total_len(), 0);
}

#[test]
fn git_modified_picker_not_a_repo_names_git() {
    let guard = HumeRuntimeGuard::new();
    let tmp = safe_tempdir();
    let extra = r#"(define-command! "test-git-not-a-repo" "" (lambda ()
                     (call! "pickers/git-picker-with" #f)))"#;
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", extra);

    ed.state.status_msg = None;
    call(&mut ed, "test-git-not-a-repo");

    let msg = ed
        .state
        .status_msg
        .clone()
        .expect("error must surface as a status message");
    assert!(msg.contains("git"), "error must name git; got: {msg}");
    assert!(ed.state.config.picker.is_none());
}

#[test]
fn git_modified_picker_git_status_failure_does_not_say_clean() {
    let guard = HumeRuntimeGuard::new();
    // Not a repo — no `git init`. The seam's `root` argument is only used by
    // `on-select`'s `path-join`, never to select `git status`'s cwd, so a
    // truthy-but-bogus root bypasses the not-a-repo check while `git status`
    // itself still runs (and fails) against this non-repo cwd — the failure
    // branch `pickers/open-git-picker!` must not fold into "clean".
    let sandbox = CwdSandbox::new();
    let tmp = safe_tempdir();
    let extra = r#"(define-command! "test-git-status-fails" "" (lambda ()
                     (call! "pickers/git-picker-with" "/nonexistent-root")))"#;
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", extra);
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.state.status_msg = None;
    call(&mut ed, "test-git-status-fails");
    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    let msg = ed
        .state
        .status_msg
        .clone()
        .expect("error must surface as a status message");
    assert!(
        !msg.contains("clean"),
        "a failed `git status` must not be reported as a clean working tree; got: {msg}"
    );
    assert!(
        msg.contains("fatal") || msg.contains("git repository"),
        "the log message must carry git's own diagnostic (stderr), not just a \
         generic failure line; got: {msg}"
    );
    assert!(ed.state.config.picker.is_none());
}

#[test]
fn git_modified_picker_esc_dismisses_cleanly() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::write(sandbox.raw().join("a.txt"), "").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();
    let starting_bid = ed.focused_buffer_id();

    ed.feed_key(key('g'));
    ed.feed_key(key('m'));
    assert!(ed.state.config.picker.is_some());
    assert_eq!(
        ed.state.config.async_jobs.len(),
        1,
        "the `git status` job must be tracked while the picker is open — \
         this is the fail oracle for `(cancel-async! job-id)`: delete that \
         call from plugin.scm and this job leaks past Esc below"
    );

    ed.feed_key(key_esc());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    assert!(
        ed.state.config.async_jobs.is_empty(),
        "dismissing the picker must cancel the outstanding `git status` job, \
         not leave it running to completion for nothing"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "cancelling must not raise — a `job-id` still `#f` when `on-select` \
         fires would surface as a status message here"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        starting_bid,
        "Esc must not switch buffers"
    );

    // Keep interacting past the terminal action.
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    let text = ed.state.buffers.get(starting_bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after Esc must behave as plain input"
    );
}

// ── Buffers picker ────────────────────────────────────────────────────────────

#[test]
fn buffers_picker_lists_switches_and_disambiguates() {
    let guard = HumeRuntimeGuard::new();
    let sandbox = CwdSandbox::new();
    std::fs::create_dir_all(sandbox.raw().join("a")).unwrap();
    std::fs::create_dir_all(sandbox.raw().join("b")).unwrap();
    std::fs::write(sandbox.raw().join("a/mod.rs"), "").unwrap();
    std::fs::write(sandbox.raw().join("b/mod.rs"), "").unwrap();

    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();
    type_cmd(&mut ed, ":e a/mod.rs");
    type_cmd(&mut ed, ":e b/mod.rs");

    ed.feed_key(key('g'));
    ed.feed_key(key('b'));
    let picker = ed.state.config.picker.as_ref().expect("picker open");
    assert_eq!(picker.total_len(), 3);
    let rows: Vec<&str> = picker.window(10).collect();
    assert!(rows.iter().any(|r| r.ends_with("a/mod.rs")));
    assert!(rows.iter().any(|r| r.ends_with("b/mod.rs")));
    assert_eq!(
        rows.iter().filter(|r| r.ends_with("mod.rs")).count(),
        2,
        "both mod.rs buffers must be distinct rows, not collapsed to a bare basename; got {rows:?}"
    );

    for ch in "a/mod".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_enter());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert!(path.ends_with("a/mod.rs"), "got {path:?}");
}

#[test]
fn buffers_picker_esc_is_a_no_op() {
    let guard = HumeRuntimeGuard::new();
    let tmp = safe_tempdir();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    let starting_bid = ed.focused_buffer_id();

    ed.feed_key(key('g'));
    ed.feed_key(key('b'));
    assert!(ed.state.config.picker.is_some());

    ed.feed_key(key_esc());
    ed.settle();

    assert!(ed.state.config.picker.is_none());
    assert_eq!(ed.focused_buffer_id(), starting_bid);

    // Keep interacting past the terminal action.
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    let text = ed.state.buffers.get(starting_bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after Esc must behave as plain input"
    );
}
