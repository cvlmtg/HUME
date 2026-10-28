//! Directory trees that take longer to delete than a script budget, for
//! tests that check a delete is left out of that budget.

use std::fmt::Debug;
use std::path::Path;
use std::time::{Duration, Instant};

const FILES_PER_DIR: usize = 100;
const FIRST_DIRS: usize = 200;
const MAX_DIRS: usize = 6_400;

/// Runs `delete` on ever larger directory trees until one call takes longer
/// than `budget`, checking each call succeeds and removes its tree.
///
/// A call that succeeds after running past its budget shows the delete was
/// not counted against it. Deleting 20,000 files takes about 1 s on macOS
/// and under 250 ms on a Linux CI runner, so the tree doubles from that
/// size instead of fixing one.
///
/// # Panics
/// When a call fails or leaves its tree behind, or when a 640,000-file tree
/// still deletes within `budget`.
pub fn assert_delete_outlasts_budget<T: Debug, E: Debug>(
    budget: Duration,
    mut delete: impl FnMut(&Path) -> Result<T, E>,
) {
    let tmp = tempfile::tempdir().expect("a temp dir for the tree");
    let root = tmp.path().join("tree");
    let mut dirs = FIRST_DIRS;
    loop {
        for d in 0..dirs {
            let dir = root.join(format!("d{d}"));
            std::fs::create_dir_all(&dir).expect("create a tree dir");
            for f in 0..FILES_PER_DIR {
                std::fs::write(dir.join(format!("f{f}")), "x").expect("create a tree file");
            }
        }

        let started = Instant::now();
        let result = delete(&root);
        let elapsed = started.elapsed();

        assert!(result.is_ok(), "{result:?}");
        assert!(!root.exists(), "the tree must be gone");
        if elapsed > budget {
            return;
        }
        assert!(
            dirs < MAX_DIRS,
            "the delete must outlast the budget for the call to show it was not counted"
        );
        dirs *= 2;
    }
}
