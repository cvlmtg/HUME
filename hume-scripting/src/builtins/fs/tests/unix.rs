//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;
use crate::log::LogLevel;
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
    assert_eq!(*level, LogLevel::Warning);
    assert!(
        message.contains("repo.replaced"),
        "the warning names the leftover backup: {message}"
    );
}
