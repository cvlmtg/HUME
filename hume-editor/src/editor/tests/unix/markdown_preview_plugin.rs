// core:markdown-preview: end-to-end plugin tests.
//
// Loads the real plugin from the repo's `runtime/` tree onto a full-chrome
// editor holding the markdown and markdown.inline grammar fixtures, opens a
// `.md` file as startup would, and asserts the decoration stores the
// plugin's setters fill. Expected offsets are counted by hand from each
// fixture text. The frame tests pin whole drawn frames as snapshots.

use super::*;

use hume_core::OptionalQuery;
use hume_decorations::{ConcealEntrySpan, EolTextEntry, InlayHintEntry, VirtualLineEntry};
use hume_engine::pane::WrapMode;
use hume_grid::Rect;
use hume_rope::line::ContentLine;
use hume_scripting::ScriptingHost;
use test_fixtures::{require_grammars, require_helix_query};

use super::super::render_snapshot::render_to_styled_string;

const SOURCE: &str = "markdown-preview";

const WIDTH: u16 = 80;
const HEIGHT: u16 = 25;

/// An editor with the plugin loaded lazily, as `init.scm.example` loads it,
/// the clock held so each debounce runs out only when a test says, and
/// frames drawn at `size`.
struct Preview {
    ed: Editor,
    tmp: tempfile::TempDir,
    size: (u16, u16),
    _dirs: RealRuntimeDirs,
}

/// The markdown fixture's queries as PLUM installs them: highlights,
/// injections, context and folds.
fn markdown_queries() -> QueryFiles {
    [OptionalQuery::Context, OptionalQuery::Folds]
        .into_iter()
        .fold(markdown_query_files(), |files, kind| {
            files.with(kind, require_helix_query("markdown", kind.file_name()))
        })
}

/// `config` is the Steel source of the plugin's `#:config`; `inline` gives
/// `markdown.inline` its grammar.
fn preview_with(config: Option<&str>, inline: bool) -> Preview {
    require_grammars(&["markdown", "markdown.inline"]);
    let dirs = RealRuntimeDirs::new();
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(
        None,
        crate::editor::run::HEADLESS_TERMINAL,
        no_op_wake(),
        dirs.dirs(),
        test_cwd(),
    )
    .unwrap();
    ed.state
        .config
        .languages
        .register_identity("markdown", &["md"], &[], &[])
        .unwrap();
    attach_fixture_grammar_with(
        &mut ed,
        "markdown",
        "tree_sitter_markdown",
        &markdown_queries(),
    );
    if inline {
        ed.state
            .config
            .languages
            .register_identity("markdown.inline", &[], &[], &[])
            .unwrap();
        attach_fixture_grammar(&mut ed, "markdown.inline", "tree_sitter_markdown_inline");
    }
    let config = config.map(|c| format!(" #:config {c}")).unwrap_or_default();
    let mut host = ScriptingHost::for_testing(&ed.state.dirs);
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:markdown-preview\"{config})"
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.state.clock.hold();
    ed.view.theme = crate::testing::build_snapshot_theme();
    Preview {
        ed,
        tmp,
        size: (WIDTH, HEIGHT),
        _dirs: dirs,
    }
}

fn preview() -> Preview {
    preview_with(None, true)
}

impl Preview {
    /// Draws every frame at `width` x `height` cells.
    fn sized(self, width: u16, height: u16) -> Self {
        Self {
            size: (width, height),
            ..self
        }
    }

    /// Opens `text` as `<tmp>/<name>` in the focused pane.
    fn open(&mut self, name: &str, text: &str) -> BufferId {
        let path = self.tmp.path().join(name);
        std::fs::write(&path, text).unwrap();
        self.ed
            .execute_typed("e", Some(path.to_str().unwrap()))
            .unwrap();
        self.ed.focused_buffer_id()
    }

    /// Draws frames until the focused buffer's tree matches its text (a
    /// parse runs on a worker thread), then lets every pending debounce run
    /// out, three times over: the syntax hook arms the render debounce, the
    /// viewport hook renders, and the debounced render renders again. No
    /// hook may have raised on the way.
    fn render(&mut self) {
        self.render_logging();
        assert_eq!(error_log(&self.ed), Vec::<String>::new());
    }

    /// [`Self::render`] without the check that no hook raised.
    fn render_logging(&mut self) {
        let bid = self.ed.focused_buffer_id();
        let (width, height) = self.size;
        poll_until(
            &mut self.ed,
            |ed| {
                frame(ed, width, height);
                ed.state.clock.advance(Duration::from_millis(200));
                ed.settle();
            },
            |ed| ed.state.buffers.get(bid).current_layers().is_some(),
        );
        for _ in 0..3 {
            self.frame();
            self.wait();
        }
    }

    fn frame(&mut self) {
        frame(&mut self.ed, self.size.0, self.size.1);
    }

    /// The styled frame at `size`, drawn in the snapshot theme, without its
    /// statusline row: that row names the temp file's path.
    fn snapshot(&mut self) -> String {
        let (width, height) = self.size;
        render_to_styled_string(&mut self.ed, Rect::new(0, 0, width, height))
            .lines()
            .take(usize::from(height) - 1)
            .map(|row| format!("{row}\n"))
            .collect()
    }

    /// Moves the held clock past every debounce and settles.
    fn wait(&mut self) {
        self.ed.state.clock.advance(Duration::from_millis(200));
        self.ed.settle();
    }

    fn conceals(&self, bid: BufferId) -> Vec<Conceal> {
        let ed = &self.ed;
        ed.state
            .config
            .decorations
            .conceals_for(SOURCE, bid)
            .iter()
            .map(|entry| {
                let text = match &entry.span {
                    ConcealEntrySpan::Hidden { text, .. } => text.as_ref(),
                    ConcealEntrySpan::Inserted { text, .. } => Some(text),
                };
                let (start, end) = (entry.chars().start.index(), entry.chars().end.index());
                Conceal {
                    start,
                    end,
                    text: text.map(|t| t.text.to_string()),
                    scope: text
                        .and_then(|t| t.scope)
                        .map(|s| scope_name(ed, s).to_owned()),
                    reveal: entry.reveal.map(|r| (r.start.index(), r.end.index())),
                }
            })
            .collect()
    }

    /// The start of each concealed range, for tests that only ask which
    /// elements were set.
    fn conceal_starts(&self, bid: BufferId) -> Vec<usize> {
        self.conceals(bid).iter().map(|c| c.start).collect()
    }

    /// `(line, scope)` of each line background, sorted by line.
    fn bands(&self, bid: BufferId) -> Vec<(usize, String)> {
        let mut v: Vec<_> = self
            .ed
            .state
            .config
            .decorations
            .line_backgrounds_for(SOURCE, bid)
            .iter()
            .map(|e| (e.line.index(), scope_name(&self.ed, e.scope).to_owned()))
            .collect();
        v.sort();
        v
    }

    /// `(start, end, scope)` of each extra highlight, sorted.
    fn highlights(&self, bid: BufferId) -> Vec<(usize, usize, String)> {
        let mut v: Vec<_> = self
            .ed
            .state
            .config
            .decorations
            .extra_highlights_for(SOURCE, bid)
            .iter()
            .map(|e| {
                (
                    e.start.index(),
                    e.end.index(),
                    scope_name(&self.ed, e.scope).to_owned(),
                )
            })
            .collect();
        v.sort();
        v
    }

    fn is_blank(&self, bid: BufferId) -> bool {
        self.conceals(bid).is_empty()
            && self.bands(bid).is_empty()
            && self.highlights(bid).is_empty()
    }
}

/// One stored conceal: its char range (an inserted one is empty at its
/// position), its text and scope name, and its reveal span in chars.
#[derive(Debug, PartialEq)]
struct Conceal {
    start: usize,
    end: usize,
    text: Option<String>,
    scope: Option<String>,
    reveal: Option<(usize, usize)>,
}

fn replaced(start: usize, end: usize, text: &str, scope: Option<&str>) -> Conceal {
    Conceal {
        start,
        end,
        text: Some(text.to_owned()),
        scope: scope.map(str::to_owned),
        reveal: None,
    }
}

