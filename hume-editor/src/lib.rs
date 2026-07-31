pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), env!("HUME_VERSION_SUFFIX"));

pub mod editor;
pub mod settings;
pub mod ui;

// `extern crate self as hume` lets files that are #[path]-included into both
// the lib's own test build and external integration-test crates use `hume::`
// paths uniformly, without conditional `crate::` vs `hume::` branching.
extern crate self as hume;

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

    // Headless: no terminal, so nothing to wake — background threads (parse
    // worker, LSP transport) call this harmlessly into the void.
    let mut editor = editor::Editor::open(Some(input), std::sync::Arc::new(|| {}))?;
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
/// [`hume_platform::terminal::init`] installs a panic hook that restores the
/// terminal on unwind; the explicit [`hume_platform::terminal::restore`] call
/// at the end covers the clean-return and `?`-propagated-error paths. Both
/// are safe to run even if `init` was never reached — every escape sequence
/// `restore` emits is a documented no-op for a mode that was never entered.
pub fn run(file_paths: Vec<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let shared = hume_platform::terminal::create()?;

    // The cross-thread waker: background threads (LSP transport, parse
    // worker) call `wake()` after posting a result so the main loop wakes
    // instead of polling for completion (see `Editor::run`'s event step).
    let waker = shared.event_reader().waker();
    let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(move || {
        let _ = waker.wake();
    });

    // Set by the terminator thread to the process exit code on termination,
    // polled at the top of `Editor::run`'s loop and re-read below after it
    // returns; shared with `editor.attach_terminate_flag` so both sides
    // observe the same atomic. `0` means "no termination requested" — never
    // a valid signal-termination exit code. A pty hangup does not go through
    // this — see `hume_platform::spawn_terminator`'s doc comment.
    let terminate = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let request_quit = {
        let (terminate, wake) = (terminate.clone(), wake.clone());
        move |code: i32| {
            // Store before waking: the loop must see the code set by the
            // time the wake actually rouses `reader.poll`.
            terminate.store(code, std::sync::atomic::Ordering::Release);
            wake();
        }
    };
    if let Err(e) = hume_platform::spawn_terminator(shared.clone(), request_quit) {
        // Non-fatal: Ctrl+C/SIGTERM/SIGHUP/pty-hangup will leak terminal
        // state or spin the event loop instead of exiting, but the editor
        // still works correctly for normal exit.
        eprintln!("hume: failed to start terminator: {e}");
    }

    let (first, rest) = match file_paths.split_first() {
        Some((first, rest)) => (Some(first.clone()), rest),
        None => (None, &[][..]),
    };

    let mut editor = editor::Editor::open(first, wake)?;
    editor.attach_terminate_flag(terminate.clone());
    let kitty_enabled = hume_platform::terminal::probe_kitty(&shared)?;
    editor.set_kitty_support(kitty_enabled);
    editor.attach_terminal(shared.clone());
    editor.init_scripting(&mut Default::default());
    // Open remaining paths after scripting init so OnBufferOpen hooks fire.
    editor.open_extra_files(rest);
    // Drain hooks queued during init (OnBufferOpen, OnLanguageSet, etc.) before
    // entering the event loop. fire_hook_silent only enqueues; without an explicit
    // drain here they would silently defer to the first keypress.
    editor.drain_hooks();

    let mut term = hume_platform::terminal::init(
        &shared,
        editor.state.settings.mouse_enabled,
        editor.state.settings.mouse_select,
        kitty_enabled,
    )?;
    let result = editor.run(&mut term);
    // Dropped before the exit claim below, not left to the function's own
    // scope-end order: `restore_for_exit` may lose its race with the
    // terminator thread's `force_exit` and park forever (see its doc), so
    // this thread's Drop-owned teardown (each attached LSP server's
    // `WRITER_FLUSH_GRACE`) must run first rather than risk never running.
    drop(editor);
    let restored = hume_platform::restore_for_exit(&shared);

    let code = terminate.load(std::sync::atomic::Ordering::Acquire);
    if code != 0 {
        // Killed by a signal, not by `:q` — exit with the terminator's own
        // code rather than propagating `restored`/`result`: on a genuine
        // terminal hangup (e.g. SIGHUP with the pty already gone) every
        // teardown write fails with `EIO`, and surfacing that as a `?`
        // instead of the signal's exit code would print an error to a
        // terminal that's already gone and report the wrong code.
        std::process::exit(code);
    }

    // Not a signal exit — an `EIO` here is a real, reportable failure.
    restored?;
    Ok(result?)
}
