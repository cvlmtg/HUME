//! The `hume` binary and what makes it an editor: buffers, keymaps,
//! command dispatch, the scripting and LSP glue, and the event loop.
//!
//! [`run`] starts the interactive editor in a terminal, and [`run_keys`]
//! replays a key sequence against a file with no terminal. [`cli`] holds
//! the file and config arguments both take.
//!
//! Inside, `Editor` keeps the Steel engine apart from `EditorState`, the
//! data a command can change, so a Steel-backed command can borrow both at
//! once. A key goes to the top of the input stack (the mode's keymap, or an
//! overlay that owns the keyboard), resolves to a command name in the
//! command registry, and runs, for the built-in commands, `hume-ops`'s pure
//! functions against the buffer. Each frame, the editor syncs its panes and
//! providers into `hume-engine` and draws.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

// The dump on panic runs while unwinding. Under `panic = "abort"` it would
// never run, so the build is refused.
#[cfg(panic = "abort")]
compile_error!("hume must build with panic = \"unwind\": dirty buffers are dumped while unwinding");

pub mod cli;
pub(crate) mod editor;
// `pub` for `tests/scripting.rs`, which names `StatusElement` from outside
// the crate.
pub mod statusline;
pub(crate) mod tabline;

// Lets `testing::MockHost` use `hume::` paths both in the lib's own test
// build and from the integration-test crates.
extern crate self as hume;

#[cfg(test)]
mod proptest_doc;
#[cfg(test)]
mod proptest_editor;
// `pub` for the integration-test crates, which enable it through the
// `test-util` feature; a release build leaves it out.
#[cfg(any(test, feature = "test-util"))]
pub mod testing;

/// Run a key sequence against a file without entering the interactive terminal.
///
/// Opens `input` at its position, if it has one, feeds every key in `keys`
/// (golf-stream notation; see [`hume_scripting::parse_key_stream`]) through
/// the editor's normal dispatch path, then writes the final buffer content to
/// `output`. No terminal is initialised. `config` picks what `init_scripting`
/// evaluates (the default `init.scm`, a `--config` override, or
/// `ConfigSource::Skip` for `--no-config`), as it does for [`run`].
/// `cwd` is the startup working directory.
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
    cwd: hume_platform::path::AbsolutePath,
) -> Result<(), Box<dyn std::error::Error>> {
    let parsed =
        hume_scripting::parse_key_stream(keys).map_err(|e| format!("invalid key stream: {e}"))?;

    // A fixed 80×25 screen (`HEADLESS_TERMINAL`), so scores are reproducible.
    let mut editor = editor::Editor::start(
        std::slice::from_ref(&input),
        editor::Driver::Headless,
        hume_platform::dirs::Dirs::from_env(&cwd)?,
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
            // `step` only dispatches the key; `settle()` drains the work
            // dispatch queues (hooks, LSP/timer callbacks), as `Editor::run`'s
            // loop does.
            editor.settle();
            if editor.state.should_quit {
                break;
            }
        }
        // `drain_async_sources` runs once per `settle()`, outside its
        // fixpoint, so a timer armed during the last key's settle becomes due
        // work only on the next call.
        editor.settle();
        std::fs::write(&output, editor.doc().text().to_string())
    };
    // Config can spawn LSP servers: give them `run`'s graceful shutdown
    // window rather than leaving `ServerHandle::drop` to `SIGKILL` them.
    // Before any `?`, which would skip it.
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
    cwd: hume_platform::path::AbsolutePath,
) -> Result<(), Box<dyn std::error::Error>> {
    let shared = hume_platform::terminal::create(&cwd)?;

    // Background threads (LSP transport, parse worker) call `wake()` after
    // posting a result, so the main loop wakes instead of polling.
    let waker = shared.event_reader().waker();
    let wake: hume_platform::wake::WakeCallback = std::sync::Arc::new(move || {
        let _ = waker.wake();
    });

    // The exit code the terminator thread sets on termination, polled by
    // `Editor::run`'s loop and re-read below. `0` means no termination was
    // requested. A bare pty hangup with no signal does not set it.
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
        // Non-fatal: Ctrl-c/SIGTERM/SIGHUP then leak terminal state or spin
        // the event loop instead of exiting, but a normal exit still works.
        eprintln!("hume: failed to start terminator: {e}");
    }

    let panic_wake = wake.clone();
    let kitty_enabled = hume_platform::terminal::probe_kitty(&shared)?;
    let (width, height) = hume_platform::screen::dimensions(&shared)?;
    let mut editor = editor::Editor::start(
        &files,
        editor::Driver::Terminal {
            kitty_enabled,
            wake,
            size: hume_grid::Rect::new(0, 0, width, height),
        },
        hume_platform::dirs::Dirs::from_env(&cwd)?,
        config,
        cwd,
    )?;
    editor.attach_terminate_flag(terminate.clone());

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
    // Restore the terminal before the LSP grace window: `lsp_shutdown_all`
    // can take `SHUTDOWN_GRACE` per server, which on the alternate screen
    // reads as a frozen editor. Errors are collected, not propagated, so a
    // dead pty does not skip the LSP shutdown. The cursor resets are
    // idempotent and need no gate; `restore_for_exit` claims the one
    // restore against the terminator thread's `force_exit`. If this thread
    // loses that claim it parks, and `force_exit` kills every tracked child,
    // LSP servers included.
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
    // Shutdown request, then exit notification, so `ServerHandle::drop`
    // does not `SIGKILL` the servers.
    editor.lsp_shutdown_all(editor::Editor::SHUTDOWN_GRACE);
    // The signal branch below calls `std::process::exit`, which runs no
    // destructors and would skip each LSP server's `WRITER_FLUSH_GRACE`
    // teardown.
    drop(editor);
    // Panics raised during the LSP shutdown or the drop above.
    editor::buffer::print_worker_panics(&mut std::io::stderr().lock(), &worker_panics);

    let code = terminate.load(std::sync::atomic::Ordering::Acquire);
    if code != 0 {
        // Killed by a signal: exit with the terminator's code. On a hangup
        // every teardown write fails with `EIO`, and propagating that would
        // print to a gone terminal and report the wrong code.
        std::process::exit(code);
    }

    // A hangup with no signal surfaces here: `editor.run`'s reader returns
    // `UnexpectedEof`/`EIO` once the pty master closes. Every teardown write
    // fails the same way, so `restore_err` needs the same recognition as
    // `result`.
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
