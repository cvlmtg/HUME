//! Unix test helpers for paths, shared with consumer crates' tests
//! through the `test-util` feature.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Writes a `sh` script named `name` into `dir` with permission bits
/// `mode`, and returns its path.
pub fn write_program(dir: &Path, name: &str, mode: u32) -> PathBuf {
    write_script(dir, name, "", mode)
}

/// [`write_program`] with `body` after the `#!/bin/sh` line.
pub fn write_script(dir: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("program written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
        .expect("program mode set");
    path
}