fn hidden(start: usize, end: usize) -> Conceal {
    Conceal {
        start,
        end,
        text: None,
        scope: None,
        reveal: None,
    }
}

fn inserted(at: usize, text: &str, scope: &str) -> Conceal {
    replaced(at, at, text, Some(scope))
}

impl Conceal {
    fn revealing(self, start: usize, end: usize) -> Self {
        Self {
            reveal: Some((start, end)),
            ..self
        }
    }
}

fn band(line: usize, scope: &str) -> (usize, String) {
    (line, scope.to_owned())
}

fn bands(lines: std::ops::Range<usize>, scope: &str) -> Vec<(usize, String)> {
    lines.map(|line| band(line, scope)).collect()
}

fn highlight(start: usize, end: usize, scope: &str) -> (usize, usize, String) {
    (start, end, scope.to_owned())
}

// ── Headings ──────────────────────────────────────────────────────────

#[test]
fn plugin_activates_on_a_markdown_buffer_with_no_events_clause() {
    let mut p = preview();
    let bid = p.open("a.md", "# Title\n");
    p.render();
    assert_eq!(p.conceal_starts(bid), [0]);
}

#[test]
fn atx_heading_marker_becomes_its_level_with_band_and_text_highlight() {
    let mut p = preview();
    // Line starts: 0, 4, 9, 15, 22, 30; each marker is followed by one
    // space and a one-letter text.
    let bid = p.open("a.md", "# a\n## b\n### c\n#### d\n##### e\n###### f\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, " 1 ", Some("markup.heading.1.marker")),
            replaced(4, 6, " 2 ", Some("markup.heading.2.marker")),
            replaced(9, 12, " 3 ", Some("markup.heading.3.marker")),
            replaced(15, 19, " 4 ", Some("markup.heading.4.marker")),
            replaced(22, 27, " 5 ", Some("markup.heading.5.marker")),
            replaced(30, 36, " 6 ", Some("markup.heading.6.marker")),
        ]
    );
    assert_eq!(
        p.bands(bid),
        (1..=6)
            .map(|n| band(n - 1, &format!("markup.heading.{n}.line")))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        p.highlights(bid),
        [
            highlight(2, 3, "markup.heading.1.text"),
            highlight(7, 8, "markup.heading.2.text"),
            highlight(13, 14, "markup.heading.3.text"),
            highlight(20, 21, "markup.heading.4.text"),
            highlight(28, 29, "markup.heading.5.text"),
            highlight(37, 38, "markup.heading.6.text"),
        ]
    );
}

#[test]
fn setext_heading_gets_band_and_bold_only() {
    let mut p = preview();
    // "Title\n" 0..6, "=====\n" 6..12, "\n" 12, "Sub\n" 13..17, "---\n" 17..21.
    let bid = p.open("a.md", "Title\n=====\n\nSub\n---\n");
    p.render();
    assert_eq!(p.conceals(bid), []);
    assert_eq!(
        p.bands(bid),
        [
            band(0, "markup.heading.1.line"),
            band(1, "markup.heading.1.line"),
            band(3, "markup.heading.2.line"),
            band(4, "markup.heading.2.line"),
        ]
    );
    assert_eq!(
        p.highlights(bid),
        [
            highlight(0, 5, "markup.heading.1.text"),
            highlight(13, 16, "markup.heading.2.text"),
        ]
    );
}

// ── Lists and quotes ──────────────────────────────────────────────────

#[test]
fn bullet_becomes_a_dot() {
    let mut p = preview();
    // "- a\n" 0..4, "* b\n" 4..8, "+ c\n" 8..12, "1. d\n" 12..17: the
    // ordered marker stays raw.
    let bid = p.open("a.md", "- a\n* b\n+ c\n1. d\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, "•", None),
            replaced(4, 5, "•", None),
            replaced(8, 9, "•", None),
        ]
    );
}

#[test]
fn task_boxes_become_checkboxes() {
    let mut p = preview();
    // "- [ ] a\n" 0..8, "- [x] b\n" 8..16: bullet and box become one glyph,
    // the space after the box stays.
    let bid = p.open("a.md", "- [ ] a\n- [x] b\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 5, "☐", Some("markup.list.unchecked")),
            replaced(8, 13, "☑", Some("markup.list.checked")),
        ]
    );
}

#[test]
fn quote_marker_becomes_a_bar() {
    let mut p = preview();
    // "> a\n" 0..4, "> > b\n" 4..10: one `>` per quote level, the space
    // after each kept.
    let bid = p.open("a.md", "> a\n> > b\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, "┃", Some("markup.quote")),
            replaced(4, 5, "┃", Some("markup.quote")),
            replaced(6, 7, "┃", Some("markup.quote")),
        ]
    );
}

#[test]
fn quote_line_without_a_marker_gets_an_inserted_bar() {
    let mut p = preview();
    // "> a\n" 0..4, the lazy line "b\n" 4..6, "- > c\n" 6..12, the lazy
    // line "  d\n" 12..16 whose text starts at 14.
    let bid = p.open("a.md", "> a\nb\n- > c\n  d\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, "┃", Some("markup.quote")),
            inserted(4, "┃ ", "markup.quote"),
            replaced(6, 7, "•", None),
            replaced(8, 9, "┃", Some("markup.quote")),
            inserted(14, "┃ ", "markup.quote"),
        ]
    );
}

#[test]
fn lazy_line_of_a_nested_quote_gets_one_bar() {
    let mut p = preview();
    // "> > a\n" 0..6, the lazy line "b\n" 6..8, inside both quotes.
    let bid = p.open("a.md", "> > a\nb\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, "┃", Some("markup.quote")),
            replaced(2, 3, "┃", Some("markup.quote")),
            inserted(6, "┃ ", "markup.quote"),
        ]
    );
}

// ── Rules and fences ──────────────────────────────────────────────────

#[test]
fn horizontal_rule_is_rule_width_dashes() {
    let mut p = preview_with(Some("(hash 'rule-width 10)"), true);
    // "a\n" 0..2, "\n" 2, "___\n" 3..7, "\n" 7, "***\n" 8..12: each rule
    // up to its line's end, never its line break.
    let bid = p.open("a.md", "a\n\n___\n\n***\n");
    p.render();
    let rule = "─".repeat(10);
    assert_eq!(
        p.conceals(bid),
        [
            replaced(3, 6, &rule, Some("punctuation.special")),
            replaced(8, 11, &rule, Some("punctuation.special")),
        ]
    );
}

#[test]
fn fence_lines_hide_their_fence_chars_keep_the_info_string_and_band_the_block() {
    let mut p = preview();
    // "```rust\n" 0..8, "fn f() {}\n" 8..18, "```\n" 18..22.
    let bid = p.open("a.md", "```rust\nfn f() {}\n```\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            hidden(0, 3).revealing(0, 22),
            hidden(18, 21).revealing(0, 22)
        ]
    );
    assert_eq!(p.bands(bid), bands(0..3, "markup.raw.block.line"));
    assert_eq!(p.highlights(bid), []);
}

#[test]
fn an_unclosed_fence_runs_to_the_end_of_the_document() {
    let mut p = preview();
    // "```rust\n" 0..8, "fn f() {}\n" 8..18, "# h\n" 18..22: no closing fence,
    // so the heading line is code.
    let bid = p.open("a.md", "```rust\nfn f() {}\n# h\n");
    p.render();
    assert_eq!(p.conceals(bid), [hidden(0, 3).revealing(0, 22)]);
    assert_eq!(p.bands(bid), bands(0..3, "markup.raw.block.line"));
    assert_eq!(p.highlights(bid), []);
}

