//! Thin wrappers around `std::fs` primitives.
//!
//! Every filesystem syscall in the workspace must go through one of these
//! functions, providing a single audit surface for file I/O. Each wrapper is a
//! direct one-line delegation to `std::fs`; the value is the allow-list, not
//! any added behavior.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn canonicalize(p: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(p)
}

pub fn exists(p: &Path) -> bool {
    p.exists()
}

pub fn metadata(p: &Path) -> io::Result<fs::Metadata> {
    fs::metadata(p)
}

pub fn read_dir(p: &Path) -> io::Result<fs::ReadDir> {
    fs::read_dir(p)
}

pub fn create_dir_all(p: &Path) -> io::Result<()> {
    fs::create_dir_all(p)
}

pub fn remove_dir_all(p: &Path) -> io::Result<()> {
    fs::remove_dir_all(p)
}

pub fn remove_file(p: &Path) -> io::Result<()> {
    fs::remove_file(p)
}

pub fn read_to_string(p: &Path) -> io::Result<String> {
    fs::read_to_string(p)
}

pub fn symlink_metadata(p: &Path) -> io::Result<fs::Metadata> {
    fs::symlink_metadata(p)
}
