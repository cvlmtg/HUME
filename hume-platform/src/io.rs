//! Atomic file I/O with permission and ownership preservation.
//!
//! The key primitive is [`write_file_atomic`]: write to a sibling temp file,
//! restore the original file's permissions and ownership, then `rename(2)` it
//! into place. The caller always sees either the old content or the new content
//! — never a partial write. [`FileMeta`] bundles the metadata captured on open
//! so it can be faithfully restored on save.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

// ── FileSignature ─────────────────────────────────────────────────────────────

/// A cheap fingerprint of a file's on-disk state, used to detect external
/// changes without reading or hashing content.
///
/// Compares mtime **and** size: some filesystems (HFS+, FAT) only report
/// mtime to one-second resolution, so a same-second rewrite can be invisible
/// to mtime alone. Equality (`!=`), not ordering — restoring a file from a
/// backup moves mtime *backwards* and must still count as a change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileSignature {
    /// `None` on platforms/filesystems that don't report a modification
    /// time — never conjured into a fake value, since that could compare
    /// equal to a genuinely different unset case.
    mtime: Option<SystemTime>,
    size: u64,
}

impl FileSignature {
    /// Extracts the mtime+size fingerprint from an already-fetched
    /// `fs::Metadata` — shared by [`read_signature`] and [`read_file_meta`]
    /// so the fingerprint rule stays defined in one place.
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        FileSignature {
            mtime: metadata.modified().ok(),
            size: metadata.len(),
        }
    }
}

/// Read a file's current [`FileSignature`] without touching its content.
///
/// No `canonicalize` — callers already hold the resolved path (from an
/// earlier `read_file_meta`/`read_file`).
pub fn read_signature(path: &Path) -> io::Result<FileSignature> {
    Ok(FileSignature::from_metadata(&fs::metadata(path)?))
}

// ── FileMeta ──────────────────────────────────────────────────────────────────

/// Metadata captured from a file on open, restored when saving atomically.
///
/// Bundles everything the I/O layer needs for a faithful round-trip: the real
/// write target (symlink-resolved), permissions, and ownership. Constructed
/// only by [`read_file_meta`] and [`read_file`]; never built directly.
pub struct FileMeta {
    /// The canonical path after following all symlinks.
    ///
    /// Writes always target this path so the symlink itself is preserved —
    /// `rename(2)` replaces inodes, not symlink targets.
    ///
    /// Private: always produced by `canonicalize` inside `read_file_meta` /
    /// `read_file`. Constructing a `FileMeta` with an unresolved path would
    /// silently break the atomic-write-to-symlink-target guarantee.
    resolved_path: PathBuf,

    /// Original permission bits. Restored on the temp file before the rename
    /// so the file is never transiently exposed with wrong permissions.
    permissions: fs::Permissions,

    /// Original owner UID. Restored with `fchown` (best-effort, Unix only).
    #[cfg(unix)]
    uid: u32,

    /// Original group GID. Restored with `fchown` (best-effort, Unix only).
    #[cfg(unix)]
    gid: u32,

    /// The file's fingerprint as of the last read or write through this
    /// `FileMeta`. Compared against a fresh [`read_signature`] to detect
    /// external changes; refreshed by [`write_file_atomic`] after a
    /// successful write so the editor's own saves never look external.
    signature: FileSignature,
}

impl FileMeta {
    /// The canonical path after following all symlinks.
    ///
    /// This is the target for atomic writes — using it ensures the symlink
    /// itself is preserved while the content behind it is updated.
    pub fn resolved_path(&self) -> &Path {
        &self.resolved_path
    }

    /// The file's fingerprint as of the last read or write.
    pub fn signature(&self) -> FileSignature {
        self.signature
    }
}

// ── read_file_meta ────────────────────────────────────────────────────────────

