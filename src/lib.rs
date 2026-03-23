pub(crate) mod display_line;
pub(crate) mod view;
pub(crate) mod renderer;
pub(crate) mod terminal;
pub(crate) mod editor;
pub(crate) mod buffer;
pub(crate) mod document;
pub(crate) mod changeset;
pub(crate) mod edit;
pub(crate) mod error;
pub(crate) mod grapheme;
pub(crate) mod helpers;
pub(crate) mod history;
pub(crate) mod motion;
pub(crate) mod register;
pub(crate) mod selection;
pub(crate) mod selection_cmd;
pub(crate) mod text_object;
pub(crate) mod transaction;

// The test DSL is compiled only when running tests. It lives in its own
// module so every other module can `use crate::testing::*;` inside
// `#[cfg(test)]` blocks without any runtime cost in release builds.
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod proptest_doc;

/// Start the editor.
///
/// Installs the panic hook, initialises the terminal, runs the event loop,
/// and restores the terminal on exit (clean or panicking).
pub fn run(file_path: Option<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    terminal::install_panic_hook();

    let mut editor = editor::Editor::open(file_path)?;
    let mut term = terminal::init()?;

    let result = editor.run(&mut term);

    // Always restore the terminal, even if the event loop returned an error.
    terminal::restore()?;

    Ok(result?)
}
