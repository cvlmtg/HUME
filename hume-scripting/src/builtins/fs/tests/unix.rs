//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;
use hume_core::Severity;
use std::os::unix::fs::PermissionsExt;

/// Makes `dir` deletable again when the test ends, so the temp dir cleans up.
struct Unlock(PathBuf);

impl Drop for Unlock {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, PermissionsExt::from_mode(0o755));
    }
}

/// Fills `locked` with a file, then makes it read-only, so `remove_dir_all`
/// cannot empty it. `false` when the process can write there anyway (root).
fn lock_dir(locked: &Path) -> bool {
    std::fs::create_dir_all(locked).unwrap();
    std::fs::write(locked.join("file"), "").unwrap();
    std::fs::set_permissions(locked, PermissionsExt::from_mode(0o555)).unwrap();
    std::fs::write(locked.join("probe"), "").is_err()
}

#[test]
fn replace_dir_succeeds_when_the_old_contents_cannot_be_deleted() {
    let tmp = TempDir::new().unwrap();
    let (src, parent) = (tmp.path().join("stage/repo"), tmp.path().join("plugins"));
    let dest = parent.join("repo");
    dir_with(&src, "new");
    dir_with(&dest, "old");
    let _unlock = Unlock(parent.join("repo.replaced/locked"));
    if !lock_dir(&dest.join("locked")) {
        return;
    }

    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    replace_dir(
        &mut ctx,
        AbsolutePath::rooted(&src),
        AbsolutePath::rooted(&dest),
    )
    .unwrap();
    drop(ctx);

    assert_eq!(entries(&dest), ["new"]);
    let (level, message) = h.pending_messages.last().expect("a warning is logged");
    assert_eq!(*level, Severity::Warning);
    assert!(
        message.contains("repo.replaced"),
        "the warning names the leftover backup: {message}"
    );
}

// ── path containment ─────────────────────────────────────────────────────

#[test]
fn first_path_outside_is_false_for_a_relative_link_that_stays_inside() {
    let tmp = server_tree();
    std::os::unix::fs::symlink("../lib/x", tmp.path().join("srv/bin/x")).unwrap();
    assert_eq!(first_outside(&tmp, "srv", &["srv/bin/x"]), Ok(None));
}

#[test]
fn first_path_outside_returns_an_absolute_link_to_a_file_outside() {
    let tmp = server_tree();
    std::os::unix::fs::symlink(tmp.path().join("outside"), tmp.path().join("srv/bin/x")).unwrap();
    assert_eq!(
        first_outside(&tmp, "srv", &["srv/bin/x"]),
        Ok(Some(under(&tmp, "srv/bin/x")))
    );
}

#[test]
fn first_path_outside_returns_a_dangling_link_to_a_path_outside() {
    let tmp = server_tree();
    std::os::unix::fs::symlink("../../missing", tmp.path().join("srv/bin/x")).unwrap();
    assert_eq!(
        first_outside(&tmp, "srv", &["srv/bin/x"]),
        Ok(Some(under(&tmp, "srv/bin/x")))
    );
}

#[test]
fn first_path_outside_raises_for_a_symlink_cycle_naming_the_path() {
    let tmp = server_tree();
    std::os::unix::fs::symlink("b", tmp.path().join("srv/bin/a")).unwrap();
    std::os::unix::fs::symlink("a", tmp.path().join("srv/bin/b")).unwrap();
    let err = first_outside(&tmp, "srv", &["srv/bin/a"]).unwrap_err();
    assert!(
        err.contains("first-path-outside: ") && err.contains("srv/bin/a"),
        "the error names the builtin and the path: {err}"
    );
}
