use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

use hume_editing::text::BufferText;
use hume_platform::io::FileMeta;
use hume_platform::path::AbsolutePath;

/// The file behind a file-backed buffer: the path the user named it by and
/// the state of the file on disk as of its last read or write. The typed
/// path keeps its symlinks: every read and write goes through it, so a
/// symlink retargeted since reaches its new target, language detection reads
/// it first, and the buffer's name and display form are derived from it. The
/// resolved path, with every symlink resolved as of that read or write, is
/// the buffer's identity, stored once, in [`Backing`]. A `BufferFile` never
/// changes: a write or reload builds a new one.
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

    /// Reads the file at `typed`: its text, and the file the read resolved.
    pub(in crate::editor) fn read(typed: AbsolutePath) -> io::Result<(BufferText, Self)> {
        let (content, meta) = hume_platform::io::read_file(&typed)?;
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

    /// Whether `fresh`, the file now at the typed path, is the file this
    /// describes, unchanged: the same resolved path and signature as at the
    /// last read or write. Never for a file not on disk yet, which has none.
    pub(in crate::editor) fn describes(&self, fresh: &FileMeta) -> bool {
        self.file_meta().is_some_and(|meta| {
            meta.resolved_path() == fresh.resolved_path() && meta.signature() == fresh.signature()
        })
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
