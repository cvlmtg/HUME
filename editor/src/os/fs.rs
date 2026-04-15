//! Thin wrappers around `std::fs` primitives.
//!
//! Every filesystem syscall in the `editor` crate (outside `os::io`) must go
//! through one of these functions so that `editor/src/os/` is the sole audit
//! surface for file I/O. Each wrapper is a direct one-line delegation to
//! `std::fs`; the value is the allow-list, not any added behavior.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) fn canonicalize(p: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(p)
}

pub(crate) fn exists(p: &Path) -> bool {
    p.exists()
}

pub(crate) fn metadata(p: &Path) -> io::Result<fs::Metadata> {
    fs::metadata(p)
}

pub(crate) fn read_dir(p: &Path) -> io::Result<fs::ReadDir> {
    fs::read_dir(p)
}

pub(crate) fn create_dir_all(p: &Path) -> io::Result<()> {
    fs::create_dir_all(p)
}

pub(crate) fn remove_dir_all(p: &Path) -> io::Result<()> {
    fs::remove_dir_all(p)
}

pub(crate) fn read_to_string(p: &Path) -> io::Result<String> {
    fs::read_to_string(p)
}
