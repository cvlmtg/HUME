#![deny(rustdoc::broken_intra_doc_links)]

// The dump on panic runs while unwinding. Under `panic = "abort"` it would
// never run, so the build is refused.
#[cfg(panic = "abort")]
compile_error!("hume must build with panic = \"unwind\": dirty buffers are dumped while unwinding");

pub mod cli;
pub(crate) mod editor;
// `pub`, not `pub(crate)`: `tests/scripting.rs` is a separate crate that
// links against this lib and names `hume_editor::statusline::StatusElement`
// directly.
pub mod statusline;
pub(crate) mod tabline;

// `extern crate self as hume` lets `hume::testing::MockHost` (reached both
// from the lib's own `#[cfg(test)]` build and from `tests/scripting.rs`/
// `tests/unix/main.rs` via the `test-util` feature below) use `hume::` paths
// uniformly in both contexts, without conditional `crate::` vs `hume::`
// branching.
extern crate self as hume;

#[cfg(test)]
mod proptest_doc;
#[cfg(test)]
mod proptest_editor;
// `pub`, not `pub(crate)`: `tests/scripting.rs` and `tests/unix/main.rs` are
// separate crates that link against this lib rather than being compiled into
// it, so `pub(crate)` would be invisible to them. `test-util` is what lets
// those crates' `Cargo.toml` dev-dependency turn this module on without it
// leaking into a release build.
#[cfg(any(test, feature = "test-util"))]
pub mod testing;

/// Run a key sequence against a file without entering the interactive terminal.
///
/// Opens `input` at its position, if it has one, feeds every key in `keys`
/// (golf-stream notation; see [`hume_scripting::parse_key_stream`]) through
/// the editor's normal dispatch path, then writes the final buffer content to
/// `output`. No terminal is
/// initialised. `config` picks what `init_scripting` evaluates (the default
/// `init.scm`, a `--config` override, or `ConfigSource::Skip` for
/// `--no-config`), exactly as it would for [`run`]. `cwd` is the startup
/// working directory.
///
/// Stops replaying at `:wq` / `:q` / `<c-c>` (the editor sets
/// `should_quit`); the buffer is written to `output` regardless.
///
/// With no screen, the message log is the run's only feedback: its warnings
/// and errors go to stderr, and any error fails the run. An error logged
/// while the config loads stops it before the first key, leaving `output`
/// unwritten, since keys replayed on a half-loaded config prove nothing. One
/// logged during the replay fails it after `output` is written.
pub fn run_keys(
    input: cli::FileArg,
    keys: &str,
    output: std::path::PathBuf,
    config: cli::ConfigSource,
    cwd: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let parsed =
        hume_scripting::parse_key_stream(keys).map_err(|e| format!("invalid key stream: {e}"))?;

    // Headless: no terminal, so nothing to wake: background threads (parse
    // worker, LSP transport) call this harmlessly into the void. The pane
    // viewport defaults to 80×24 (from Pane::new) and is never updated
    // without a terminal, so scores are reproducible.
    let mut editor = editor::Editor::start(
        std::slice::from_ref(&input),
        editor::Driver::Headless,
        std::sync::Arc::new(|| {}),
        hume_platform::dirs::Dirs::from_env(&cwd),
        config,
        cwd,
    )?;
    // The startup work `Editor::run` settles before its first frame
    // (`on-buffer-open` hooks, the plugins they activate, the input's
    // position) runs before the first key, so its errors count as the
    // config's.
    editor.settle();
    let config_failed = editor.state.message_log.totals().0 > 0;

    let written = if config_failed {
        Ok(())
    } else {
        for key in parsed {
            editor.step(key);
            // `step` only dispatches the key; `settle()` is what drains the
            // queued work (hooks, LSP/timer callbacks) that dispatch enqueues.
            // This mirrors `Editor::run`'s interactive loop, where the loop settles
            // and the input handler doesn't (see `Editor::settle`'s doc).
            editor.settle();
            if editor.state.should_quit {
                break;
            }
        }
        // One more pass after the loop: `drain_async_sources` runs once per
        // `settle()` call, outside its fixpoint (see that doc), so a timer a
        // handler armed *during* the last key's own settle only converts to
        // due work on the next call. With no further key to trigger one, it
        // would otherwise never fire.
        editor.settle();
        std::fs::write(&output, editor.doc().text().to_string())
    };
    // Config can spawn LSP servers, so give them the same graceful shutdown
    // window `run` gives them, rather than leaving `ServerHandle::drop` to
    // `SIGKILL` them on the way out. Runs before any result is propagated:
    // an early `?` would skip it.
    editor.lsp_shutdown_all(editor::Editor::SHUTDOWN_GRACE);
    let reported = editor
        .state
        .message_log
        .write_problems(&mut std::io::stderr().lock());
    written?;
    reported?;
    match editor.state.message_log.totals().0 {
        0 => Ok(()),
        _ if config_failed => Err("the config failed to load, so no keys were replayed".into()),
        1 => Err("1 error was logged".into()),
        n => Err(format!("{n} errors were logged").into()),
    }
}

