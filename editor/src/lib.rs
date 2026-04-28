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
/// Installs the panic hook, initialises the terminal, runs the event loop,
/// and restores the terminal on exit (clean or panicking).
pub fn run(file_paths: Vec<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    os::terminal::install_panic_hook();

    let (first, rest) = match file_paths.split_first() {
        Some((first, rest)) => (Some(first.clone()), rest),
        None => (None, &[][..]),
    };

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

    // Always restore the terminal, even if the event loop returned an error.
    os::terminal::restore()?;

    Ok(result?)
}
