//! Headless editors for the criterion benches: a pane with a full selection
//! history (`benches/selection_history.rs`), a concealed Markdown buffer
//! (`benches/conceal.rs`), and a file drawn by a real plugin: a Markdown
//! buffer by `core:markdown` (`benches/markdown_preview.rs`) and color
//! literals by `core:colorizer` (`benches/colorizer.rs`); and a paste over
//! many selections with `on-paste` handlers (`benches/on_paste.rs`).

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
    /// `keys` in golf-stream notation (`<esc>`, `<c-p>`).
    ///
    /// # Panics
    /// Panics if `keys` does not parse.
    pub fn parse(keys: &str) -> Self {
        Self(hume_scripting::parse_key_stream(keys).expect("bench keys parse"))
    }

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

/// An editor opened on `file` with no terminal, reading `config` from `dirs`.
fn headless_editor(file: &Path, dirs: Dirs, config: ConfigSource) -> Editor {
    let cwd = AbsolutePath::current_dir().expect("the bench runs in a directory");
    let input = FileArg {
        path: AbsolutePath::from_path(file, &cwd).expect("the bench file has a path"),
        pos: None,
    };
    Editor::start(
        std::slice::from_ref(&input),
        Driver::Headless,
        dirs,
        config,
        cwd,
    )
    .expect("the editor starts headless")
}

fn assert_setup_clean(editor: &Editor) {
    assert_eq!(
        editor.state.message_log.totals().0,
        0,
        "the setup logged an error"
    );
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
        let editor = headless_editor(file, Dirs::none(), ConfigSource::Skip);
        let mut bench = Self { editor };
        bench.editor.settle();
        bench.press(&Keys::parse(&format!(
            "{UNDO_LEVELS}{}C{FILL_HISTORY}",
            cursors - 1
        )));
        assert_setup_clean(&bench.editor);
        bench
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
            chars: ExclusiveRange::new(CharOffset::new(start), CharOffset::new(end)),
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
        let editor = headless_editor(file, Dirs::none(), ConfigSource::Skip);
        let mut bench = Self { editor, down: true };
        bench.editor.resize_terminal(80, 25);
        bench.editor.settle();
        Keys::parse(":set global wrap-mode=soft<ret>").press(&mut bench.editor);
        if let Some(size) = conceals {
            let bid = bench.editor.focused_buffer_id();
            bench.editor.state.config.decorations.set_conceals(
                SOURCE.into(),
                bid,
                sample_conceals(&text, size),
            );
        }
        bench.draw();
        assert_setup_clean(&bench.editor);
        bench
    }

    fn draw(&mut self) {
        self.editor.settle();
        self.editor.prepare_frame(&mut RenderContext::new());
    }

    /// Selects every match of `pattern` in the buffer, returning how many
    /// selections the pane then holds.
    pub fn select_matches(&mut self, pattern: &str) -> usize {
        Keys::parse(&format!("%s{pattern}<ret>")).press(&mut self.editor);
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
        Keys::parse(key).press(&mut self.editor);
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

/// The text [`markdown_preview_bench`] opens, its first screen being what
/// each render draws.
#[derive(Clone, Copy, Debug)]
pub enum PreviewSample {
    /// The 5,000-line [`markdown_sample`].
    Prose,
    /// A table of 400 rows under a heading, many screens tall: a render that
    /// shows one of its rows fetches and draws all of them.
    TallTable,
}

/// Rows of the [`PreviewSample::TallTable`] table.
const TALL_TABLE_ROWS: usize = 400;

/// Pipes on each row of the tall table, each one conceal.
const PIPES_PER_ROW: usize = 4;

impl PreviewSample {
    fn text(self) -> String {
        match self {
            Self::Prose => markdown_sample(5_000),
            Self::TallTable => {
                let mut text =
                    String::from("## Table\n\n| name | value | note |\n|------|-------|------|\n");
                for n in 0..TALL_TABLE_ROWS {
                    text.push_str(&format!("| row {n} | {n} | *text* |\n"));
                }
                text.push_str("\nAfter the table.\n");
                text
            }
        }
    }
}

/// The text [`colorizer_bench`] opens. Every sample but the minified one is
/// 5,000 lines.
#[derive(Clone, Copy, Debug)]
pub enum ColorizerSample {
    /// Prose with no color literal, in a `.txt` file.
    Prose,
    /// One `#rrggbb` per line, each line's color distinct, in a `.txt` file.
    Hex,
    /// One `oklch(<lightness> 0.4 <hue>)` per line, outside sRGB, in a `.txt`
    /// file. The hue is distinct on each line and the lightness cycles over
    /// 60%-80%: at one lightness every hue maps onto one sRGB boundary curve
    /// of about 1,250 colors, so a page of lines can show none the session
    /// has not styled.
    Oklch,
    /// One minified CSS line of this many chars, rules with a hex literal, an
    /// `rgb()` call and a color name each, then two short rules, in a `.css`
    /// file so names are looked up.
    Minified(usize),
    /// A [`Self::Minified`] line of the plugin's default render budget on
    /// every line, opened with that budget, so a render spends it all on the
    /// densest content.
    LongLines,
    /// Prose lines of this many chars with no color literal, in a `.txt`
    /// file: a render walks the whole screen, the chars
    /// [`PluginRenderBench::walked_chars`] counts.
    Screen(usize),
}

/// Lines of every [`ColorizerSample`] but the minified one.
const COLORIZER_LINES: usize = 5_000;

/// The rule a [`ColorizerSample::Minified`] line repeats.
const MINIFIED_RULE: &str = ".a{color:#fff;background:rgb(1,2,3);border-color:red}";

/// The default of `core:colorizer`'s `'max-render-chars`, read from the
/// plugin's own config call.
fn colorizer_default_budget() -> usize {
    const PLUGIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../runtime/plugins/core/colorizer/plugin.scm"
    ));
    let (_, after) = PLUGIN
        .split_once("'max-render-chars ")
        .expect("plugin.scm reads 'max-render-chars");
    after
        .split_whitespace()
        .next()
        .and_then(|default| default.parse().ok())
        .expect("'max-render-chars has a number for its default")
}

