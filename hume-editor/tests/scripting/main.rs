//! Scripting-crate integration tests, split by builtin area. A sibling
//! crate root of `tests/unix/main.rs`'s own split — this file plays the
//! `main.rs` role: Cargo auto-discovers `tests/*.rs` and `tests/*/main.rs`
//! as integration-test targets, so the split keeps the binary name
//! `scripting` by living at `tests/scripting/main.rs` instead of renaming.

// Alias so this file can use `hume::` paths, matching the lib's own
// `extern crate self as hume`.
extern crate hume_editor as hume;

use hume::testing::MockHost;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_scripting::EvalWatchdog;
use hume_scripting::host::BindMode;
use hume_scripting::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn host() -> ScriptingHost {
    ScriptingHost::new()
}

mod call_and_hooks;
mod commands;
mod keymap;
mod misc;
mod options;
mod registers_and_guards;
mod typed_commands_and_watchdog;