#[test]
fn markdown_fence_sets_conceals_for_its_two_fence_lines_only() {
    let mut p = preview();
    // "```markdown\n" 0..12, "# h\n" 12..16, "**b**\n" 16..22, "- c\n"
    // 22..26, "```\n" 26..30.
    let bid = p.open("a.md", "```markdown\n# h\n**b**\n- c\n```\n");
    p.render();
    let markdown = p.ed.state.config.languages.intern("markdown");
    assert!(
        p.ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .and_then(|syn| syn.layers())
            .is_some_and(|layers| layers
                .layers
                .iter()
                .any(|l| l.depth > 0 && l.bundle.language == markdown)),
        "premise: the fence's content is parsed as an injected markdown layer"
    );
    assert_eq!(
        p.conceals(bid),
        [
            hidden(0, 3).revealing(0, 30),
            hidden(26, 29).revealing(0, 30)
        ]
    );
    assert_eq!(p.bands(bid), bands(0..5, "markup.raw.block.line"));
    assert_eq!(p.highlights(bid), []);
}

#[test]
fn quote_lines_inside_a_code_block_get_no_inserted_bar() {
    let mut p = preview();
    // "> ```\n" 0..6, "> code\n" 6..13, "> ```\n" 13..19. Every line has its
    // marker, so none gets an inserted bar.
    let bid = p.open("a.md", "> ```\n> code\n> ```\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, "┃", Some("markup.quote")),
            hidden(2, 5).revealing(0, 19),
            replaced(6, 7, "┃", Some("markup.quote")),
            replaced(13, 14, "┃", Some("markup.quote")),
            hidden(15, 18).revealing(0, 19),
        ]
    );
}

#[test]
fn every_line_of_a_code_block_inside_a_quote_gets_its_bar() {
    let mut p = preview();
    // "> ```\n" 0..6, "> a\n" 6..10, "> b\n" 10..14, "> ```\n" 14..20: the
    // markers of the lines after the first content line lie inside the
    // block's content.
    let bid = p.open("a.md", "> ```\n> a\n> b\n> ```\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            replaced(0, 1, "┃", Some("markup.quote")),
            hidden(2, 5).revealing(0, 20),
            replaced(6, 7, "┃", Some("markup.quote")),
            replaced(10, 11, "┃", Some("markup.quote")),
            replaced(14, 15, "┃", Some("markup.quote")),
            hidden(16, 19).revealing(0, 20),
        ]
    );
}

#[test]
fn a_quoted_code_block_inside_a_markdown_fence_sets_no_bar() {
    let mut p = preview();
    // "````markdown\n" 0..13, "> ```\n" 13..19, "> x\n" 19..23, "> ```\n"
    // 23..29, "````\n" 29..34: the quote and its code block are the
    // injected layer's.
    let bid = p.open("a.md", "````markdown\n> ```\n> x\n> ```\n````\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            hidden(0, 4).revealing(0, 34),
            hidden(29, 33).revealing(0, 34)
        ]
    );
}

// ── Inline elements ───────────────────────────────────────────────────

#[test]
fn emphasis_strikethrough_and_code_span_delimiters_are_hidden_revealing_over_their_construct() {
    let mut p = preview();
    // One paragraph: "x *a*\n" 0..6, "**b\n" 6..10, "c** ~~d~~ `e`\n"
    // 10..24. `*a*` lies on line 0, the strong emphasis on lines 1 and 2,
    // the strikethrough (two nested ones, a delimiter each side) and the
    // code span on line 2.
    let bid = p.open("a.md", "x *a*\n**b\nc** ~~d~~ `e`\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            hidden(2, 3).revealing(0, 6),
            hidden(4, 5).revealing(0, 6),
            hidden(6, 7).revealing(6, 24),
            hidden(7, 8).revealing(6, 24),
            hidden(11, 12).revealing(6, 24),
            hidden(12, 13).revealing(6, 24),
            hidden(14, 15).revealing(10, 24),
            hidden(15, 16).revealing(10, 24),
            hidden(17, 18).revealing(10, 24),
            hidden(18, 19).revealing(10, 24),
            hidden(20, 21).revealing(10, 24),
            hidden(22, 23).revealing(10, 24),
        ]
    );
    assert_eq!(p.bands(bid), []);
    assert_eq!(p.highlights(bid), []);
}

#[test]
fn inline_link_shows_its_text_only() {
    let mut p = preview();
    // `[t](http://x "ti")` 0..18 keeps `t` at 1; `![i](y)` 19..26 keeps
    // `i` at 21.
    let bid = p.open("a.md", "[t](http://x \"ti\") ![i](y)\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [hidden(0, 1), hidden(2, 18), hidden(19, 21), hidden(22, 26)]
    );
}

#[test]
fn reference_links_show_their_text_only() {
    let mut p = preview();
    // `[t][r]` 0..6 keeps `t` at 1; `[c][]` 7..12 keeps `c` at 8.
    let bid = p.open("a.md", "[t][r] [c][]\n\n[r]: u\n[c]: v\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [hidden(0, 1), hidden(2, 6), hidden(7, 8), hidden(9, 12)]
    );
}

#[test]
fn shortcut_link_stays_raw() {
    let mut p = preview();
    // `[s]` 0..3 has a definition and stays raw; `[t](u)` 4..10 is the
    // inline link beside it.
    let bid = p.open("a.md", "[s] [t](u)\n\n[s]: v\n");
    p.render();
    assert_eq!(p.conceals(bid), [hidden(4, 5), hidden(6, 10)]);
}

#[test]
fn autolink_hides_its_brackets() {
    let mut p = preview();
    // `<http://a.b>` 0..12, `<me@x.y>` 13..21.
    let bid = p.open("a.md", "<http://a.b> <me@x.y>\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [hidden(0, 1), hidden(11, 12), hidden(13, 14), hidden(20, 21)]
    );
}

#[test]
fn backslash_escape_hides_the_backslash() {
    let mut p = preview();
    // `\*x\*`: backslashes at 0 and 3.
    let bid = p.open("a.md", "\\*x\\*\n");
    p.render();
    assert_eq!(p.conceals(bid), [hidden(0, 1), hidden(3, 4)]);
}

/// `cases` joined by single spaces into one line, and the replacement each
/// one is expected to get, at its offset in that line.
fn entity_line(cases: &[(&str, Option<&str>)]) -> (String, Vec<Conceal>) {
    let mut expected = Vec::new();
    let mut at = 0;
    for (source, text) in cases {
        if let Some(text) = text {
            expected.push(replaced(at, at + source.len(), text, None));
        }
        at += source.len() + 1;
    }
    let line = cases.iter().map(|(source, _)| *source).collect::<Vec<_>>();
    (format!("{}\n", line.join(" ")), expected)
}

#[test]
fn entities_are_replaced() {
    let mut p = preview();
    let (text, expected) = entity_line(&[
        ("&amp;", Some("&")),
        ("&lt;", Some("<")),
        ("&gt;", Some(">")),
        ("&quot;", Some("\"")),
        ("&apos;", Some("'")),
        ("&nbsp;", Some("\u{a0}")),
        ("&copy;", Some("©")),
        ("&reg;", Some("®")),
        ("&trade;", Some("™")),
        ("&mdash;", Some("—")),
        ("&ndash;", Some("–")),
        ("&hellip;", Some("…")),
        ("&#65;", Some("A")),
        ("&#x41;", Some("A")),
        ("&#X41;", Some("A")),
    ]);
    let bid = p.open("a.md", &text);
    p.render();
    assert_eq!(p.conceals(bid), expected);
}

#[test]
fn unknown_or_control_entities_stay_raw() {
    let mut p = preview();
    // `&euro;` is an entity outside the plugin's table; the numeric ones
    // name a line break, NUL, a C1 control, a surrogate and a value past
    // the last code point.
    let (text, expected) = entity_line(&[
        ("&#65;", Some("A")),
        ("&bogus;", None),
        ("&euro;", None),
        ("&#10;", None),
        ("&#0;", None),
        ("&#x9F;", None),
        ("&#xD800;", None),
        ("&#1114112;", None),
    ]);
    let bid = p.open("a.md", &text);
    p.render();
    assert_eq!(p.conceals(bid), expected);
}