/// `chars` minified CSS chars: [`MINIFIED_RULE`] repeated, padded with
/// spaces.
fn minified_line(chars: usize) -> String {
    let mut line = MINIFIED_RULE.repeat(chars / MINIFIED_RULE.len());
    line.push_str(&" ".repeat(chars - line.len()));
    line
}

/// [`ColorizerSample::Prose`]'s line `n`, without its newline.
fn prose_line(n: usize) -> String {
    format!("Line {n} of plain prose, with no literal in it and only the number {n}.")
}

impl ColorizerSample {
    fn file_name(self) -> String {
        match self {
            Self::Prose => "prose.txt".into(),
            Self::Hex => "hex.txt".into(),
            Self::Oklch => "oklch.txt".into(),
            Self::Minified(chars) => format!("minified_{chars}.css"),
            Self::LongLines => "long_lines.css".into(),
            Self::Screen(chars) => format!("screen_{chars}.txt"),
        }
    }

    fn text(self) -> String {
        let lines =
            |line: &dyn Fn(usize) -> String| (0..COLORIZER_LINES).map(line).collect::<String>();
        match self {
            Self::Prose => lines(&|n| format!("{}\n", prose_line(n))),
            // An odd step is invertible modulo 2^24, so no two lines share a color.
            Self::Hex => lines(&|n| {
                format!(
                    "Line {n} sets the color #{:06x} on item {n}.\n",
                    n * 0x01_0101 % 0x100_0000
                )
            }),
            Self::Oklch => lines(&|n| {
                let lightness = 60 + n % 21;
                let hue = n as f64 * 360.0 / COLORIZER_LINES as f64;
                format!("Line {n} sets oklch({lightness}% 0.4 {hue:.3}) on item {n}.\n")
            }),
            Self::Minified(chars) => {
                format!(
                    "{}\n.b{{color:blue}}\n.c{{margin:0}}\n",
                    minified_line(chars)
                )
            }
            Self::LongLines => {
                let line = minified_line(colorizer_default_budget());
                lines(&|_| format!("{line}\n"))
            }
            Self::Screen(chars) => lines(&|n| {
                let line: String = prose_line(n).chars().cycle().take(chars).collect();
                format!("{line}\n")
            }),
        }
    }
}

/// A plugin a [`PluginRenderBench`] loads: the `init.scm` that loads it as a
/// user's does, and the store its render fills. `skips_painted` says its
/// render does nothing on a text and view it has painted, so a render it
/// is to time follows [`PluginRenderBench::touch`].
struct BenchPlugin {
    init: &'static str,
    store: PluginStore,
    skips_painted: bool,
}

