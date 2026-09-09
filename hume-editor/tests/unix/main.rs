//! Unix-only scripting integration tests, gated once at the crate level —
//! `cargo test` compiles this to an empty crate on Windows.
//!
//! A sibling of `tests/scripting.rs` rather than a `tests/scripting/unix.rs`
//! submodule: Cargo only auto-discovers `tests/*.rs` and `tests/*/main.rs` as
//! integration-test targets, so the unix-only half needs its own crate root.

#![cfg(unix)]

// Alias so `scripting.rs` can use `hume::` paths, matching the lib's own
// `extern crate self as hume`.
extern crate hume_editor as hume;

mod scripting;
