pub(crate) mod core;
pub(crate) mod ops;
pub(crate) mod ui;
pub(crate) mod auto_pairs;
pub(crate) mod cursor;
pub(crate) mod helpers;
pub(crate) mod os;
pub(crate) mod scripting;
pub(crate) mod settings;
pub(crate) mod editor;

// The test DSL is compiled only when running tests. It lives in its own
// module so every other module can `use crate::testing::*;` inside
// `#[cfg(test)]` blocks without any runtime cost in release builds.
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod proptest_doc;
#[cfg(test)]
mod proptest_editor;

/// Start the editor.
///
/// Installs the panic hook, initialises the terminal, runs the event loop,
/// and restores the terminal on exit (clean or panicking).
pub fn run(file_path: Option<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    os::terminal::install_panic_hook();

    let mut editor = editor::Editor::open(file_path)?;
    let (mut term, kitty_enabled) = os::terminal::init(editor.settings.mouse_enabled, editor.settings.mouse_select)?;
    editor.kitty_enabled = kitty_enabled;
    editor.init_scripting();

    let result = editor.run(&mut term);

    // Always restore the terminal, even if the event loop returned an error.
    os::terminal::restore()?;

    Ok(result?)
}
