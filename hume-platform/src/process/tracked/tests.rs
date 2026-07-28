//! No cross-platform tests here: every meaningful behaviour needs a real
//! spawned child, and the fixture commands (`sleep`, `sh`) are Unix-only —
//! same reasoning as `process/tests.rs`'s own split.

#[cfg(unix)]
mod unix;
