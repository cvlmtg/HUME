//! Headless editors for the criterion benches: a pane with a full selection
//! history (`benches/selection_history.rs`) and a concealed Markdown buffer
//! (`benches/conceal.rs`).

use std::path::Path;

use hume_decorations::{ConcealEntry, ConcealEntrySpan};
use hume_engine::pipeline::RenderContext;
use hume_engine::providers::ConcealText;
use hume_platform::dirs::Dirs;
use hume_platform::path::AbsolutePath;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use termina::event::KeyEvent;

use super::{Driver, Editor};
use crate::cli::{ConfigSource, FileArg};

/// Undo kept bounded, so the timed loops do not grow it.
const UNDO_LEVELS: &str = ":set global undo-levels=100<ret>";

/// A line selection and its collapse, ten times: 20 entries.
const FILL_HISTORY: &str = "x;x;x;x;x;x;x;x;x;x;";

/// Keys parsed once, outside the timed loop.
pub struct Keys(Vec<KeyEvent>);

impl Keys {
    /// Feeds each key to `editor`, settling after each as `run_keys` does.
    fn press(&self, editor: &mut Editor) {
        for &key in &self.0 {
            editor.step(key);
            editor.settle();
        }
    }
}

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
        keys.press(&mut self.editor);
    }

    /// How many selections the focused pane has.
    pub fn selection_count(&self) -> usize {
        self.editor.current_view().len()
    }
}

/// A Markdown file of `lines` lines: a heading every 10 lines, prose with one
/// emphasis or code run per line, a 5-row table every 50 and a fenced block
/// every 100. ASCII only, so a char offset is a byte offset.
fn markdown_sample(lines: usize) -> String {
    let mut text = String::new();
    for n in 0..lines {
        match (n % 10, n % 50, n % 100) {
            (0, ..) => text.push_str(&format!("## Heading {n}\n")),
            (_, 21, _) => text.push_str("| name | value | note |\n"),
            (_, 22, _) => text.push_str("|------|-------|------|\n"),
            (_, 23..=25, _) => text.push_str(&format!("| row {n} | {n} | text |\n")),
            (.., 62) => text.push_str("```rust\n"),
            (.., 63..=65) => text.push_str(&format!("let value_{n} = {n};\n")),
            (.., 66) => text.push_str("```\n"),
            _ => {
                let run = match n % 3 {
                    0 => format!("**bold {n}**"),
                    1 => format!("*italic {n}*"),
                    _ => format!("`code {n}`"),
                };
                text.push_str(&format!(
                    "Line {n} of the prose, with a {run} run and enough words after it \
                     to wrap past the right edge of a pane (item {n}).\n"
                ));
            }
        }
    }
    text
}

/// A `Hidden` conceal over chars `start..end` drawing `glyph` in its place
/// when set.
fn hidden(
    start: usize,
    end: usize,
    glyph: Option<&str>,
    reveal: Option<ExclusiveRange<CharOffset>>,
) -> ConcealEntry {
    ConcealEntry {
        span: ConcealEntrySpan::Hidden {
            start: CharOffset::new(start),
            end: CharOffset::new(end),
            text: glyph.map(|g| ConcealText {
                text: g.into(),
                scope: None,
            }),
        },
        reveal,
    }
}

/// How many conceals a bench's store holds.
#[derive(Clone, Copy, Debug)]
pub enum StoreSize {
    /// The conceals of the first [`VIEWPORT_LINES`] lines, widened to every
    /// table a held line belongs to: a stand-in for what a viewport-range
    /// render sets, sized to the plugin's own store on the sample.
    Viewport,
    /// The conceals of every line, about 10k on the 5,000-line sample.
    AllLines,
}

/// Lines the viewport stand-in holds. The plugin's render of an 80x25 pane at
/// the top of the sample sets 86 conceals; with the table's four pipes per
/// line and two conceals on every other line, 38 lines give the same count.
const VIEWPORT_LINES: usize = 38;

impl StoreSize {
    /// Whether the store holds the conceals of buffer line `line`.
    fn holds(self, line: usize) -> bool {
        match self {
            Self::Viewport => line < VIEWPORT_LINES,
            Self::AllLines => true,
        }
    }
}

