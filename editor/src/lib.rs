pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("HUME_GIT_SHA"));

pub(crate) mod auto_pairs;
pub(crate) mod core;
pub(crate) mod cursor;
pub(crate) mod editor;
pub(crate) mod helpers;
pub(crate) mod ops;
pub(crate) mod os;
pub(crate) mod scripting;
pub(crate) mod settings;
pub(crate) mod ui;

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
    if let Err(e) = os::install_signal_handlers() {
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
    let mut guard = os::terminal::TerminalGuard::new();

    let mut editor = editor::Editor::open(first)?;
    let (mut term, kitty_enabled) =
        os::terminal::init(editor.settings.mouse_enabled, editor.settings.mouse_select)?;
    editor.kitty_enabled = kitty_enabled;
    // Paint the buffer with default settings immediately so the user sees the
    // editor chrome while Steel initialises, rather than a blank alt-screen.
    editor.draw_once(&mut term)?;
    editor.init_scripting();
    // Open remaining paths after scripting init so OnBufferOpen hooks fire.
    editor.open_extra_files(rest);

    let result = editor.run(&mut term);

    // Explicit restore on the happy path so IO errors propagate to the caller.
    // Disarm the guard afterwards to suppress the drop-time call.
    os::terminal::restore()?;
    guard.disarm();

    Ok(result?)
}
