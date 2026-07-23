use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

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

/// Drains async sources (and their queued Steel callbacks) in a bounded loop
/// until `until` returns true — CI scheduling jitter can't flake this.
fn drain_until(ed: &mut Editor, mut until: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        ed.drain_async_sources();
        ed.drain_pending_steel_calls();
        if until(ed) {
            return;
        }
        assert!(Instant::now() < deadline, "condition never became true");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Load the real `core:pickers` plugin, plus `extra_source` appended to the
/// same init.scm (evaluated after the plugin, so `call!`-dispatchable
/// commands the plugin registered are reachable by name — see the module
/// doc comment on why plain, non-command helpers are not used this way).
fn setup(guard: &HumeRuntimeGuard, tmp: &Path, input: &str, extra_source: &str) -> Editor {
    write_core_plugin(guard, "pickers", PICKERS_PLUGIN);
    let mut ed = editor_from(input);
    let mut host = ScriptingHost::new();
    let source = format!("(load-plugin \"core:pickers\")\n{extra_source}");
    eval_with_real_host(&mut ed, &mut host, &source, tmp);
    ed.scripting = Some(host);
    ed
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
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

    let tmp = tempfile::tempdir().unwrap();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();

    ed.feed_key(key('g'));
    ed.feed_key(key('f'));
    drain_until(&mut ed, |ed| {
        ed.state.picker.as_ref().map(|p| p.total_len()).unwrap_or(0) == 3
    });

    let picker = ed.state.picker.as_ref().expect("picker open");
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
    ed.drain_pending_steel_calls();

    assert!(ed.state.picker.is_none());
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

    let tmp = tempfile::tempdir().unwrap();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();
    let starting_bid = ed.focused_buffer_id();

    ed.feed_key(key('g'));
    ed.feed_key(key('f'));
    drain_until(&mut ed, |ed| {
        ed.state.picker.as_ref().map(|p| p.total_len()).unwrap_or(0) >= 1
    });

    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();

    assert!(ed.state.picker.is_none());
    assert_eq!(
        ed.focused_buffer_id(),
        starting_bid,
        "Esc must not switch buffers"
    );

    // LESSONS.md L4: keep interacting past the terminal action.
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
    let tmp = tempfile::tempdir().unwrap();

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
    drain_until(&mut ed, |ed| {
        ed.state.picker.as_ref().map(|p| p.total_len()).unwrap_or(0) == 2
    });
    let picker = ed.state.picker.as_ref().expect("picker open");
    assert_eq!(
        picker.window(10).collect::<Vec<_>>(),
        vec!["one.txt", "two.txt"]
    );

    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert!(path.ends_with("one.txt"), "got {path:?}");
}

#[test]
fn files_picker_error_path_names_fd() {
    let guard = HumeRuntimeGuard::new();
    let tmp = tempfile::tempdir().unwrap();
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
    assert!(ed.state.picker.is_none());
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

    let tmp = tempfile::tempdir().unwrap();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    ed.set_cwd(&sandbox.path()).unwrap();
    type_cmd(&mut ed, ":e a/mod.rs");
    type_cmd(&mut ed, ":e b/mod.rs");

    ed.feed_key(key('g'));
    ed.feed_key(key('b'));
    let picker = ed.state.picker.as_ref().expect("picker open");
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
    ed.drain_pending_steel_calls();

    assert!(ed.state.picker.is_none());
    let bid = ed.focused_buffer_id();
    let path = ed.state.buffers.get(bid).path().expect("buffer has a path");
    assert!(path.ends_with("a/mod.rs"), "got {path:?}");
}

#[test]
fn buffers_picker_esc_is_a_no_op() {
    let guard = HumeRuntimeGuard::new();
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = setup(&guard, tmp.path(), "-[h]>ello\n", "");
    let starting_bid = ed.focused_buffer_id();

    ed.feed_key(key('g'));
    ed.feed_key(key('b'));
    assert!(ed.state.picker.is_some());

    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();

    assert!(ed.state.picker.is_none());
    assert_eq!(ed.focused_buffer_id(), starting_bid);

    // LESSONS.md L4: keep interacting past the terminal action.
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    let text = ed.state.buffers.get(starting_bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after Esc must behave as plain input"
    );
}
