//! A file HUME writes for its own later reading: an 8-byte magic, a
//! little-endian `u32` format version, then a body. A file of another
//! format version reads as absent, so a format change needs no migration.
//! Each file is readable by its owner only and replaced in one rename.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;

/// What identifies one kind of stamped file.
pub(in crate::editor) struct Stamp {
    pub(in crate::editor) magic: &'static [u8; 8],
    pub(in crate::editor) format: u32,
    /// The kind with its article, as in "not {kind}".
    pub(in crate::editor) kind: &'static str,
}

const HEADER_LEN: u64 = 8 + size_of::<u32>() as u64;

impl Stamp {
    /// The header every file of this kind starts with, for the body to be
    /// appended to.
    pub(in crate::editor) fn header(&self) -> Vec<u8> {
        let mut bytes = self.magic.to_vec();
        bytes.extend_from_slice(&self.format.to_le_bytes());
        bytes
    }

    /// Open the file at `path` past its header. `None` when there is no
    /// file or it holds another format version. An error names `path`.
    pub(in crate::editor) fn open(&self, path: &Path) -> Result<Option<File>, String> {
        let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let mut file = match hume_platform::io::open_own_file(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(fail(&e)),
        };
        let mut header = Vec::new();
        file.by_ref()
            .take(HEADER_LEN)
            .read_to_end(&mut header)
            .map_err(|e| fail(&e))?;
        let Some(rest) = header.strip_prefix(self.magic.as_slice()) else {
            return Err(fail(&format_args!("not {}", self.kind)));
        };
        let Ok(format) = <[u8; 4]>::try_from(rest) else {
            return Err(fail(&"the file ends inside its header"));
        };
        Ok((u32::from_le_bytes(format) == self.format).then_some(file))
    }
}

/// Write `bytes` to `path`, creating its directory.
pub(in crate::editor) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let write = || {
        if let Some(parent) = path.parent() {
            hume_platform::io::create_private_dir_all(parent)?;
        }
        hume_platform::io::write_private_file(bytes, path)
    };
    write().map_err(|e| format!("{}: {e}", path.display()))
}

/// Decode `bytes` as one postcard value that takes all of them.
pub(in crate::editor) fn decode_whole<'de, T: Deserialize<'de>>(
    bytes: &'de [u8],
) -> Result<T, String> {
    match postcard::take_from_bytes::<T>(bytes) {
        Ok((value, [])) => Ok(value),
        Ok(_) => Err("trailing bytes after the body".to_owned()),
        Err(e) => Err(e.to_string()),
    }
}