/// Start the editor.
///
/// `config` is the validated `ConfigSource` from `main.rs`'s `resolve`; it is
/// set on the editor before `init_scripting` runs. `cwd` is the startup
/// working directory.
///
/// Scripting init (Steel VM boot and `init.scm`, ~150-200 ms) runs before raw
/// mode and the alternate screen, so the shell stays visible meanwhile and the
/// first frame is already themed. Keys typed in that window echo to the shell.
/// The kitty protocol is probed first so kitty-only default keybinds install
/// before `init.scm` can override them.
///
/// [`hume_platform::terminal::init`]'s panic hook restores the terminal on
/// unwind; the explicit [`hume_platform::terminal::restore`] covers normal
/// and error returns. Both are safe if `init` was never reached.
///
/// Errors are checked against [`hume_platform::hangup_exit_code`] before
/// propagating. That is sound only while every fallible step in
/// `editor.run`'s loop is terminal I/O: `hangup_exit_code` cannot tell a
/// non-terminal error apart.
pub fn run(
    files: Vec<cli::FileArg>,
    config: cli::ConfigSource,
    cwd: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let shared = hume_platform::terminal::create(&cwd)?;

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
    // observe the same atomic. `0` means "no termination requested", never
    // a valid signal-termination exit code. A bare pty hangup with no signal
    // does not go through this; see `hume_platform::hangup_exit_code`'s doc.
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
        // Non-fatal: Ctrl-c/SIGTERM/SIGHUP will leak terminal
        // state or spin the event loop instead of exiting, but the editor
        // still works correctly for normal exit.
        eprintln!("hume: failed to start terminator: {e}");
    }

    let panic_wake = wake.clone();
    let kitty_enabled = hume_platform::terminal::probe_kitty(&shared)?;
    let mut editor = editor::Editor::start(
        &files,
        editor::Driver::Terminal { kitty_enabled },
        wake,
        hume_platform::dirs::Dirs::from_env(&cwd),
        config,
        cwd,
    )?;
    editor.attach_terminate_flag(terminate.clone());
    // No explicit startup drain: work queued during init (OnBufferOpen,
    // OnLanguageSet, etc.) sits in `pending_work` until `Editor::run`'s loop
    // reaches its first `settle()`, which now runs before the first
    // `should_quit` check and the first frame, so nothing is lost by leaving
    // it queued here.

    let mut term = hume_platform::terminal::init(
        &shared,
        editor.state.settings.mouse,
        editor.state.settings.mouse_select,
        kitty_enabled,
    )?;
    // After `init`, so the hook it wraps is the terminal's: from here a panic
    // on a background thread is queued for the editor to report instead of
    // restoring the terminal under a loop that keeps drawing.
    let worker_panics = hume_platform::worker_panic::WorkerPanics::install(panic_wake);
    editor.attach_worker_panics(worker_panics.clone());
    let scratch_dumps = editor.state.dirs.data.as_ref().map(|dir| dir.join("dumps"));
    let result = editor.run_dumping_on_panic(scratch_dumps.as_deref(), |editor| {
        editor.run(&shared, &mut term)
    });
    // Before any terminal write or teardown: the terminator thread forces
    // the process down `QUIT_GRACE` after a signal, and each dump is one
    // write and one rename.
    let dumps = editor.dump_if_abnormal_exit(&result, scratch_dumps.as_deref());
    // Restore the terminal (cursor shape/colour, leave alt-screen, cooked
    // mode) before the LSP grace window below, not after: `lsp_shutdown_all`
    // can take up to `SHUTDOWN_GRACE` per server, and every millisecond of
    // that is otherwise spent sitting in the alternate screen, reading as a
    // frozen editor. Errors are collected rather than propagated immediately
    // (`run_all`'s "attempt everything, report the first failure" discipline,
    // `hume_platform::terminal`) so a dead pty here doesn't skip the graceful
    // LSP shutdown that follows. The cursor shape/colour resets below are
    // unconditional and idempotent regardless of who else is tearing down at
    // the same moment, so they need no gate; `restore_for_exit` (not a bare
    // `terminal::restore`) is the one function allowed to write the
    // alt-screen-leave/raw-mode-restore/kitty-keyboard-pop sequence that
    // follows them. It gates on `claim_exit`, the process-wide
    // single-restorer race with the terminator thread's `force_exit`, so a
    // second thread mid-teardown at the same moment can't interleave a
    // second copy of that sequence into this one's. Calling it first, while
    // there's the most of `QUIT_GRACE` left, also minimizes how often this
    // thread is the one that loses that race and parks here instead of
    // returning: if it does, the terminator thread's `force_exit` has
    // already won the claim ahead of it (the only way this thread's own
    // claim attempt loses) and goes on to `kill_tracked_children`,
    // including every attached LSP server, right after, so the graceful
    // shutdown below would never run regardless.
    let mut restore_err = hume_platform::terminal::set_cursor_shape(
        &shared,
        hume_platform::terminal::CursorStyle::Default,
    )
    .err();
    let _ = hume_platform::terminal::set_cursor_color(&shared, false); // emits reset sequence
    if let Err(e) = hume_platform::restore_for_exit(&shared) {
        restore_err.get_or_insert(e);
    }
    // Back on the normal screen, where the messages stay in scrollback.
    editor::buffer::report_dumps(&mut std::io::stderr().lock(), &dumps);
    editor::buffer::print_worker_panics(&mut std::io::stderr().lock(), &worker_panics);
    // Give every running LSP server a chance to exit cleanly (shutdown
    // request, then exit notification) before the process ends;
    // ServerHandle::drop would otherwise SIGKILL them.
    editor.lsp_shutdown_all(editor::Editor::SHUTDOWN_GRACE);
    // Explicit, not left to the function's own scope-end order: the signal
    // exit branch below calls `std::process::exit`, which runs no
    // destructors, so leaving this to an implicit end-of-scope drop would
    // skip each attached LSP server's `WRITER_FLUSH_GRACE` teardown whenever
    // that branch is taken.
    drop(editor);
    // Panics raised during the LSP shutdown or the drop above.
    editor::buffer::print_worker_panics(&mut std::io::stderr().lock(), &worker_panics);

    let code = terminate.load(std::sync::atomic::Ordering::Acquire);
    if code != 0 {
        // Killed by a signal, not by `:q`: exit with the terminator's own
        // code rather than propagating `restore_err`/`result`: on a genuine
        // terminal hangup (e.g. SIGHUP with the pty already gone) every
        // teardown write fails with `EIO`, and surfacing that as a `?`
        // instead of the signal's exit code would print an error to a
        // terminal that's already gone and report the wrong code.
        std::process::exit(code);
    }

    // Not a signal exit. A bare hangup (the controlling terminal went away
    // with no signal delivered at all) surfaces here instead: `editor.run`'s
    // reader returns `UnexpectedEof`/`EIO` once the pty master closes rather
    // than spinning (see `hume_platform::hangup_exit_code`'s doc for exactly
    // which errors this recognizes). Every teardown write above fails the
    // same way against a dead pty, so `restore_err` (checked first, same as
    // the signal branch above) needs the same recognition as `result`, or a
    // hangup here would be misreported as a real teardown failure instead of
    // exiting with the conventional code.
    if let Some(code) = restore_err
        .as_ref()
        .and_then(hume_platform::hangup_exit_code)
    {
        std::process::exit(code);
    }
    if let Some(e) = restore_err {
        return Err(e.into());
    }
    if let Some(code) = result
        .as_ref()
        .err()
        .and_then(hume_platform::hangup_exit_code)
    {
        std::process::exit(code);
    }
    Ok(result?)
}