/// Capture metadata for an existing file without reading its content.
///
/// Used when saving over an existing file: we need the permissions and
/// ownership to preserve them, but not the content itself.
pub fn read_file_meta(path: &Path) -> io::Result<FileMeta> {
    let resolved = fs::canonicalize(path)?;
    let metadata = fs::metadata(&resolved)?;
    let signature = FileSignature::from_metadata(&metadata);

    #[cfg(unix)]
    let meta = {
        use std::os::unix::fs::MetadataExt;
        FileMeta {
            resolved_path: resolved,
            permissions: metadata.permissions(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            signature,
        }
    };

    #[cfg(not(unix))]
    let meta = FileMeta {
        resolved_path: resolved,
        permissions: metadata.permissions(),
        signature,
    };

    Ok(meta)
}

// ── read_file ─────────────────────────────────────────────────────────────────

/// Read a file from disk, resolving symlinks and capturing metadata.
///
/// Returns `(content, meta)` where:
/// - `content` is the raw file text (CRLF normalization happens in `Buffer::from`)
/// - `meta` carries the resolved path, permissions, ownership, and fingerprint
///   for write-back and external-change detection
///
/// The stat backing `meta.signature` happens in `read_file_meta`, before the
/// content read below. If a writer races us here, storing the *older*
/// signature means a later disk-change check reports a change instead of
/// silently missing one — biased toward a spurious check, never toward a
/// miss.
pub fn read_file(path: &Path) -> io::Result<(String, FileMeta)> {
    let meta = read_file_meta(path)?;
    let content = fs::read_to_string(&meta.resolved_path)?;
    Ok((content, meta))
}

// ── write_file_atomic ─────────────────────────────────────────────────────────

/// Write `content` atomically to the path recorded in `meta`. Returns `true`
/// when the chmod-retry path below was taken, `false` on a plain successful
/// write.
///
/// The temp file is created in the target's own directory, so `rename(2)`
/// stays on one filesystem; permissions are restored *before* the rename so
/// the file is never transiently visible with the wrong mode bits. Ownership
/// (`fchown`, Unix) is best-effort — it only succeeds as root or as the owner.
///
/// `force` retries once after a `PermissionDenied` rename by clearing the
/// target's readonly attribute. The rename unlinks the old, transiently
/// writable inode, so nothing needs restoring on the new one — it already
/// carries `meta.permissions`.
///
/// **Atomicity:** guaranteed on POSIX, where `rename(2)` is one syscall. On
/// Windows `tempfile::persist` uses `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`,
/// which is not crash-atomic for file replacement — the best available
/// option without the deprecated transactional NTFS.
///
/// On success, re-stats the target to refresh `meta.signature`: the rename
/// swaps in a fresh inode with a new mtime, so without this refresh the
/// editor's own save looks like an external change on the very next
/// disk-state check. `meta` is `&mut` so the refresh can't be forgotten at a
/// call site.
///
/// A failed re-stat does *not* fail the write — the content is already
/// durable, and surfacing an error here would report a successful save as
/// failed (buffer never marked saved, `:q` keeps refusing, no `didSave`
/// fires) while the new content sits on disk regardless. A stale signature
/// only biases the next disk-state check toward a spurious "changed", never
/// toward missing a real one — the same bias `read_file`'s doc accepts for
/// the read-side race.
pub fn write_file_atomic(content: &str, meta: &mut FileMeta, force: bool) -> io::Result<bool> {
    let target = &meta.resolved_path;
    let dir = target.parent().unwrap_or(Path::new("."));

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut tmp, content.as_bytes())?;

    // Set permissions before rename — the window with wrong perms is zero.
    tmp.as_file().set_permissions(meta.permissions.clone())?;

    // fchown requires root or matching uid to succeed; ignore errors so a
    // non-privileged user can still save their own files even if the
    // group-change portion is rejected.
    #[cfg(unix)]
    {
        use nix::unistd::{Gid, Uid, fchown};
        let _ = fchown(
            tmp.as_file(),
            Some(Uid::from_raw(meta.uid)),
            Some(Gid::from_raw(meta.gid)),
        );
    }

    // Note: on POSIX, rename(2) ignores the target file's permission bits when
    // the containing directory is writable, so this PermissionDenied branch is
    // primarily reached on Windows (READONLY attribute) and on exotic POSIX
    // filesystems / ACL setups — it is genuinely hard to exercise from a
    // unit test on macOS/Linux without root or chflags.
    let result = match tmp.persist(target) {
        Ok(_) => Ok(false),
        Err(persist_err)
            if force && persist_err.error.kind() == io::ErrorKind::PermissionDenied =>
        {
            // Target is readonly; make it writable just long enough for the
            // rename. After rename(2) the old inode (transiently writable) is
            // unlinked — the new inode already carries meta.permissions, so
            // `set_readonly(false)` on a clone of meta.permissions is enough
            // (the value we pass is overwritten by the rename anyway).
            // Using the cross-platform API is deliberate; PermissionsExt is Unix-only.
            let mut perms = meta.permissions.clone();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(target, perms)?;
            match persist_err.file.persist(target) {
                Ok(_) => Ok(true),
                Err(retry_err) => {
                    // A failed `:w!` must not leave the target more permissive
                    // than it was. Best-effort restore — if this also fails,
                    // surface the original retry error rather than masking it.
                    let _ = fs::set_permissions(target, meta.permissions.clone());
                    Err(retry_err.error)
                }
            }
        }
        Err(persist_err) => Err(persist_err.error),
    };

    if result.is_ok()
        && let Ok(sig) = read_signature(&meta.resolved_path)
    {
        meta.signature = sig;
    }
    result
}