#[test]
fn todo_footnote_note_and_dash_task_set_no_link_conceal() {
    let mut p = preview();
    // "[TODO] [^1] [x](u)\n" 0..19 with the inline link at 12..18, "\n"
    // 19, "> [!NOTE]\n" 20..30, "\n" 30, "- [-] item\n" 31..42.
    let bid = p.open("a.md", "[TODO] [^1] [x](u)\n\n> [!NOTE]\n\n- [-] item\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            hidden(12, 13),
            hidden(14, 18),
            replaced(20, 21, "┃", Some("markup.quote")),
            replaced(31, 32, "•", None),
        ]
    );
}

#[test]
fn nested_constructs_set_without_an_overlap_error() {
    let mut p = preview();
    // "[**b** x](u\* \"&amp;\")\n" 0..23: bold inside the link text, an
    // escape (11..13) and an entity (15..20) inside the hidden tail 8..22.
    // "[![i](y)](u)\n" 23..36: an image as the link's whole text.
    let bid = p.open("a.md", "[**b** x](u\\* \"&amp;\")\n[![i](y)](u)\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [
            hidden(0, 1),
            hidden(1, 2).revealing(0, 23),
            hidden(2, 3).revealing(0, 23),
            hidden(4, 5).revealing(0, 23),
            hidden(5, 6).revealing(0, 23),
            hidden(8, 22),
            hidden(23, 24),
            hidden(24, 26),
            hidden(27, 31),
            hidden(31, 35),
        ]
    );
}

// ── Tables ────────────────────────────────────────────────────────────

/// The scope of every table border.
const BORDER: &str = "punctuation.special";

/// A table conceal that replaces `start..end` with `text`.
fn bar(start: usize, end: usize, text: &str) -> Conceal {
    replaced(start, end, text, Some(BORDER))
}

/// A table conceal inserted at `at`.
fn edge(at: usize, text: &str) -> Conceal {
    inserted(at, text, BORDER)
}

/// `conceals`, each revealing over the chars `start..end`.
fn revealing(conceals: Vec<Conceal>, start: usize, end: usize) -> Vec<Conceal> {
    conceals
        .into_iter()
        .map(|c| c.revealing(start, end))
        .collect()
}

#[test]
fn table_cells_are_padded_to_the_widest_cell_after_inline_conceals() {
    let mut p = preview();
    // "| a | **bb** |\n" 0..15, "|---|---|\n" 15..25, "| ccc | d |\n"
    // 25..37. Shown widths: `a` 1, `bb` 2 (its four `*` hidden), `ccc` 3,
    // `d` 1, so the columns are 3 and 2 wide.
    let bid = p.open("a.md", "| a | **bb** |\n|---|---|\n| ccc | d |\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                bar(3, 6, "   │ "),
                hidden(6, 7),
                hidden(7, 8),
                hidden(10, 11),
                hidden(11, 12),
                bar(12, 14, " │"),
                bar(15, 24, "├─────┼────┤"),
                bar(25, 27, "│ "),
                bar(30, 33, " │ "),
                bar(34, 36, "  │"),
            ],
            0,
            37
        )
    );
}

#[test]
fn bold_inside_a_link_inside_a_table_cell_sets_without_an_overlap_error() {
    let mut p = preview();
    // "| [**b**](u) | x |\n" 0..19, "|---|---|\n" 19..29. The link
    // `[**b**](u)` is 2..12 and shows `b`, so both columns are 1 wide.
    let bid = p.open("a.md", "| [**b**](u) | x |\n|---|---|\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                hidden(2, 3),
                hidden(3, 4),
                hidden(4, 5),
                hidden(6, 7),
                hidden(7, 8),
                hidden(8, 12),
                bar(12, 15, " │ "),
                bar(16, 18, " │"),
                bar(19, 28, "├───┼───┤"),
            ],
            0,
            29
        )
    );
}

#[test]
fn a_table_inside_a_quote_keeps_its_quote_bars() {
    let mut p = preview();
    // "> | a | b |\n" 0..12, "> |---|---|\n" 12..24, "> | c | d |\n" 24..36: the
    // table's cells start after each line's marker.
    let bid = p.open("a.md", "> | a | b |\n> |---|---|\n> | c | d |\n");
    p.render();
    let quote = |at| replaced(at, at + 1, "┃", Some("markup.quote"));
    assert_eq!(
        p.conceals(bid),
        [
            quote(0),
            bar(2, 4, "│ ").revealing(0, 36),
            bar(5, 8, " │ ").revealing(0, 36),
            bar(9, 11, " │").revealing(0, 36),
            quote(12),
            bar(14, 23, "├───┼───┤").revealing(0, 36),
            quote(24),
            bar(26, 28, "│ ").revealing(0, 36),
            bar(29, 32, " │ ").revealing(0, 36),
            bar(33, 35, " │").revealing(0, 36),
        ]
    );
}

#[test]
fn delimiter_row_becomes_a_box_rule() {
    let mut p = preview();
    // "| a | b |\n" 0..10, "|---|---|\n" 10..20: two columns one cell wide.
    let bid = p.open("a.md", "| a | b |\n|---|---|\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                bar(3, 6, " │ "),
                bar(7, 9, " │"),
                bar(10, 19, "├───┼───┤"),
            ],
            0,
            20
        )
    );
}

#[test]
fn alignment_comes_from_delimiter_colons() {
    let mut p = preview();
    // "| l | c | r |\n" 0..14, "|:--|:-:|--:|\n" 14..28,
    // "| xxx | yyy | zzz |\n" 28..48. Every column is 3 wide, so each
    // header cell has 2 cells of padding: after `l`, one each side of `c`,
    // before `r`.
    let bid = p.open(
        "a.md",
        "| l | c | r |\n|:--|:-:|--:|\n| xxx | yyy | zzz |\n",
    );
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                bar(3, 6, "   │  "),
                bar(7, 10, "  │   "),
                bar(11, 13, " │"),
                bar(14, 27, "├─────┼─────┼─────┤"),
                bar(28, 30, "│ "),
                bar(33, 36, " │ "),
                bar(39, 42, " │ "),
                bar(45, 47, " │"),
            ],
            0,
            48
        )
    );
}

#[test]
fn empty_cell_escaped_pipe_inline_conceal_and_entity_inside_a_cell() {
    let mut p = preview();
    // "| a |  | c\|d |\n" 0..16: cells `a` (2), a blank one (5..7), `c\|d`
    // (9..13) with its backslash at 10. "|---|---|---|\n" 16..30.
    // "|*e*||&amp;|\n" 30..43: cells `*e*` (31..34), an empty one at 35,
    // `&amp;` (36..41). Shown widths: `a` and `e` 1, both blanks 0, `c|d`
    // 3 and `&` 1.
    let bid = p.open("a.md", "| a |  | c\\|d |\n|---|---|---|\n|*e*||&amp;|\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                bar(3, 5, " │ "),
                bar(5, 9, " │ "),
                hidden(10, 11),
                bar(13, 15, " │"),
                bar(16, 29, "├───┼──┼─────┤"),
                bar(30, 31, "│ "),
                hidden(31, 32),
                hidden(33, 34),
                bar(34, 35, " │ "),
                bar(35, 36, " │ "),
                replaced(36, 41, "&", None),
                bar(41, 42, "   │"),
            ],
            0,
            43
        )
    );
}

#[test]
fn rows_without_outer_pipes_get_inserted_edge_conceals() {
    let mut p = preview();
    // "a | b\n" 0..6, "--|--:\n" 6..13, "c | dd\n" 13..20. The second
    // column is 2 wide and right-aligned, so `b` gets one cell before it.
    let bid = p.open("a.md", "a | b\n--|--:\nc | dd\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                edge(0, "│ "),
                bar(1, 4, " │  "),
                edge(5, " │"),
                bar(6, 12, "├───┼────┤"),
                edge(13, "│ "),
                bar(14, 17, " │ "),
                edge(19, " │"),
            ],
            0,
            20
        )
    );
}

#[test]
fn a_row_with_fewer_cells_gets_its_missing_ones_as_blanks() {
    let mut p = preview();
    // "| a | b |\n" 0..10, "|---|---|\n" 10..20, "| c |\n" 20..26: the last
    // row's edge draws the missing second column, one cell wide.
    let bid = p.open("a.md", "| a | b |\n|---|---|\n| c |\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                bar(3, 6, " │ "),
                bar(7, 9, " │"),
                bar(10, 19, "├───┼───┤"),
                bar(20, 22, "│ "),
                bar(23, 25, " │   │"),
            ],
            0,
            26
        )
    );
}

