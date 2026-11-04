//! A headless editor whose pane holds a full selection history, for
//! `benches/selection_history.rs`.

use std::path::Path;

use hume_platform::dirs::Dirs;
use hume_platform::path::AbsolutePath;
use termina::event::KeyEvent;

use super::{Driver, Editor};
use crate::cli::{ConfigSource, FileArg};

/// Undo kept bounded, so the timed loops do not grow it.
const UNDO_LEVELS: &str = ":set global undo-levels=100<ret>";

/// A line selection and its collapse, ten times: 20 entries.
const FILL_HISTORY: &str = "x;x;x;x;x;x;x;x;x;x;";

/// Keys parsed once, outside the timed loop.
pub struct Keys(Vec<KeyEvent>);

/// An editor on one file, its pane with a cursor on each of the first lines
/// and a full selection history.
pub struct SelectionHistoryBench {
    editor: Editor,
}

impl SelectionHistoryBench {
    /// Opens `file` with no config, puts a cursor on each of its first
    /// `cursors` lines (at least two) and fills the history.
    ///
    /// # Panics
    /// Panics if `cursors` is under two, the editor cannot start or the
    /// setup logs an error.
    pub fn new(file: &Path, cursors: usize) -> Self {
        assert!(cursors >= 2, "the setup copies the first cursor down");
        let cwd = AbsolutePath::current_dir().expect("the bench runs in a directory");
        let input = FileArg {
            path: AbsolutePath::from_path(file, &cwd).expect("the bench file has a path"),
            pos: None,
        };
        let editor = Editor::start(
            std::slice::from_ref(&input),
            Driver::Headless,
            Dirs::none(),
            ConfigSource::Skip,
            cwd,
        )
        .expect("the editor starts headless");
        let mut bench = Self { editor };
        bench.editor.settle();
        bench.press(&Self::keys(&format!(
            "{UNDO_LEVELS}{}C{FILL_HISTORY}",
            cursors - 1
        )));
        assert_eq!(
            bench.editor.state.message_log.totals().0,
            0,
            "the setup logged an error"
        );
        bench
    }

    /// `keys` in golf-stream notation (`<esc>`, `<c-p>`).
    ///
    /// # Panics
    /// Panics if `keys` does not parse.
    pub fn keys(keys: &str) -> Keys {
        Keys(hume_scripting::parse_key_stream(keys).expect("bench keys parse"))
    }

    /// Feeds `keys` one at a time, settling after each as `run_keys` does.
    pub fn press(&mut self, keys: &Keys) {
        for &key in &keys.0 {
            self.editor.step(key);
            self.editor.settle();
        }
    }

    /// How many selections the focused pane has.
    pub fn selection_count(&self) -> usize {
        self.editor.current_view().len()
    }
}
