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
    /// `typed` made absolute against `cwd` as [`super::absolute()`] does, and
    /// failing as it does.
    pub fn from_typed(typed: &Path, cwd: &AbsolutePath) -> io::Result<Self> {
        super::absolute(typed, cwd).map(Self::stripped)
    }

    /// The process working directory, as [`std::env::current_dir`] returns
    /// it (on Unix with every symlink resolved, as `getcwd` reports it).
    /// Fails with `InvalidData` if that is not absolute.
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

    /// The file this path names, with every symlink resolved: its canonical
    /// form when it exists. A dangling symlink is followed to the missing
    /// path it names, and a missing path is its longest existing ancestor,
    /// canonicalized, joined with the missing components, so a file not on
    /// disk yet resolves to the path a write through it creates.
    ///
    /// Fails with `NotFound` when a `..` follows a missing directory, and with
    /// the filesystem's error for anything else that stops the walk: a symlink
    /// cycle, a permission error, a file where a directory should be.
    pub fn resolve(&self) -> io::Result<Self> {
        match Self::canonicalize(&self.0) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            resolved => return resolved,
        }
        let target = dangling_target(&self.0)?;
        let mut missing = Vec::new();
        let mut path = target.as_path();
        loop {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{}: no such directory", path.display()),
                ));
            };
            missing.push(name);
            match Self::canonicalize(parent) {
                Ok(ancestor) => {
                    let mut resolved = ancestor.0;
                    resolved.extend(missing.iter().rev());
                    return Ok(Self(resolved));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => path = parent,
                Err(e) => return Err(e),
            }
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

/// The path at the end of the chain of symlinks `path` starts, read without
/// resolving anything else: `path` itself when it is not a symlink.
fn dangling_target(path: &Path) -> io::Result<PathBuf> {
    const MAX_HOPS: u32 = 40;
    let mut current = path.to_path_buf();
    for _ in 0..MAX_HOPS {
        match std::fs::symlink_metadata(&current) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(e),
            Ok(meta) if !meta.is_symlink() => return Ok(current),
            Ok(_) => {
                let link = std::fs::read_link(&current)?;
                current = match current.parent() {
                    Some(parent) => parent.join(link),
                    None => link,
                };
            }
        }
    }
    Err(io::Error::other(format!(
        "{}: too many levels of symbolic links",
        path.display()
    )))
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