#[test]
fn every_table_conceal_reveals_over_the_whole_table() {
    let mut p = preview();
    // "x\n" 0..2, "\n" 2, the table "| a |\n|---|\n| *b* |\n" 3..23 on lines
    // 2..=4, "\n" 23, "y\n" 24..26: both borders, the cell bars, the rule
    // and the emphasis delimiters at 17 and 19.
    let bid = p.open("a.md", "x\n\n| a |\n|---|\n| *b* |\n\ny\n");
    p.render();
    let table = Some((3, 23));
    assert_eq!(
        p.conceals(bid)
            .iter()
            .map(|c| (c.start, c.reveal))
            .collect::<Vec<_>>(),
        [2, 3, 6, 9, 15, 17, 19, 20, 23].map(|start| (start, table))
    );
}

#[test]
fn table_borders_land_on_empty_neighbour_lines_with_the_tables_indent() {
    let mut p = preview();
    // "- item\n" 0..7, "\n" 7, "  | a |\n" 8..16, "  |---|\n" 16..24,
    // "  | b |\n" 24..32, "\n" 32, "z\n" 33..35. The table sits in the list
    // item, 2 cells in, on lines 2..=4.
    let bid = p.open("a.md", "- item\n\n  | a |\n  |---|\n  | b |\n\nz\n");
    p.render();
    let mut expected = vec![replaced(0, 1, "•", None)];
    expected.extend(revealing(
        vec![
            edge(7, "  ┌───┐"),
            bar(10, 12, "│ "),
            bar(13, 15, " │"),
            bar(18, 23, "├───┤"),
            bar(26, 28, "│ "),
            bar(29, 31, " │"),
            edge(32, "  └───┘"),
        ],
        8,
        32,
    ));
    assert_eq!(p.conceals(bid), expected);
}

#[test]
fn table_border_is_skipped_next_to_text_and_at_the_buffer_edges() {
    let mut p = preview();
    // "| a |\n|---|\n" 0..12 on the first line, "# h\n" 12..16 below it,
    // "\n" 16, "x\n" 17..19 above the second table "| b |\n|---|\n"
    // 19..31, which ends the buffer.
    let bid = p.open("a.md", "| a |\n|---|\n# h\n\nx\n| b |\n|---|\n");
    p.render();
    let mut expected = revealing(
        vec![bar(0, 2, "│ "), bar(3, 5, " │"), bar(6, 11, "├───┤")],
        0,
        12,
    );
    expected.push(replaced(12, 13, " 1 ", Some("markup.heading.1.marker")));
    expected.extend(revealing(
        vec![bar(19, 21, "│ "), bar(22, 24, " │"), bar(25, 30, "├───┤")],
        19,
        31,
    ));
    assert_eq!(p.conceals(bid), expected);
}

#[test]
fn one_empty_line_between_two_tables_carries_the_upper_tables_bottom_border() {
    let mut p = preview();
    // "|a|b|\n|-|-|\n|c|d|\n" 0..18, "\n" 18, "|e|\n|-|\n|f|\n" 19..31.
    let bid = p.open("a.md", "|a|b|\n|-|-|\n|c|d|\n\n|e|\n|-|\n|f|\n");
    p.render();
    let mut expected = revealing(
        vec![
            bar(0, 1, "│ "),
            bar(2, 3, " │ "),
            bar(4, 5, " │"),
            bar(6, 11, "├───┼───┤"),
            bar(12, 13, "│ "),
            bar(14, 15, " │ "),
            bar(16, 17, " │"),
            edge(18, "└───┴───┘"),
        ],
        0,
        18,
    );
    expected.extend(revealing(
        vec![
            bar(19, 20, "│ "),
            bar(21, 22, " │"),
            bar(23, 26, "├───┤"),
            bar(27, 28, "│ "),
            bar(29, 30, " │"),
        ],
        19,
        31,
    ));
    assert_eq!(p.conceals(bid), expected);
}

#[test]
fn cjk_and_combining_marks_in_cells_keep_the_columns_aligned() {
    let mut p = preview();
    // "| 漢字 | e\u{301} |\n" 0..12: `漢字` (2..4) is 4 cells wide, `é` (7..9,
    // a base and a combining mark) 1. "|---|---|\n" 12..22.
    // "| a | bb |\n" 22..33. The columns are 4 and 2 wide.
    let bid = p.open("a.md", "| 漢字 | e\u{301} |\n|---|---|\n| a | bb |\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        revealing(
            vec![
                bar(0, 2, "│ "),
                bar(4, 7, " │ "),
                bar(9, 11, "  │"),
                bar(12, 21, "├──────┼────┤"),
                bar(22, 24, "│ "),
                bar(25, 28, "    │ "),
                bar(30, 32, " │"),
            ],
            0,
            33
        )
    );
}

// ── Viewport render ───────────────────────────────────────────────────

/// `lines` lines, `# hN` on every tenth line `N` and `x` on the others:
/// every heading opens a section of ten lines.
fn sectioned(lines: usize) -> String {
    (0..lines)
        .map(|line| {
            if line % 10 == 0 {
                format!("# h{line}\n")
            } else {
                "x\n".to_owned()
            }
        })
        .collect()
}

/// The char offset each line of `text` starts at.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for line in text.split_inclusive('\n') {
        starts.push(starts.last().unwrap() + line.chars().count());
    }
    starts
}

/// The heading starts of [`sectioned`] text whose section shares a line
/// with `lines`.
fn sections_meeting(text: &str, lines: std::ops::Range<usize>) -> Vec<usize> {
    let starts = line_starts(text);
    (0..starts.len() - 1)
        .step_by(10)
        .filter(|&heading| heading < lines.end && lines.start < heading + 10)
        .map(|heading| starts[heading])
        .collect()
}

impl Preview {
    /// The lines pane `pid` showed on the last frame.
    fn visible(&self, pid: PaneId) -> std::ops::Range<usize> {
        let (_, range) = self.ed.last_visible_range[&pid];
        range.start.index()..range.end.index()
    }
}

#[test]
fn scroll_past_the_rendered_window_renders_the_new_lines_after_the_debounce() {
    let mut p = preview();
    let text = sectioned(100);
    let bid = p.open("a.md", &text);
    let pid = p.ed.view.focused_pane();
    p.render();
    let first = p.visible(pid);
    assert_eq!(
        p.conceal_starts(bid),
        sections_meeting(&text, first.clone())
    );

    p.ed.feed_key(key_ctrl('d'));
    p.ed.feed_key(key_ctrl('d'));
    p.frame();
    let second = p.visible(pid);
    assert!(
        second.start >= first.end,
        "premise: the view scrolled past {first:?}, to {second:?}"
    );
    assert_eq!(
        p.conceal_starts(bid),
        sections_meeting(&text, first),
        "the new lines wait for the debounce"
    );

    p.render();
    assert_eq!(p.conceal_starts(bid), sections_meeting(&text, second));
}

