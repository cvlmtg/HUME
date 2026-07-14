pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("HUME_GIT_SHA"));

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

/// Run a key sequence against a file without entering the interactive terminal.
///
/// Opens `input`, feeds every key in `keys` (golf-stream notation — see
/// [`hume_scripting::parse_key_stream`]) through the editor's normal dispatch
/// path, then writes the final buffer content to `output`.  No terminal is
/// initialised and no `init.scm` is loaded.
///
/// Exits cleanly when the key sequence contains `:wq` / `:q` / `<c-c>` (the
/// editor sets `should_quit`); the buffer is written to `output` regardless.
pub fn run_keys(
    input: std::path::PathBuf,
    keys: &str,
    output: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let parsed =
        hume_scripting::parse_key_stream(keys).map_err(|e| format!("invalid key stream: {e}"))?;

    let mut editor = editor::Editor::open(Some(input))?;
    // Headless mode: no terminal to negotiate kitty protocol, so assume
    // full capability. Ctrl+letter keys (e.g. `<c-w>`) are no-ops without
    // this since the dispatcher strips the Ctrl modifier only when
    // kitty_enabled is true (see handle_normal). The kitty-only default
    // binds are also installed to match interactive kitty.
    editor.set_kitty_support(true);
    // The pane viewport defaults to 80×24 (from Pane::new) and is never
    // updated without a terminal, so scores are reproducible.

    for key in parsed {
        editor.step(key);
        if editor.state.should_quit {
            break;
        }
    }

    let content = editor.doc().text().to_string();
    std::fs::write(&output, content)?;
    Ok(())
}

/// Start the editor.
///
/// Scripting initialisation (Steel VM boot + `init.scm`, ~150-200 ms) runs
/// *before* the terminal enters raw mode / the alternate screen, so the
/// user's shell stays visible during that window — the first alt-screen
/// frame shown is fully themed and (at most one poll later)
/// syntax-highlighted. The kitty protocol is probed first (on the normal
/// screen) and applied via `set_kitty_support` before `init_scripting`, so
/// kitty-only default keybinds install before any user `bind-key!` in
/// `init.scm` can override them.
///
/// Accepted side effect: keys typed during the pre-alt-screen window echo
/// to the shell and aren't seen by the editor; any left in the input buffer
/// are read once raw mode / the alt-screen are entered.
///
/// `TerminalGuard` ensures restore runs on every exit path — clean return,
/// `?`-propagated error, or panic — including a panic before the terminal
/// was ever touched (`restore` is a harmless no-op then).
pub fn run(file_paths: Vec<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = hume_platform::install_signal_handlers() {
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
    let mut guard = hume_platform::terminal::TerminalGuard::new();

    let mut editor = editor::Editor::open(first)?;
    let kitty_enabled = hume_platform::terminal::probe_kitty()?;
    editor.set_kitty_support(kitty_enabled);
    editor.init_scripting();
    // Open remaining paths after scripting init so OnBufferOpen hooks fire.
    editor.open_extra_files(rest);
    // Drain hooks queued during init (OnBufferOpen, OnLanguageSet, etc.) before
    // entering the event loop. fire_hook_silent only enqueues; without an explicit
    // drain here they would silently defer to the first keypress.
    editor.drain_hooks();

    let mut term = hume_platform::terminal::init(
        editor.state.settings.mouse_enabled,
        editor.state.settings.mouse_select,
        kitty_enabled,
    )?;
    let result = editor.run(&mut term);

    // Explicit restore on the happy path so IO errors propagate to the caller.
    // Disarm the guard afterwards to suppress the drop-time call.
    hume_platform::terminal::restore()?;
    guard.disarm();

    Ok(result?)
}
