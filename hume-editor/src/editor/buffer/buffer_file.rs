use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

use hume_editing::text::BufferText;
use hume_platform::io::FileMeta;
use hume_platform::path::AbsolutePath;

/// The file behind a file-backed buffer: the path the user named it by and
/// the state of the file on disk. The resolved path, with every symlink
/// resolved, is the buffer's identity and the file its writes reach; it is
/// stored once, in [`Backing`]. The typed path keeps its symlinks: language
/// detection reads it first, and the buffer's name and display form are
/// derived from it.
pub(in crate::editor) struct BufferFile {
    typed: AbsolutePath,
    /// `typed`'s file name.
    name: OsString,
    /// `typed` `~`-collapsed.
    display: String,
    backing: Backing,
}

/// Whether the file has been read or written yet.
enum Backing {
    /// Not on disk yet (`:e` on a missing path): a write creates it.
    /// `resolved` is [`AbsolutePath::resolve`]'s form of the typed path.
    New { resolved: AbsolutePath },
    /// Read or written; the metadata's resolved path is the identity.
    OnDisk(FileMeta),
}

/// What a write of the buffer's own file works from: the path to create a
/// new file at, or the metadata of the file to replace.
pub(in crate::editor) enum SaveTarget<'a> {
    New(&'a AbsolutePath),
    OnDisk(&'a mut FileMeta),
}

impl BufferFile {
    /// A file not on disk yet, to be created at `resolved`.
    pub(in crate::editor) fn new_file(
        resolved: AbsolutePath,
        typed: AbsolutePath,
    ) -> io::Result<Self> {
        Self::build(typed, Backing::New { resolved })
    }

    /// The file `meta` describes, named by the user as `typed`.
    pub(in crate::editor) fn on_disk(meta: FileMeta, typed: AbsolutePath) -> io::Result<Self> {
        Self::build(typed, Backing::OnDisk(meta))
    }

    /// Reads the file at `path`, named by the user as `typed`: its text, and
    /// the file the read resolved.
    pub(in crate::editor) fn read(
        typed: AbsolutePath,
        path: &AbsolutePath,
    ) -> io::Result<(BufferText, Self)> {
        let (content, meta) = hume_platform::io::read_file(path)?;
        Ok((
            BufferText::from(content.as_str()),
            Self::on_disk(meta, typed)?,
        ))
    }

    /// Fails with `InvalidInput` when either path has no file name (`/`,
    /// `C:\`): the buffer's name is the typed path's file name, and a
    /// nameless one would read as the scratch buffer's.
    fn build(typed: AbsolutePath, backing: Backing) -> io::Result<Self> {
        let nameless = |path: &AbsolutePath| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{}: path has no file name", path.display()),
            )
        };
        let resolved = backing.resolved();
        if resolved.file_name().is_none() {
            return Err(nameless(resolved));
        }
        let name = typed
            .file_name()
            .ok_or_else(|| nameless(&typed))?
            .to_owned();
        let display = hume_platform::path::display_form(&typed);
        Ok(Self {
            typed,
            name,
            display,
            backing,
        })
    }

    pub(in crate::editor) fn resolved(&self) -> &AbsolutePath {
        self.backing.resolved()
    }

    pub(in crate::editor) fn typed(&self) -> &AbsolutePath {
        &self.typed
    }

    /// The buffer's name: the file name of the path as the user named it.
    pub(in crate::editor) fn file_name(&self) -> &OsStr {
        &self.name
    }

    /// Print verbatim, no further transforms needed.
    pub(in crate::editor) fn display(&self) -> &str {
        &self.display
    }

    /// The file's metadata, `None` until it is first read or written.
    pub(in crate::editor) fn file_meta(&self) -> Option<&FileMeta> {
        match &self.backing {
            Backing::New { .. } => None,
            Backing::OnDisk(meta) => Some(meta),
        }
    }

    /// The only mutable access: a write refreshes the metadata of the file
    /// it replaces, which cannot change the resolved path.
    pub(in crate::editor) fn save_target(&mut self) -> SaveTarget<'_> {
        match &mut self.backing {
            Backing::New { resolved } => SaveTarget::New(resolved),
            Backing::OnDisk(meta) => SaveTarget::OnDisk(meta),
        }
    }
}

/// A file the user named: the typed path and the path that keys its buffer
/// ([`AbsolutePath::resolve`]), resolved once for both the dedup lookup and
/// a missing file's new buffer.
pub(in crate::editor) struct FileTarget {
    typed: AbsolutePath,
    resolved: AbsolutePath,
}

impl FileTarget {
    /// `path` as typed against `cwd` (see [`AbsolutePath::from_typed`]), and
    /// the file it names. Fails as either step fails.
    pub(in crate::editor) fn from_typed(path: &Path, cwd: &AbsolutePath) -> io::Result<Self> {
        let typed = AbsolutePath::from_typed(path, cwd)?;
        let resolved = typed.resolve()?;
        Ok(Self { typed, resolved })
    }

    pub(in crate::editor) fn typed(&self) -> &AbsolutePath {
        &self.typed
    }

    pub(in crate::editor) fn resolved(&self) -> &AbsolutePath {
        &self.resolved
    }

    /// The target as a file not on disk yet.
    pub(in crate::editor) fn into_new_file(self) -> io::Result<BufferFile> {
        BufferFile::new_file(self.resolved, self.typed)
    }
}

impl Backing {
    fn resolved(&self) -> &AbsolutePath {
        match self {
            Self::New { resolved } => resolved,
            Self::OnDisk(meta) => meta.resolved_path(),
        }
    }
}

#[cfg(test)]
impl BufferFile {
    /// A file not on disk yet whose typed and resolved paths are both `path`
    /// (see `AbsolutePath::rooted`).
    pub(in crate::editor) fn at(path: impl AsRef<std::path::Path>) -> Self {
        let path = AbsolutePath::rooted(path);
        Self::new_file(path.clone(), path).expect("a test path names a file")
    }

    /// The existing file at `path`, typed as `path` (see
    /// `AbsolutePath::rooted`).
    pub(in crate::editor) fn on_disk_at(path: impl AsRef<std::path::Path>) -> Self {
        let typed = AbsolutePath::rooted(path);
        let meta = hume_platform::io::read_file_meta(&typed).expect("the test file exists");
        Self::on_disk(meta, typed).expect("a test path names a file")
    }
}
