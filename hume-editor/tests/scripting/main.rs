//! Scripting-crate integration tests, split by builtin area. A sibling
//! crate root of `tests/unix/main.rs`'s own split. This file plays the
//! `main.rs` role: Cargo auto-discovers `tests/*.rs` and `tests/*/main.rs`
//! as integration-test targets, so the split keeps the binary name
//! `scripting` by living at `tests/scripting/main.rs` instead of renaming.

// Alias so this file can use `hume::` paths, matching the lib's own
// `extern crate self as hume`.
extern crate hume_editor as hume;

use hume::testing::MockHost;
use hume_engine::pipeline::BufferId;
use hume_platform::dirs::Dirs;
use hume_scripting::EvalWatchdog;
use hume_scripting::host::BindMode;
use hume_scripting::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn host() -> ScriptingHost {
    ScriptingHost::new(&Dirs::none())
}

/// Real `runtime/scheme/prelude.scm` source, read fresh per call so these tests
/// exercise the file plugin authors actually get, using the same `CARGO_MANIFEST_DIR`-
/// relative approach as `editor::tests::scripting_grammar::runtime_scheme_dir`,
/// which is independent of the test runner's CWD.
fn real_prelude_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-editor manifest dir has a parent")
        .join("runtime/scheme/prelude.scm");
    std::fs::read_to_string(&path).expect("reading runtime/scheme/prelude.scm failed")
}

mod call_and_hooks;
mod commands;
mod keymap;
mod misc;
mod options;
mod registers_and_guards;
mod typed_commands_and_watchdog;
