//! Unix-only scripting integration tests, gated once at the crate level —
//! `cargo test` compiles this to an empty crate on Windows.
//!
//! A sibling of `tests/scripting.rs` rather than a `tests/scripting/unix.rs`
//! submodule: Cargo only auto-discovers `tests/*.rs` and `tests/*/main.rs` as
//! integration-test targets, so the unix-only half needs its own crate root.

#![cfg(unix)]

// Alias so mock_host.rs (included below via #[path]) can keep its `hume::` paths.
extern crate hume_editor as hume;

#[path = "../../src/testing/mock_host.rs"]
mod mock_host;

mod scripting;
