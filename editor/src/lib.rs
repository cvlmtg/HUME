pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("HUME_GIT_SHA"));

pub mod auto_pairs;
pub mod editor;
pub mod ops;
pub mod settings;
pub mod ui;

// `extern crate self as hume` lets files that are #[path]-included into both
// the lib's own test build and external integration-test crates use `hume::`
// paths uniformly, without conditional `crate::` vs `hume::` branching.
extern crate self as hume;

// Re-exports for editor/tests/ integration tests.
pub use editor::keymap::{BindMode as KeymapBindMode, Keymap};

// The test DSL is compiled only when running tests. It lives in its own
// module so every other module can `use crate::testing::*;` inside
// `#[cfg(test)]` blocks without any runtime cost in release builds.
#[cfg(test)]
mod proptest_doc;
#[cfg(test)]
mod proptest_editor;
#[cfg(test)]
pub(crate) mod testing;

/// Start the editor.
///
/// Initialises the terminal, runs the event loop, and restores the terminal
/// on exit. The `TerminalGuard` ensures restore runs on every exit path:
/// clean return, `?`-propagated error, and panic unwinding.
pub fn run(file_paths: Vec<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = platform::install_signal_handlers() {
        // Non-fatal: SIGTERM/SIGHUP will leak terminal state, but the editor
        // still works correctly for normal exit.
        eprintln!("hume: failed to install signal handlers: {e}");
    }

    let (first, rest) = match file_paths.split_first() {
        Some((first, rest)) => (Some(first.clone()), rest),
        None => (None, &[][..]),
    };

    // Guard is declared first so it drops last. On any unwinding path the
    // Terminal's BufWriter flushes before restore() fires, ensuring no
    // buffered render bytes reach the main screen after the alt screen exits.
    let mut guard = platform::terminal::TerminalGuard::new();

    let mut editor = editor::Editor::open(first)?;
    let (mut term, kitty_enabled) =
        platform::terminal::init(editor.state.settings.mouse_enabled, editor.state.settings.mouse_select)?;
    editor.kitty_enabled = kitty_enabled;
    // Paint the buffer with default settings immediately so the user sees the
    // editor chrome while Steel initialises, rather than a blank alt-screen.
    editor.draw_once(&mut term)?;
    editor.init_scripting();
    // Open remaining paths after scripting init so OnBufferOpen hooks fire.
    editor.open_extra_files(rest);
    // Drain any commands queued by (call! …) in init.scm or plugin load.
    editor.run_startup_commands();
    // Drain hooks queued during init (OnBufferOpen, OnLanguageSet, etc.) before
    // entering the event loop. fire_hook_silent only enqueues; without an explicit
    // drain here they would silently defer to the first keypress.
    editor.drain_hooks();

    let result = editor.run(&mut term);

    // Explicit restore on the happy path so IO errors propagate to the caller.
    // Disarm the guard afterwards to suppress the drop-time call.
    platform::terminal::restore()?;
    guard.disarm();

    Ok(result?)
}
