//! Paths that are absolute on this platform.

use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};

/// A path that is absolute on this platform. Minted only from a typed path
/// made absolute against a working directory, from a canonicalized path,
/// from the process working directory, or from another `AbsolutePath`.
///
/// It never carries a Windows verbatim drive prefix (`\\?\C:\`), which
/// `canonicalize` adds and many consumers mishandle; the standard library
/// adds one itself to a path whose length needs it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AbsolutePath(PathBuf);

impl AbsolutePath {
    /// `typed` joined onto `cwd` when relative and normalized lexically,
    /// symlinks kept (see [`super::absolute_unresolved`]).
    ///
    /// Fails with `InvalidInput` when `typed` is a Windows drive-relative path
    /// (`D:foo`): joined onto `cwd` it replaces it and stays relative.
    pub fn from_typed(typed: &Path, cwd: &AbsolutePath) -> io::Result<Self> {
        let path = super::absolute_unresolved(typed, cwd);
        if path.is_absolute() {
            Ok(Self::stripped(path))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{}: not an absolute path", path.display()),
            ))
        }
    }

    /// The process working directory, as [`std::env::current_dir`] returns
    /// it, symlinks kept. Fails with `InvalidData` if that is not absolute.
    pub fn current_dir() -> io::Result<Self> {
        let cwd = std::env::current_dir()?;
        if cwd.is_absolute() {
            Ok(Self::stripped(cwd))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: working directory is not absolute", cwd.display()),
            ))
        }
    }

    /// `path` with every symlink resolved, as [`std::fs::canonicalize`]
    /// returns it less a verbatim drive prefix.
    pub fn canonicalize(path: &Path) -> io::Result<Self> {
        std::fs::canonicalize(path).map(Self::stripped)
    }

    /// The canonical form of this path when it exists. When it does not, its
    /// canonical parent joined with its file name, which still resolves the
    /// symlinks in the parent chain (`/tmp` to `/private/tmp` on macOS), so a
    /// missing file keys the same as the file a later write creates there.
    /// When the parent does not exist either, the path itself.
    pub fn resolve_existing(&self) -> Self {
        if let Ok(canonical) = Self::canonicalize(&self.0) {
            return canonical;
        }
        match (self.0.parent(), self.0.file_name()) {
            (Some(parent), Some(name)) => Self::canonicalize(parent)
                .map(|parent| Self(parent.0.join(name)))
                .unwrap_or_else(|_| self.clone()),
            _ => self.clone(),
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// `path`, already absolute, less a verbatim drive prefix.
    fn stripped(path: PathBuf) -> Self {
        Self(super::strip_unc_prefix(path))
    }
}

#[cfg(any(test, feature = "test-util"))]
impl AbsolutePath {
    /// `path`, an absolute path or a root-anchored one such as `/tmp/a.rs`,
    /// as an absolute path. On Windows a root-anchored path with no drive is
    /// placed on drive `C:`.
    pub fn rooted(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let path = if cfg!(windows) && !path.is_absolute() {
            let mut on_drive = std::ffi::OsString::from("C:");
            on_drive.push(path);
            PathBuf::from(on_drive)
        } else {
            path.to_path_buf()
        };
        assert!(path.is_absolute(), "{path:?} is not root-anchored");
        Self::stripped(path)
    }
}

impl Deref for AbsolutePath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
mod tests;