/// The conceals a Markdown plugin sets on [`markdown_sample`]: two per
/// heading (its marker and the space after it), two per emphasis or code run
/// and one per table pipe, a table's sharing its whole line span as the
/// reveal unit. `size` keeps the entries of the lines it holds and of every
/// table with a line it holds.
fn sample_conceals(text: &str, size: StoreSize) -> Vec<ConcealEntry> {
    let mut entries = Vec::new();
    // The first line, first char and pipe offsets of the table being scanned.
    let mut table: Option<(usize, usize, Vec<usize>)> = None;
    let mut in_fence = false;
    let mut line_start = 0;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let body = line.trim_end_matches('\n');
        if body.starts_with('|') {
            let (_, _, pipes) = table.get_or_insert((n, line_start, Vec::new()));
            pipes.extend(body.match_indices('|').map(|(i, _)| line_start + i));
        } else if let Some((first_line, first, pipes)) = table.take()
            && (first_line..n).any(|line| size.holds(line))
        {
            let reveal = ExclusiveRange::new(CharOffset::new(first), CharOffset::new(line_start));
            entries.extend(
                pipes
                    .into_iter()
                    .map(|at| hidden(at, at + 1, Some("\u{2502}"), Some(reveal))),
            );
        }
        if body.starts_with("```") {
            in_fence = !in_fence;
        } else if size.holds(n) && !in_fence && body.starts_with("## ") {
            entries.push(hidden(line_start, line_start + 2, Some("\u{258c}"), None));
            entries.push(hidden(line_start + 2, line_start + 3, None, None));
        } else if size.holds(n) && !in_fence && !body.starts_with('|') {
            let run = ["**", "*", "`"]
                .into_iter()
                .find_map(|delim| body.find(delim).map(|open| (delim, open)));
            if let Some((delim, open)) = run {
                let close = open
                    + delim.len()
                    + body[open + delim.len()..]
                        .find(delim)
                        .expect("a run closes");
                for at in [open, close] {
                    entries.push(hidden(
                        line_start + at,
                        line_start + at + delim.len(),
                        None,
                        None,
                    ));
                }
            }
        }
        line_start += line.len();
    }
    entries
}

/// A headless editor on a [`markdown_sample`] with soft wrap and, when
/// given a [`StoreSize`], the conceals a Markdown plugin sets on it.
pub struct ConcealBench {
    editor: Editor,
    /// Whether the next [`Self::reveal_all_j_then_settle`] presses `j` or `k`.
    down: bool,
}

const SOURCE: &str = "bench";

impl ConcealBench {
    /// Writes a `lines`-line sample to `file`, opens it at 80x25 with soft
    /// wrap and draws one frame, with `conceals` set first when given.
    ///
    /// # Panics
    /// Panics if the editor cannot start or the setup logs an error.
    pub fn new(file: &Path, lines: usize, conceals: Option<StoreSize>) -> Self {
        let text = markdown_sample(lines);
        std::fs::write(file, &text).expect("the bench file is written");
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
        let mut bench = Self { editor, down: true };
        bench.editor.resize_terminal(80, 25);
        bench.editor.settle();
        bench.press(&SelectionHistoryBench::keys(
            ":set global wrap-mode=soft<ret>",
        ));
        if let Some(size) = conceals {
            let bid = bench.editor.focused_buffer_id();
            bench.editor.state.config.decorations.set_conceals(
                SOURCE.into(),
                bid,
                sample_conceals(&text, size),
            );
        }
        bench.draw();
        assert_eq!(
            bench.editor.state.message_log.totals().0,
            0,
            "the setup logged an error"
        );
        bench
    }

    /// How many conceals the sample holds.
    pub fn conceal_count(&self) -> usize {
        let bid = self.editor.focused_buffer_id();
        self.editor
            .state
            .config
            .decorations
            .conceals_for_buffer(bid)
            .count()
    }

    fn draw(&mut self) {
        self.editor.settle();
        self.editor.prepare_frame(&mut RenderContext::new());
    }

    fn press(&mut self, keys: &Keys) {
        keys.press(&mut self.editor);
    }

    /// Selects every match of `pattern` in the buffer, returning how many
    /// selections the pane then holds.
    pub fn select_matches(&mut self, pattern: &str) -> usize {
        self.press(&SelectionHistoryBench::keys(&format!("%s{pattern}<ret>")));
        self.editor.current_view().len()
    }

    /// Rebuilds the pane's mirror of the conceals, as the next frame does
    /// after an edit bumps the decoration generation.
    pub fn mirror_rebuild(&mut self) {
        let pid = self.editor.view.focused_pane();
        self.editor.buffer_mirrors_synced.remove(&pid);
        let size = self.editor.state.pane_size(&self.editor.view, pid);
        let panes = self.editor.decorated_panes(&[(pid, size)], &[]);
        self.editor.sync_buffer_mirrors(&panes);
    }

    /// The focused pane's layout key, which reads every selection.
    pub fn layout_key(&mut self) {
        let pid = self.editor.view.focused_pane();
        let size = self.editor.state.pane_size(&self.editor.view, pid);
        std::hint::black_box(self.editor.state.layout_key(&self.editor.view, pid, size));
    }

    /// One `j` or `k` (alternating), settled: the cursor moves over the raw
    /// layout and the view follows by the layout the move leaves.
    pub fn reveal_all_j_then_settle(&mut self) {
        let key = if self.down { "j" } else { "k" };
        self.down = !self.down;
        self.press(&SelectionHistoryBench::keys(key));
        self.editor.settle_views();
    }

    /// Lays out every line of the buffer once with the pane's soft wrap.
    pub fn format_every_line(&mut self) {
        self.editor.view.begin_frame();
        let pid = self.editor.view.focused_pane();
        let size = self.editor.state.pane_size(&self.editor.view, pid);
        let inputs = self
            .editor
            .state
            .layout_key(&self.editor.view, pid, size)
            .inputs();
        let bid = self.editor.focused_buffer_id();
        let text = self.editor.state.buffers.get(bid).text();
        let mut map = self.editor.view.panes[pid].display_lines(text.rope(), inputs);
        for line in 0..text.content_line_count().get() {
            std::hint::black_box(map.block(ContentLine::new(line)));
        }
    }
}