#[test]
fn a_fence_crossing_the_window_edge_is_set_whole() {
    let mut p = preview();
    // Lines 0..20 `x`, the fence on lines 20..=60, then `x` to line 70.
    let text: String = (0..71)
        .map(|line| match line {
            20 | 60 => "```\n",
            21..60 => "c\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let bid = p.open("a.md", &text);
    let pid = p.ed.view.focused_pane();
    p.render();
    let shown = p.visible(pid);
    assert!(
        shown.contains(&20) && !shown.contains(&60),
        "premise: {shown:?} shows the opening fence only"
    );
    let block = (starts[20], starts[61]);
    assert_eq!(
        p.conceals(bid),
        [
            hidden(starts[20], starts[20] + 3).revealing(block.0, block.1),
            hidden(starts[60], starts[60] + 3).revealing(block.0, block.1),
        ]
    );
    assert_eq!(p.bands(bid), bands(20..61, "markup.raw.block.line"));
}

#[test]
fn a_table_crossing_the_window_edge_is_set_whole_with_its_widths() {
    let mut p = preview();
    // `x` lines, a blank line 9, the table on lines 10..60, a blank line 60,
    // `x` lines to line 70. The header's first cell shows `hhhh` (4 wide),
    // every other cell one char.
    let text: String = (0..71)
        .map(|line| match line {
            9 | 60 => "\n",
            10 => "| **hhhh** | k |\n",
            11 => "|---|---|\n",
            12..60 => "| r | s |\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let bid = p.open("a.md", &text);
    let pid = p.ed.view.focused_pane();
    seek_to_line(&mut p.ed, 70);
    p.render();
    let shown = p.visible(pid);
    assert!(
        (11..60).contains(&shown.start),
        "premise: {shown:?} starts inside the table"
    );
    // The top border's line 9 lies above the render's range, so it gets
    // none: nothing in range says whether a table ends on line 8.
    let head = starts[10];
    let mut expected = vec![
        bar(head, head + 2, "│ "),
        hidden(head + 2, head + 3),
        hidden(head + 3, head + 4),
        hidden(head + 8, head + 9),
        hidden(head + 9, head + 10),
        bar(head + 10, head + 13, " │ "),
        bar(head + 14, head + 16, " │"),
        bar(starts[11], starts[11] + 9, "├──────┼───┤"),
    ];
    for &row in &starts[12..60] {
        expected.extend([
            bar(row, row + 2, "│ "),
            bar(row + 3, row + 6, "    │ "),
            bar(row + 7, row + 9, " │"),
        ]);
    }
    expected.push(edge(starts[60], "└──────┴───┘"));
    assert_eq!(p.conceals(bid), revealing(expected, starts[10], starts[60]));
}

#[test]
fn the_empty_line_between_two_tables_at_the_window_top_carries_the_upper_tables_border() {
    const SHARED: usize = 34;
    let mut p = preview();
    // `x` lines, a blank line 31, the table `| a |` on lines 32..=33, the
    // shared blank line 34, the table `| b |` on lines 35..=36, a blank
    // line 37, `x` lines to line 70.
    let text: String = (0..71)
        .map(|line| match line {
            31 | SHARED | 37 => "\n",
            32 => "| a |\n",
            35 => "| b |\n",
            33 | 36 => "|---|\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let bid = p.open("a.md", &text);
    let pid = p.ed.view.focused_pane();
    // `z k` puts the cursor line on the first row below the pinned header
    // row, which covers the line above it.
    run_set(&mut p.ed, "global scroll-margin=0").expect("the setting applies");
    seek_to_line(&mut p.ed, SHARED + 1);
    keys(&mut p.ed, "zk");
    p.render();
    assert_eq!(
        p.visible(pid).start,
        SHARED,
        "premise: the shared line is the first visible line"
    );
    let at = starts[SHARED];
    assert_eq!(
        p.conceals(bid)
            .into_iter()
            .filter(|c| c.start == at)
            .collect::<Vec<_>>(),
        [edge(at, "└───┘").revealing(starts[32], at)]
    );
}

#[test]
fn emphasis_crossing_the_window_edge_hides_both_delimiters() {
    const EDGE: usize = 23;
    let mut p = preview();
    // `x` lines, a blank line before line EDGE, `**a` on line EDGE and `b**`
    // on the next, then a blank line and `x` lines to line 40.
    let text: String = (0..41)
        .map(|line| match line {
            l if l == EDGE - 1 || l == EDGE + 2 => "\n",
            l if l == EDGE => "**a\n",
            l if l == EDGE + 1 => "b**\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let bid = p.open("a.md", &text);
    let pid = p.ed.view.focused_pane();
    p.render();
    let shown = p.visible(pid);
    assert!(
        shown.contains(&EDGE) && !shown.contains(&(EDGE + 1)),
        "premise: {shown:?} ends at the line holding the opening `**`"
    );
    let (open, close) = (starts[EDGE], starts[EDGE + 1]);
    let reveal = (open, starts[EDGE + 2]);
    assert_eq!(
        p.conceals(bid),
        [
            hidden(open, open + 1).revealing(reveal.0, reveal.1),
            hidden(open + 1, open + 2).revealing(reveal.0, reveal.1),
            hidden(close + 1, close + 2).revealing(reveal.0, reveal.1),
            hidden(close + 2, close + 3).revealing(reveal.0, reveal.1),
        ]
    );
}

#[test]
fn a_heading_above_the_window_is_set_for_its_pinned_section() {
    let mut p = preview();
    // `# a` on line 0, `# b` on line 10, `## c` on line 20 and its section
    // to the end: the window at the end is inside both `b` and `c`.
    let text: String = (0..100)
        .map(|line| match line {
            0 => "# a\n",
            10 => "# b\n",
            20 => "## c\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let bid = p.open("a.md", &text);
    let pid = p.ed.view.focused_pane();
    seek_to_line(&mut p.ed, 99);
    p.render();
    assert!(
        p.visible(pid).start > 20,
        "premise: the window starts below `## c`"
    );
    assert_eq!(p.conceal_starts(bid), [starts[10], starts[20]]);
}

/// Two side-by-side panes on `text`'s buffer: the first at the top, the
/// second at `line`. Returns the buffer and the two panes.
fn two_panes(p: &mut Preview, text: &str, line: usize) -> (BufferId, PaneId, PaneId) {
    let bid = p.open("a.md", text);
    let first = p.ed.view.focused_pane();
    p.ed.execute_typed("vsplit", None).unwrap();
    let second = p.ed.view.focused_pane();
    seek_to_line(&mut p.ed, line);
    (bid, first, second)
}

/// The lines a render covers for a pane showing `shown`: those lines and the
/// one above them.
fn covered(shown: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    shown.start.saturating_sub(1)..shown.end
}

#[test]
fn two_panes_on_one_buffer_render_each_viewport_and_not_the_lines_between() {
    let mut p = preview();
    let text = sectioned(100);
    let (bid, first, second) = two_panes(&mut p, &text, 80);
    p.render();
    let (first, second) = (p.visible(first), p.visible(second));
    assert!(
        first.end <= 30 && second.start >= 40 && second.end <= 90,
        "premise: {first:?} and {second:?} leave the section at 30 between them and the one at 90 after"
    );
    let mut expected = sections_meeting(&text, covered(&first));
    expected.extend(sections_meeting(&text, covered(&second)));
    expected.sort_unstable();
    expected.dedup();
    assert_eq!(p.conceal_starts(bid), expected);
}

#[test]
fn a_quoted_code_block_spanning_two_panes_is_set_once() {
    let mut p = preview();
    // `# a` on line 0, `x` lines, a code block inside a quote on lines
    // 10..=70, then `x` lines to line 99.
    let text: String = (0..100)
        .map(|line| match line {
            0 => "# a\n",
            10 | 70 => "> ```\n",
            11..70 => "> c\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let (bid, first, second) = two_panes(&mut p, &text, 75);
    p.render();
    let (first, second) = (p.visible(first), p.visible(second));
    assert!(
        first.contains(&11)
            && !first.contains(&70)
            && second.contains(&70)
            && first.end < second.start,
        "premise: {first:?} shows the block's start and {second:?} its end, apart"
    );
    let block = (starts[10], starts[71]);
    let mut expected = vec![replaced(0, 1, " 1 ", Some("markup.heading.1.marker"))];
    for (line, &at) in starts.iter().enumerate().take(71).skip(10) {
        expected.push(replaced(at, at + 1, "┃", Some("markup.quote")));
        if line == 10 || line == 70 {
            expected.push(hidden(at + 2, at + 5).revealing(block.0, block.1));
        }
    }
    assert_eq!(p.conceals(bid), expected);
    let mut expected_bands = vec![band(0, "markup.heading.1.line")];
    expected_bands.extend(bands(10..71, "markup.raw.block.line"));
    assert_eq!(p.bands(bid), expected_bands);
    assert_eq!(
        p.highlights(bid),
        [highlight(2, 3, "markup.heading.1.text")]
    );
}

#[test]
fn a_table_spanning_two_panes_keeps_its_top_border_and_sets_each_row_once() {
    let mut p = preview();
    // `x` lines, a blank line 9, the table `| a |` on lines 10..70, a blank
    // line 70, `x` lines to line 99.
    let text: String = (0..100)
        .map(|line| match line {
            9 | 70 => "\n",
            10 => "| a |\n",
            11 => "|---|\n",
            12..70 => "| r |\n",
            _ => "x\n",
        })
        .collect();
    let starts = line_starts(&text);
    let (bid, first, second) = two_panes(&mut p, &text, 75);
    p.render();
    let (first, second) = (p.visible(first), p.visible(second));
    assert!(
        first.contains(&8)
            && !first.contains(&70)
            && second.contains(&70)
            && first.end < second.start,
        "premise: {first:?} shows the line above the top border and {second:?} the bottom border, apart"
    );
    let mut expected = vec![edge(starts[9], "┌───┐")];
    for (line, &row) in starts.iter().enumerate().take(70).skip(10) {
        if line == 11 {
            expected.push(bar(row, row + 5, "├───┤"));
        } else {
            expected.extend([bar(row, row + 2, "│ "), bar(row + 3, row + 5, " │")]);
        }
    }
    expected.push(edge(starts[70], "└───┘"));
    assert_eq!(p.conceals(bid), revealing(expected, starts[10], starts[70]));
}

#[test]
fn render_is_skipped_while_the_tree_is_stale() {
    let mut p = preview();
    let text = sectioned(100);
    let bid = p.open("a.md", &text);
    p.render();
    let rendered = p.conceals(bid).len();

    // The scroll arms the viewport debounce; the edit after it leaves the
    // tree behind the text when the debounce runs out.
    p.ed.feed_key(key_ctrl('d'));
    p.frame();
    for k in [key('O'), key('#'), key(' '), key('n'), key_esc()] {
        p.ed.feed_key(k);
    }
    let heading =
        p.ed.doc()
            .text()
            .to_string()
            .find("# n\n")
            .expect("the typed heading");
    p.wait();
    assert_eq!(
        p.conceals(bid).len(),
        rendered,
        "the viewport hook found the tree stale and set nothing"
    );
    assert!(!p.conceal_starts(bid).contains(&heading));

    p.render();
    assert!(p.conceal_starts(bid).contains(&heading));
}

#[test]
fn buffer_shown_by_no_pane_renders_nothing() {
    let mut p = preview();
    let hidden_bid = p.open("a.md", "# a\n");
    let shown = p.open("b.md", "# b\n");
    p.render();
    assert!(
        has_layers(&p.ed, hidden_bid),
        "premise: the hidden buffer was parsed"
    );
    assert_eq!(
        p.conceal_starts(shown),
        [0],
        "premise: the shown buffer renders"
    );
    assert!(p.is_blank(hidden_bid));
}

// ── Lifecycle ─────────────────────────────────────────────────────────

/// A heading (conceal, band, highlight) and a fence (conceals, band).
const ELEMENTS: &str = "# a\n\n```\nc\n```\n";

#[test]
fn toggle_off_clears_every_source_and_on_renders_again() {
    let mut p = preview();
    let bid = p.open("a.md", ELEMENTS);
    p.render();
    let (conceals, bands, highlights) = (p.conceals(bid), p.bands(bid), p.highlights(bid));
    assert!(!p.is_blank(bid), "premise: the buffer renders");

    p.ed.execute_typed("toggle-markdown-preview", None).unwrap();
    p.ed.settle();
    assert!(p.is_blank(bid));

    call(&mut p.ed, "markdown-preview/toggle");
    p.ed.settle();
    assert_eq!(p.conceals(bid), conceals);
    assert_eq!(p.bands(bid), bands);
    assert_eq!(p.highlights(bid), highlights);
}

#[test]
fn toggle_in_a_buffer_outside_the_plugins_languages_is_refused() {
    let mut p = preview();
    let bid = p.open("a.txt", ELEMENTS);

    let refused = p.ed.execute_typed("toggle-markdown-preview", None);

    assert!(
        refused.is_err_and(|e| e.message().contains("not a Markdown buffer")),
        "the toggle names the buffer it refuses"
    );
    let markdown = p.ed.state.config.languages.intern("markdown");
    p.ed.set_buffer_language_explicit(bid, Some(markdown), ConfigOrigin::User)
        .expect_open();
    p.render_logging();
    assert!(!p.is_blank(bid), "the preview starts enabled");
}

#[test]
fn buffer_close_clears_the_sources() {
    let mut p = preview();
    let bid = p.open("a.md", ELEMENTS);
    p.render();
    assert!(!p.is_blank(bid), "premise: the buffer renders");

    // An edit whose syntax hook has armed the render debounce, then the
    // close before it runs out.
    for k in [key('o'), key('x'), key_esc()] {
        p.ed.feed_key(k);
    }
    drain_until(&mut p.ed, |ed| {
        ed.state.buffers.get(bid).current_layers().is_some()
    });
    p.ed.settle();
    p.ed.execute_typed("bd!", None).unwrap();
    p.wait();
    p.wait();

    assert!(p.is_blank(bid));
    assert_eq!(error_log(&p.ed), Vec::<String>::new());
}

#[test]
fn language_change_away_from_markdown_clears_the_sources() {
    let mut p = preview();
    let bid = p.open("a.md", ELEMENTS);
    p.render();
    assert!(!p.is_blank(bid), "premise: the buffer renders");

    p.ed.set_buffer_language(bid, None).expect_open();
    p.ed.settle();
    assert!(p.is_blank(bid));
}

#[test]
fn missing_inline_grammar_draws_the_block_elements_alone() {
    let mut p = preview_with(None, false);
    // "# a *b*\n": the marker 0..1; the emphasis delimiters
    // at 4 and 6 stay raw.
    let bid = p.open("a.md", "# a *b*\n");
    p.render();
    assert_eq!(
        p.conceals(bid),
        [replaced(0, 1, " 1 ", Some("markup.heading.1.marker"))]
    );
}

#[test]
fn missing_grammar_warns_once_naming_the_install_command() {
    let mut p = preview_with(None, false);
    p.open("a.md", "# a\n");
    p.render();
    p.open("b.md", "# b\n");
    p.render();
    let warnings: Vec<String> =
        p.ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == hume_core::Severity::Warning)
            .map(|e| e.text.to_string())
            .collect();
    assert_eq!(warnings.len(), 1, "got {warnings:?}");
    assert!(
        warnings[0].contains(":plum-install-grammar markdown.inline"),
        "got {warnings:?}"
    );
}

#[test]
fn enabled_false_renders_nothing_until_toggled() {
    let mut p = preview_with(Some("(hash 'enabled #f)"), true);
    let bid = p.open("a.md", ELEMENTS);
    p.render();
    assert!(p.is_blank(bid));

    p.ed.execute_typed("toggle-markdown-preview", None).unwrap();
    p.ed.settle();
    assert!(!p.conceals(bid).is_empty());
}

// ── Frames ────────────────────────────────────────────────────────────

/// One of each element, ending in a plain line the cursor sits on.
const SAMPLE: &str = "# Title

Some *emphasis*, **bold**, ~~strike~~ and `code`.
A [link](http://x) and &amp; entity, \\* escaped.

- bullet
- [ ] todo
- [x] done

> quote
lazy line

---

| Name | Qty |
|:-----|----:|
| apple | 3 |
| **kiwi** | 12 |

```rust
fn main() {}
```

end
";

/// The last line of `text`.
fn last_line(text: &str) -> usize {
    text.lines().count() - 1
}

/// The stored conceal texts.
fn conceal_texts(p: &Preview, bid: BufferId) -> Vec<String> {
    p.conceals(bid).into_iter().filter_map(|c| c.text).collect()
}

#[test]
fn full_sample() {
    let mut p = preview().sized(60, 32);
    let bid = p.open("a.md", SAMPLE);
    seek_to_line(&mut p.ed, last_line(SAMPLE));
    p.render();
    // `apple` and `Qty` set the widths: 5 and 3.
    assert!(conceal_texts(&p, bid).contains(&"├───────┼─────┤".to_owned()));
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn cursor_inside_a_table_reveals_the_whole_table() {
    let mut p = preview().sized(40, 12);
    // "Intro\n" 0..6, "\n" 6, the table on lines 2..=5, "\n" 47, "Outro\n".
    let text = "Intro\n\n| a | b |\n|---|---|\n| c | d |\n| e | f |\n\nOutro\n";
    let bid = p.open("a.md", text);
    seek_to_line(&mut p.ed, 5);
    p.render();
    assert!(
        p.conceals(bid)
            .contains(&edge(6, "┌───┬───┐").revealing(7, 47)),
        "the table's conceals are set, revealing over the table"
    );
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn wide_table_under_soft_wrap() {
    let mut p = preview().sized(40, 16);
    let text = "\n| a long first cell of text | a second cell that is long |\n|---|---|\n| x | y |\n\nend\n";
    let bid = p.open("a.md", text);
    pin_wrap(&mut p.ed, WrapMode::Soft { width: 0 });
    seek_to_line(&mut p.ed, last_line(text));
    p.render();
    // The columns are 25 and 26 wide.
    let rule = format!("├{}┼{}┤", "─".repeat(27), "─".repeat(28));
    assert!(conceal_texts(&p, bid).contains(&rule));
    insta::assert_snapshot!(p.snapshot());
}

/// A code block between two lines of text.
const CODE: &str = "Text\n\n```rust\nfn main() {\n}\n```\n\nend\n";

#[test]
fn code_block_with_the_cursor_outside() {
    let mut p = preview().sized(30, 12);
    p.open("a.md", CODE);
    seek_to_line(&mut p.ed, last_line(CODE));
    p.render();
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn code_block_with_the_cursor_inside() {
    let mut p = preview().sized(30, 12);
    p.open("a.md", CODE);
    seek_to_line(&mut p.ed, 3);
    p.render();
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn unicode_table_cells() {
    let mut p = preview().sized(40, 12);
    let text = "\n| 漢字 | e\u{301} |\n|---|---|\n| a | bb |\n| 🎉 | ä |\n\nend\n";
    let bid = p.open("a.md", text);
    seek_to_line(&mut p.ed, last_line(text));
    p.render();
    // `漢字` and the emoji's columns: 4 and 2 wide.
    assert!(conceal_texts(&p, bid).contains(&"├──────┼────┤".to_owned()));
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn closed_section_fold_and_closed_fenced_block_fold() {
    let mut p = preview().sized(30, 10);
    // `# One`'s section on lines 0..=3, the fence on lines 5..=8 inside
    // `# Two`'s, `end` on line 10.
    let text = "# One\na\nb\n\n# Two\n```sh\nx\ny\n```\n\nend\n";
    p.open("a.md", text);
    p.render();
    seek_to_line(&mut p.ed, 0);
    keys(&mut p.ed, "zc");
    seek_to_line(&mut p.ed, 5);
    keys(&mut p.ed, "zc");
    seek_to_line(&mut p.ed, 10);
    p.render();
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn heading_as_a_sticky_header_row() {
    let mut p = preview().sized(30, 10);
    let text = format!("# Head\n{}", "x\n".repeat(40));
    p.open("a.md", &text);
    seek_to_line(&mut p.ed, 30);
    p.render();
    insta::assert_snapshot!(p.snapshot());
}

/// A wrapping heading with an inlay hint, a table with EOL text on its last
/// row, a code block with a virtual line above it, and a closed section,
/// the cursor on `cursor_line` (`end` is line 9).
fn decorated_sample(cursor_line: usize) -> Preview {
    let mut p = preview().sized(40, 18);
    let text = "# A heading long enough to wrap past the edge\n\n| a | b |\n|---|---|\n| c | d |\n\n```sh\necho hi\n```\nend\n\n# Folded\nhidden\n";
    let bid = p.open("a.md", text);
    pin_wrap(&mut p.ed, WrapMode::Soft { width: 0 });
    p.render();
    seek_to_line(&mut p.ed, 11);
    keys(&mut p.ed, "zc");
    let decorations = &mut p.ed.state.config.decorations;
    decorations.set_inlay_hints(
        "test".to_owned(),
        bid,
        vec![InlayHintEntry {
            pos: co(10),
            text: ": hint".into(),
            side: hume_core::Side::After,
        }],
    );
    let warning = p.ed.view.registry.intern("diagnostic.warning");
    let decorations = &mut p.ed.state.config.decorations;
    decorations.set_eol_text(
        "test".to_owned(),
        bid,
        vec![EolTextEntry {
            line: ContentLine::new(4),
            text: "eol".into(),
            scope: warning,
            hide_on_insert_line: false,
        }],
    );
    decorations.set_virtual_lines(
        "test".to_owned(),
        bid,
        vec![VirtualLineEntry {
            line: ContentLine::new(6),
            content: hume_rope::spanned::SpannedText::plain("virtual".to_owned()),
            side: hume_core::Side::Before,
            scope: warning,
        }],
    );
    seek_to_line(&mut p.ed, cursor_line);
    p.render();
    assert!(
        conceal_texts(&p, bid).contains(&"├───┼───┤".to_owned()),
        "the table renders"
    );
    p
}

#[test]
fn conceals_beside_other_decorations_under_soft_wrap() {
    let mut p = decorated_sample(9);
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn conceals_beside_other_decorations_with_the_cursor_on_the_heading() {
    let mut p = decorated_sample(0);
    insta::assert_snapshot!(p.snapshot());
}

// ── Popups ───────────────────────────────────────────────────────────────

/// A popup's Markdown: a heading, emphasis, a list, a code block and a
/// table.
const POPUP: &str = "# Title\n\nSome *emphasis* and `code`.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| c | d |";

impl Preview {
    /// Shows `text` as a markdown popup docked at the bottom of the screen,
    /// from a typed command.
    fn show_popup(&mut self, text: &str) {
        run(
            &mut self.ed,
            self.tmp.path(),
            &format!(
                r#"(define-typed-command! "go" "" (lambda (pane)
                     (show-popup! pane {text:?} #:lang "markdown" #:anchor 'bottom)))"#
            ),
        );
        type_cmd(&mut self.ed, ":go");
    }

    /// The docked popup's rows.
    fn popup_rows(&self) -> Vec<String> {
        self.ed
            .state
            .views
            .popup_band
            .read()
            .as_ref()
            .map(|rows| rows.lines.texts())
            .expect("the popup shows")
    }
}

#[test]
fn a_markdown_popup_draws_without_its_syntax() {
    let mut p = preview().sized(40, 50);
    p.show_popup(POPUP);
    p.frame();
    assert_eq!(error_log(&p.ed), Vec::<String>::new());
    insta::assert_snapshot!(p.snapshot());
}

#[test]
fn a_popup_activates_the_plugin_with_no_markdown_buffer_open() {
    let mut p = preview();
    assert!(
        p.ed.state
            .config
            .registry
            .lazy_owner("format-text/markdown")
            .is_some(),
        "no markdown buffer has activated the plugin"
    );
    p.show_popup("# a");
    p.frame();
    assert_eq!(p.popup_rows(), [" 1  a"]);
    assert_eq!(error_log(&p.ed), Vec::<String>::new());
}

#[test]
fn the_completion_documentation_box_draws_markdown_without_its_syntax() {
    let mut p = preview();
    run(
        &mut p.ed,
        p.tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "foo" "documentation" (hash "kind" "markdown" "value" "**bold** text")))"#,
            "",
        ),
    );
    p.ed.feed_key(key('i'));
    p.ed.feed_key(key_ctrl(' '));
    p.ed.settle();
    p.frame();
    let rows =
        p.ed.state
            .views
            .completion_doc
            .read()
            .as_ref()
            .map(|state| state.rows.lines.texts())
            .expect("the documentation box shows");
    assert_eq!(rows, ["bold text"]);
    assert_eq!(error_log(&p.ed), Vec::<String>::new());
}