// ── write_file_new ────────────────────────────────────────────────────────────

/// Follows a chain of symlinks lexically until reaching a path with nothing
/// backing it — the write target for [`write_file_new`].
///
/// Not `canonicalize`: that requires every component (including the final
/// one) to exist, which is exactly false for a dangling symlink's target.
/// A `path` that isn't a symlink at all resolves in one `symlink_metadata`
/// call. Bounded to `SYMLOOP_MAX` (Linux's own limit) hops so a symlink cycle
/// errors instead of looping forever.
fn resolve_symlink_target(path: &Path) -> io::Result<PathBuf> {
    const MAX_HOPS: u32 = 40;
    let mut current = path.to_path_buf();
    for _ in 0..MAX_HOPS {
        match fs::symlink_metadata(&current) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(e),
            Ok(meta) if !meta.is_symlink() => return Ok(current),
            Ok(_) => {
                let link_target = fs::read_link(&current)?;
                current = if link_target.is_absolute() {
                    link_target
                } else {
                    current.parent().unwrap_or(Path::new(".")).join(link_target)
                };
            }
        }
    }
    Err(io::Error::other("too many levels of symbolic links"))
}

/// Write `content` to a **new** file at `path`, creating it with default
/// permissions (0o644 on Unix, inherited from the temp file on Windows).
///
/// Uses the same temp-file + rename strategy as [`write_file_atomic`] so the
/// file is never partially visible even for a new path.
///
/// If `path` is a dangling symlink, writes through it to the link's target
/// instead of replacing the link itself with a regular file — matching
/// [`FileMeta::resolved_path`]'s guarantee for the existing-file case (see
/// its doc). `path` may also point through a chain of missing intermediate
/// directories in the *target's* path; that still surfaces as the same I/O
/// error `tempfile::NamedTempFile::new_in` would give for a plain missing
/// parent.
///
/// Returns the `FileMeta` for the newly created file, suitable for storing on
/// the `Editor` so that subsequent `:w` (no argument) targets the same path.
pub fn write_file_new(content: &str, path: &Path) -> io::Result<FileMeta> {
    let target = resolve_symlink_target(path)?;
    let dir = target.parent().unwrap_or(Path::new("."));

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut tmp, content.as_bytes())?;

    // Set 0o644 (rw-r--r--) before rename — safe default for a new file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }

    tmp.persist(&target).map_err(|e| e.error)?;

    // Read back the metadata now that the file exists on disk.
    read_file_meta(&target)
}

#[cfg(test)]
mod tests;
