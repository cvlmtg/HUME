use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ── FileMeta ──────────────────────────────────────────────────────────────────

/// Metadata captured from a file on open, restored when saving atomically.
///
/// Keeping this separate from the `Editor` struct means the I/O layer owns
/// everything it needs to do a faithful round-trip: the real write target
/// (symlink-resolved), permissions, and ownership.
pub(crate) struct FileMeta {
    /// The canonical path after following all symlinks.
    ///
    /// Writes always target this path so the symlink itself is preserved —
    /// `rename(2)` replaces inodes, not symlink targets.
    pub resolved_path: PathBuf,

    /// Original permission bits. Restored on the temp file before the rename
    /// so the file is never transiently exposed with wrong permissions.
    pub permissions: fs::Permissions,

    /// Original owner UID. Restored with `fchown` (best-effort, Unix only).
    #[cfg(unix)]
    pub uid: u32,

    /// Original group GID. Restored with `fchown` (best-effort, Unix only).
    #[cfg(unix)]
    pub gid: u32,
}

// ── read_file_meta ────────────────────────────────────────────────────────────

/// Capture metadata for an existing file without reading its content.
///
/// Used when saving over an existing file: we need the permissions and
/// ownership to preserve them, but not the content itself.
pub(crate) fn read_file_meta(path: &Path) -> io::Result<FileMeta> {
    let resolved = fs::canonicalize(path)?;
    let metadata = fs::metadata(&resolved)?;

    #[cfg(unix)]
    let meta = {
        use std::os::unix::fs::MetadataExt;
        FileMeta {
            resolved_path: resolved,
            permissions: metadata.permissions(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        }
    };

    #[cfg(not(unix))]
    let meta = FileMeta {
        resolved_path: resolved,
        permissions: metadata.permissions(),
    };

    Ok(meta)
}

// ── read_file ─────────────────────────────────────────────────────────────────

/// Read a file from disk, resolving symlinks and capturing metadata.
///
/// Returns `(content, meta)` where:
/// - `content` is the raw file text (CRLF normalization happens in `Buffer::from`)
/// - `meta` carries the resolved path, permissions, and ownership for write-back
pub(crate) fn read_file(path: &Path) -> io::Result<(String, FileMeta)> {
    let meta = read_file_meta(path)?;
    let content = fs::read_to_string(&meta.resolved_path)?;
    Ok((content, meta))
}

// ── write_file_atomic ─────────────────────────────────────────────────────────

/// Write `content` atomically to the path recorded in `meta`.
///
/// Strategy:
/// 1. Create a temp file **in the same directory** as the target — required for
///    `rename(2)` to stay on the same filesystem.
/// 2. Write content.
/// 3. Restore permissions **before** the rename — the file must never be
///    transiently visible with wrong mode bits.
/// 4. Restore ownership via `fchown` (Unix only, best-effort — only succeeds
///    when running as root or as the file's owner).
/// 5. Rename onto the target.
///
/// **Atomicity:** on POSIX (macOS, Linux) `rename(2)` is a single syscall —
/// the target either has the old content or the new content, never a partial
/// write. On Windows, `tempfile::persist` uses `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`,
/// which is not crash-atomic for file replacement (no equivalent of POSIX
/// `rename` exists on Windows without the deprecated transactional NTFS).
/// This is the best available option on Windows.
pub(crate) fn write_file_atomic(content: &str, meta: &FileMeta) -> io::Result<()> {
    let target = &meta.resolved_path;
    let dir = target.parent().unwrap_or(Path::new("."));

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut tmp, content.as_bytes())?;

    // Set permissions before rename — the window with wrong perms is zero.
    tmp.as_file().set_permissions(meta.permissions.clone())?;

    // Restore ownership. fchown requires root or matching uid to succeed;
    // we silently ignore errors so a non-privileged user can still save their
    // own files even if the group-change portion is rejected.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        use nix::unistd::{fchown, Gid, Uid};
        // Best-effort: succeeds only as root or matching uid; ignore errors so
        // a non-privileged user can still save their own files.
        let _ = fchown(
            tmp.as_file().as_raw_fd(),
            Some(Uid::from_raw(meta.uid)),
            Some(Gid::from_raw(meta.gid)),
        );
    }

    tmp.persist(target).map_err(|e| e.error)?;
    Ok(())
}

// ── write_file_new ────────────────────────────────────────────────────────────

/// Write `content` to a **new** file at `path`, creating it with default
/// permissions (0o644 on Unix, inherited from the temp file on Windows).
///
/// Uses the same temp-file + rename strategy as [`write_file_atomic`] so the
/// file is never partially visible even for a new path.
///
/// Returns the `FileMeta` for the newly created file, suitable for storing on
/// the `Editor` so that subsequent `:w` (no argument) targets the same path.
pub(crate) fn write_file_new(content: &str, path: &Path) -> io::Result<FileMeta> {
    let dir = path.parent().unwrap_or(Path::new("."));

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut tmp, content.as_bytes())?;

    // Set 0o644 (rw-r--r--) before rename — safe default for a new file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }

    tmp.persist(path).map_err(|e| e.error)?;

    // Read back the metadata now that the file exists on disk.
    read_file_meta(path)
}