/// The decoration store a plugin fills for the focused buffer, by the
/// plugin's decoration source.
#[derive(Clone, Copy)]
enum PluginStore {
    Conceals(&'static str),
    ExtraHighlights(&'static str),
}

impl PluginStore {
    /// How many entries the store holds for the focused buffer.
    fn count(self, editor: &Editor) -> usize {
        let bid = editor.focused_buffer_id();
        let decorations = &editor.state.config.decorations;
        match self {
            Self::Conceals(source) => decorations.conceals_for(source, bid).len(),
            Self::ExtraHighlights(source) => decorations.extra_highlights_for(source, bid).len(),
        }
    }

    /// Empties the store for the focused buffer.
    fn clear(self, editor: &mut Editor) {
        let bid = editor.focused_buffer_id();
        let decorations = &mut editor.state.config.decorations;
        match self {
            Self::Conceals(source) => decorations.set_conceals(source.into(), bid, Vec::new()),
            Self::ExtraHighlights(source) => {
                decorations.set_extra_highlights(source.into(), bid, Vec::new());
            }
        }
    }
}

const MARKDOWN_PREVIEW: BenchPlugin = BenchPlugin {
    init: "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:markdown\")\n",
    store: PluginStore::Conceals("markdown-preview"),
    skips_painted: false,
};

/// `core:colorizer`, with a render budget no sample reaches.
const COLORIZER: BenchPlugin = BenchPlugin {
    init: "(load-plugin! \"core:stdlib\")\n\
     (load-plugin! \"core:colorizer\" #:config (hash 'max-render-chars 1000000))\n",
    store: PluginStore::ExtraHighlights("colorizer"),
    skips_painted: true,
};

/// `core:colorizer` with its default render budget.
const COLORIZER_DEFAULT: BenchPlugin = BenchPlugin {
    init: "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:colorizer\")\n",
    store: PluginStore::ExtraHighlights("colorizer"),
    skips_painted: true,
};

/// How often, and how long apart, [`PluginRenderBench::settle_until`] looks
/// for the plugin's first render: it can wait on a worker thread's parse and
/// on a debounce, so setup polls for it.
const SETUP_TRIES: usize = 1_000;
const SETUP_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// A headless editor started as a user's would, with a plugin loaded by its
/// `init.scm`, showing a file the plugin has drawn. Built by
/// [`markdown_preview_bench`] or [`colorizer_bench`].
pub struct PluginRenderBench {
    editor: Editor,
}

/// A [`PluginRenderBench`] on `sample`, with `core:markdown` loaded
/// and the `markdown` and `markdown.inline` grammars read from
/// `<root>/data/grammars/`, where the caller installs them as PLUM does.
///
/// # Panics
/// Panics as [`PluginRenderBench::drawn`] does, or if the render drew less
/// than the whole tall table.
pub fn markdown_preview_bench(
    root: &Path,
    sample: PreviewSample,
    width: u16,
    height: u16,
) -> PluginRenderBench {
    let file_name = format!("{sample:?}.md");
    let bench = PluginRenderBench::drawn(
        root,
        &MARKDOWN_PREVIEW,
        &file_name,
        &sample.text(),
        (width, height),
        true,
    );
    if let PreviewSample::TallTable = sample {
        let conceals = MARKDOWN_PREVIEW.store.count(&bench.editor);
        assert!(
            conceals >= PIPES_PER_ROW * TALL_TABLE_ROWS,
            "the render drew {conceals} conceals, not the whole table"
        );
    }
    bench
}

/// A [`PluginRenderBench`] on `sample`, with `core:colorizer` loaded.
///
/// # Panics
/// Panics as [`PluginRenderBench::drawn`] does.
pub fn colorizer_bench(
    root: &Path,
    sample: ColorizerSample,
    width: u16,
    height: u16,
) -> PluginRenderBench {
    let plugin = match sample {
        ColorizerSample::LongLines => &COLORIZER_DEFAULT,
        _ => &COLORIZER,
    };
    PluginRenderBench::drawn(
        root,
        plugin,
        &sample.file_name(),
        &sample.text(),
        (width, height),
        !matches!(sample, ColorizerSample::Prose | ColorizerSample::Screen(_)),
    )
}

impl PluginRenderBench {
    /// Writes `text` to `<root>/<file_name>` and `plugin`'s `init.scm` to
    /// `<root>/config/init.scm`, then starts the editor on the file at
    /// `size`, width by height, with `<root>/data/` as its data dir and the
    /// repo's `runtime/`. Once the plugin has drawn, its store is emptied
    /// and refilled by one [`Self::render`]; `paints` says the sample has
    /// something to draw.
    ///
    /// # Panics
    /// Panics if the editor cannot start, a sample that `paints` is not
    /// drawn, a pending timer outlasts the wait, or the setup logs an error.
    fn drawn(
        root: &Path,
        plugin: &BenchPlugin,
        file_name: &str,
        text: &str,
        (width, height): (u16, u16),
        paints: bool,
    ) -> Self {
        let config = root.join("config");
        std::fs::create_dir_all(&config).expect("the config dir is made");
        std::fs::write(config.join("init.scm"), plugin.init).expect("init.scm is written");
        let file = root.join(file_name);
        std::fs::write(&file, text).expect("the bench file is written");
        let dirs = Dirs {
            config: Some(config),
            data: Some(root.join("data")),
            runtime: Some(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("hume-editor sits under the workspace root")
                    .join("runtime"),
            ),
            ..Dirs::none()
        };
        let mut editor = headless_editor(&file, dirs, ConfigSource::Default);
        editor.resize_terminal(width, height);
        let mut bench = Self { editor };
        bench.settle_until(|editor| !paints || plugin.store.count(editor) > 0);
        // The hook the timed loop fires renders on its own: with the store
        // emptied, only that render can refill it.
        plugin.store.clear(&mut bench.editor);
        if plugin.skips_painted {
            bench.touch();
        }
        bench.render();
        if paints {
            assert!(
                plugin.store.count(&bench.editor) > 0,
                "the viewport hook drew nothing"
            );
        }
        assert_setup_clean(&bench.editor);
        bench
    }

    /// Draws frames until `drawn` holds and no timer is left to fire inside
    /// a timed loop.
    fn settle_until(&mut self, drawn: impl Fn(&Editor) -> bool) {
        for _ in 0..SETUP_TRIES {
            self.editor.settle();
            self.editor.prepare_frame(&mut RenderContext::new());
            if drawn(&self.editor) && self.editor.timers.is_empty() {
                return;
            }
            std::thread::sleep(SETUP_PAUSE);
        }
        panic!("the plugin did not draw, or a timer stayed pending");
    }

    /// The focused pane's `on-viewport-change`, fired as its debounce timer
    /// fires it, and settled: one render of the visible lines.
    pub fn render(&mut self) {
        let pid = self.editor.view.focused_pane();
        self.editor.queue_viewport_change(pid);
        self.editor.settle();
    }

    /// Deletes the focused pane's selection and undoes it, then draws the
    /// frame: the text is as it was, at a new generation, so the next
    /// [`Self::render`] paints it in full.
    pub fn touch(&mut self) {
        for command in ["delete-selection", "undo"] {
            self.editor
                .execute_keymap_command(command, hume_core::Count::UNTYPED, false);
        }
        self.editor.settle();
        self.editor.prepare_frame(&mut RenderContext::new());
    }

    /// [`Self::render`], then the frame that draws what it set.
    pub fn render_then_frame(&mut self) {
        self.render();
        std::hint::black_box(self.editor.prepare_frame(&mut RenderContext::new()));
    }

    /// How many whole pages of lines lie below the focused pane's viewport,
    /// a page being the pane's text height, the distance `page-down` scrolls.
    pub fn screens_below(&mut self) -> usize {
        let pid = self.editor.view.focused_pane();
        let height = usize::from(self.editor.state.pane_size(&self.editor.view, pid).height());
        let shown =
            super::geometry::shown_lines(&mut self.editor.state, &mut self.editor.view, pid)
                .visible;
        let bid = self.editor.view.panes[pid].buffer_id();
        let lines = self
            .editor
            .state
            .buffers
            .get(bid)
            .text()
            .content_line_count();
        lines.end_exclusive().lines_since(shown.end) / height
    }

    /// One `page-down`, the frame drawn after it, then
    /// [`Self::render_then_frame`]. Called while [`Self::screens_below`] is
    /// at least 1, every line the render paints is one the plugin has not
    /// painted before.
    pub fn scroll_render_then_frame(&mut self) {
        self.editor
            .execute_keymap_command("page-down", hume_core::Count::UNTYPED, false);
        self.editor.prepare_frame(&mut RenderContext::new());
        self.render_then_frame();
    }

    /// The chars `core:colorizer` walks to paint the focused pane with no
    /// line skipped and none known: every char of the lines it shows,
    /// newlines left out.
    pub fn walked_chars(&mut self) -> usize {
        let pid = self.editor.view.focused_pane();
        let shown =
            super::geometry::shown_lines(&mut self.editor.state, &mut self.editor.view, pid)
                .visible;
        let bid = self.editor.view.panes[pid].buffer_id();
        let text = self.editor.state.buffers.get(bid).text();
        shown
            .iter()
            .map(|line| {
                let lines = text.lines();
                lines
                    .newline(line)
                    .offset()
                    .chars_since(lines.start(line).offset())
            })
            .sum()
    }

    /// How many scope names the session's registry holds: it grows when a
    /// render names a scope the session has not seen.
    pub fn scope_count(&self) -> usize {
        self.editor.view.registry.len()
    }
}

/// The `on-paste` handlers an [`OnPasteBench`] registers.
#[derive(Clone, Copy, Debug)]
pub enum PasteHandlers {
    None,
    /// This many handlers that change nothing.
    Unchanged(usize),
    /// `core:markdown`'s, which makes a pasted URL a link.
    Markdown,
}

impl PasteHandlers {
    fn init(self) -> String {
        match self {
            Self::None => String::new(),
            Self::Unchanged(n) => {
                "(register-hook! 'on-paste (lambda (pane pasted previous) #f))\n".repeat(n)
            }
            Self::Markdown => MARKDOWN_PREVIEW.init.to_owned(),
        }
    }
}

/// An editor on a Markdown file with a word selected on each of its first
/// lines, and the `on-paste` handlers its `init.scm` registers.
pub struct OnPasteBench {
    editor: Editor,
    undo: Keys,
}

impl OnPasteBench {
    /// Writes a file of `selections` lines and the `init.scm` that registers
    /// `handlers` under `root`, starts the editor on the file with the
    /// `<root>/data/` data dir and the repo's `runtime/`, and selects the
    /// first word of every line.
    ///
    /// # Panics
    /// Panics if `selections` is zero, the editor cannot start, or the setup
    /// logs an error.
    pub fn new(root: &Path, handlers: PasteHandlers, selections: usize) -> Self {
        assert!(selections > 0, "a paste needs a selection");
        let config = root.join("config");
        std::fs::create_dir_all(&config).expect("the config dir is made");
        std::fs::write(config.join("init.scm"), handlers.init()).expect("init.scm is written");
        let file = root.join("words.md");
        let text: String = (0..selections)
            .map(|n| format!("word{n} and the rest of line {n}\n"))
            .collect();
        std::fs::write(&file, text).expect("the bench file is written");
        let dirs = Dirs {
            config: Some(config),
            data: Some(root.join("data")),
            runtime: Some(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("hume-editor sits under the workspace root")
                    .join("runtime"),
            ),
            ..Dirs::none()
        };
        let mut editor = headless_editor(&file, dirs, ConfigSource::Default);
        editor.settle();
        let copies = match selections {
            1 => String::new(),
            n => format!("{}C", n - 1),
        };
        Keys::parse(&format!("{UNDO_LEVELS}{copies}miw")).press(&mut editor);
        assert_setup_clean(&editor);
        Self {
            editor,
            undo: Keys::parse("u"),
        }
    }

    /// How many selections the focused pane has.
    pub fn selection_count(&self) -> usize {
        self.editor.current_view().len()
    }

    /// The first line once `text` is pasted, before the paste is undone.
    pub fn first_line_pasting(&mut self, text: &str) -> String {
        self.paste(text);
        let pasted = self.editor.doc().text().to_string();
        self.undo.press(&mut self.editor);
        pasted.lines().next().unwrap_or_default().to_owned()
    }

    /// A terminal paste of `text` over every selection, then its undo.
    pub fn paste_then_undo(&mut self, text: &str) {
        self.paste(text);
        self.undo.press(&mut self.editor);
    }

    fn paste(&mut self, text: &str) {
        self.editor
            .handle_input(termina::event::Event::Paste(text.to_owned()));
        self.editor.settle();
    }
}
