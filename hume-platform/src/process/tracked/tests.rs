//! No cross-platform tests here: every meaningful behaviour needs a real
//! spawned child, and the fixture commands (`sleep`, `sh` on Unix, `cmd` on
//! Windows) are platform-specific: same reasoning as `process/tests.rs`'s
//! own split.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
