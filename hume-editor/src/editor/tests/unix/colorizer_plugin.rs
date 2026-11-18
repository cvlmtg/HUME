// core:colorizer: end-to-end plugin tests.
//
// Loads the real plugin from the repo's `runtime/` tree onto a full editor,
// opens files as startup would, and asserts the extra-highlight spans the
// plugin sets and the styles their scopes resolve to in the live theme.
// Expected offsets are counted by hand from each fixture text.

use super::*;

use hume_engine::types::{ResolvedStyle, Scope};
use hume_grid::{Position, Rect, Rgb};
use hume_scripting::ScriptingHost;
use test_fixtures::require_grammars;

const SOURCE: &str = "colorizer";

const STYLES_MARKER: &str = "colorizer-test: set-scope-styles!";
const SPANS_MARKER: &str = "colorizer-test: set-extra-highlights!";

/// The editor's own two setters the plugin calls, kept under other names.
/// Evaluated on its own: a source that redefines a name resolves every
/// reference to it in that source to the new definition.
const SAVED_SETTERS: &str = r#"
(define colorizer-test/set-scope-styles! set-scope-styles!)
(define colorizer-test/set-extra-highlights! set-extra-highlights!)
"#;

/// Defined before the plugin loads: each setter logs its marker at trace
/// level, then calls the saved one, so a test counts the calls in the
/// message log.
const COUNTING_SETTERS: &str = r#"
(define (set-scope-styles! styles)
  (log! 'trace "colorizer-test: set-scope-styles!")
  (colorizer-test/set-scope-styles! styles))
(define (set-extra-highlights! pane source spans)
  (log! 'trace "colorizer-test: set-extra-highlights!")
  (colorizer-test/set-extra-highlights! pane source spans))
"#;

const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);
const BLACK: Rgb = Rgb(0, 0, 0);

/// An editor with the plugin loaded, `css` and `rust` named by their file
/// extensions, and the clock held so each debounce runs out only when a test
/// says.
struct Colorizer {
    ed: Editor,
    tmp: tempfile::TempDir,
    _dirs: RealRuntimeDirs,
}

/// `config` is the Steel source of the plugin's `#:config`.
fn colorizer_with(config: Option<&str>) -> Colorizer {
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
    for (language, extension) in [("css", "css"), ("rust", "rs")] {
        ed.state
            .config
            .languages
            .register_identity(language, &[extension], &[], &[])
            .unwrap();
    }
    let load = hume_scripting::testing::eager_load_scm("core:colorizer", config);
    let mut host = ScriptingHost::for_testing(&ed.state.dirs);
    eval_with_real_host(&mut ed, &mut host, SAVED_SETTERS, tmp.path());
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!("{COUNTING_SETTERS}\n(load-plugin! \"core:stdlib\")\n{load}"),
        tmp.path(),
    );
    ed.scripting = Some(host);
    crate::editor::settings::ops::apply_pending_theme(&mut ed.state, &mut ed.view);
    ed.state.clock.hold();
    Colorizer {
        ed,
        tmp,
        _dirs: dirs,
    }
}

fn colorizer() -> Colorizer {
    colorizer_with(None)
}

impl Colorizer {
    /// Opens `text` as `<tmp>/<name>` in the focused pane.
    fn open(&mut self, name: &str, text: &str) -> BufferId {
        open_in(&mut self.ed, self.tmp.path(), name, text)
    }

    /// Draws a frame, which arms the viewport hook's debounce, then lets it
    /// run out. No hook may have raised on the way.
    fn render(&mut self) {
        draw(&mut self.ed);
        self.ed.state.clock.advance(Duration::from_millis(200));
        self.ed.settle();
        assert_eq!(error_log(&self.ed), Vec::<String>::new());
    }

    /// Moves the held clock past the plugin's 50 ms edit debounce and
    /// settles.
    fn wait(&mut self) {
        self.ed.state.clock.advance(Duration::from_millis(60));
        self.ed.settle();
    }

    /// Presses `keys` one at a time, settling after each, with the clock
    /// held.
    fn press(&mut self, keys: impl IntoIterator<Item = KeyEvent>) {
        for k in keys {
            self.ed.feed_key(k);
            self.ed.settle();
        }
    }

    /// `(start, end, scope)` of each span the plugin stored for `bid`,
    /// sorted.
    fn spans(&self, bid: BufferId) -> Vec<(usize, usize, String)> {
        extra_highlight_triples(&self.ed, SOURCE, bid)
    }

    /// How many times the plugin called the setter `marker` names.
    fn calls(&self, marker: &str) -> usize {
        self.ed
            .state
            .message_log
            .entries()
            .filter(|e| e.text == marker)
            .count()
    }

    fn style(&self, scope: &'static str) -> ResolvedStyle {
        self.ed.view.theme.resolve_by_name(Scope(scope))
    }
}

/// Draws a frame at the headless terminal's size.
fn draw(ed: &mut Editor) {
    let Rect { width, height, .. } = crate::editor::run::HEADLESS_TERMINAL;
    frame(ed, width, height);
}

fn span(start: usize, end: usize, scope: &str) -> (usize, usize, String) {
    (start, end, scope.to_owned())
}

fn keys_for(text: &str) -> Vec<KeyEvent> {
    text.chars().map(key).collect()
}

#[test]
fn hex_forms_paint_without_alpha() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "#fff #ffff #123456 #12345678\n");
    c.render();

    assert_eq!(
        c.spans(bid),
        [
            span(0, 4, "colorizer.bg.ffffff"),
            span(5, 10, "colorizer.bg.ffffff"),
            span(11, 18, "colorizer.bg.123456"),
            span(19, 28, "colorizer.bg.123456"),
        ]
    );
    assert_eq!(c.style("colorizer.bg.ffffff").bg, Some(WHITE));
    assert_eq!(
        c.style("colorizer.bg.123456").bg,
        Some(Rgb(0x12, 0x34, 0x56))
    );
}

/// One literal per line: the line starts are 0, 7, 16, 22, 29, 41, 46, 52,
/// 57, 62, 70, 76 and 83.
const BOUNDARIES: &str = "&#123;\n#abcdefg\nx#fff\n(#fff)\ncolor:#fff;\n#123\n#1234\n#fff\n#12a\n\
                          #123456\n\u{e9}#fff\n\u{ab}#fff\u{bb}\n#fff\u{e9}\n";

#[test]
fn hex_boundaries() {
    let mut c = colorizer();
    let rust = c.open("a.rs", BOUNDARIES);
    c.render();
    let css = c.open("b.css", BOUNDARIES);
    c.render();

    let white = "colorizer.bg.ffffff";
    let always = [
        span(23, 27, white),
        span(35, 39, white),
        span(52, 56, white),
        span(57, 61, "colorizer.bg.1122aa"),
        span(62, 69, "colorizer.bg.123456"),
        span(71, 75, white),
        span(77, 81, white),
        span(83, 87, white),
    ];
    assert_eq!(c.spans(rust), always);
    let mut in_css = always.to_vec();
    in_css.extend([
        span(41, 45, "colorizer.bg.112233"),
        span(46, 51, "colorizer.bg.112233"),
    ]);
    in_css.sort();
    assert_eq!(c.spans(css), in_css);
}

/// The start of each line: 0, 12, 16, 28, 37, 49 and 62.
const NAMES: &str =
    "color: red;\nRED\n.red-button\n.btn-red\ntransparent\ncurrentcolor\n\u{e9}red\n";

#[test]
fn names_in_css_only() {
    let mut c = colorizer();
    let css = c.open("a.css", NAMES);
    c.render();
    let rust = c.open("b.rs", NAMES);
    c.render();

    let red = "colorizer.bg.ff0000";
    assert_eq!(
        c.spans(css),
        [span(7, 10, red), span(12, 15, red), span(63, 66, red)]
    );
    assert_eq!(c.spans(rust), []);
    assert_eq!(c.style(red).bg, Some(Rgb(0xff, 0, 0)));
}

#[test]
fn background_mode_contrast_and_foreground_mode() {
    let mut c = colorizer();
    c.open("a.rs", "#ffff00 #000080\n");
    c.render();
    let yellow = c.style("colorizer.bg.ffff00");
    assert_eq!(
        (yellow.bg, yellow.fg),
        (Some(Rgb(0xff, 0xff, 0)), Some(BLACK))
    );
    let navy = c.style("colorizer.bg.000080");
    assert_eq!((navy.bg, navy.fg), (Some(Rgb(0, 0, 0x80)), Some(WHITE)));

    let mut c = colorizer_with(Some("(hash 'mode 'foreground)"));
    let bid = c.open("a.rs", "#ffff00\n");
    c.render();
    assert_eq!(c.spans(bid), [span(0, 7, "colorizer.fg.ffff00")]);
    let yellow = c.style("colorizer.fg.ffff00");
    assert_eq!((yellow.fg, yellow.bg), (Some(Rgb(0xff, 0xff, 0)), None));
}

#[test]
fn one_scope_per_color_and_no_restyle_without_new_colors() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "#ff0000 #0000ff #ff0000\n");
    c.render();
    let (red, blue) = ("colorizer.bg.ff0000", "colorizer.bg.0000ff");
    assert_eq!(
        c.spans(bid),
        [span(0, 7, red), span(8, 15, blue), span(16, 23, red)]
    );
    assert_eq!(c.calls(STYLES_MARKER), 1, "two new colors, one call");

    let renders = c.calls(SPANS_MARKER);
    c.press([key('i'), key(' '), key_esc()]);
    c.wait();
    assert_eq!(c.calls(SPANS_MARKER), renders + 1, "the edit renders");
    assert_eq!(
        c.spans(bid),
        [span(1, 8, red), span(9, 16, blue), span(17, 24, red)]
    );
    assert_eq!(c.calls(STYLES_MARKER), 1);

    c.press([key('i')]);
    c.press(keys_for("#00ff00 #ffff00 "));
    c.press([key_esc()]);
    c.wait();
    assert_eq!(
        c.spans(bid)[..2],
        [
            span(0, 7, "colorizer.bg.00ff00"),
            span(8, 15, "colorizer.bg.ffff00")
        ]
    );
    assert_eq!(c.calls(STYLES_MARKER), 2, "two new colors, one call");
    assert_eq!(c.style("colorizer.bg.00ff00").bg, Some(Rgb(0, 0xff, 0)));
    assert_eq!(c.style("colorizer.bg.ffff00").bg, Some(Rgb(0xff, 0xff, 0)));
}

#[test]
fn two_panes_show_both_ranges() {
    let mut c = colorizer();
    let bid = c.open("a.rs", &"#ff0000\n".repeat(200));
    let first = c.ed.view.focused_pane();
    let second = vsplit_focused(&mut c.ed);
    seek_to_line(&mut c.ed, 150);
    c.render();

    let visible = |pid: PaneId| {
        let (_, range) = c.ed.last_visible_range[&pid];
        range.start.index()..range.end.index()
    };
    let (top, bottom) = (visible(first), visible(second));
    assert!(
        top.end < bottom.start && bottom.end < 200,
        "premise: {top:?} and {bottom:?} leave lines between and after them"
    );
    let expected: Vec<_> = top
        .chain(bottom)
        .map(|line| span(8 * line, 8 * line + 7, "colorizer.bg.ff0000"))
        .collect();
    assert_eq!(c.spans(bid), expected);
}

#[test]
fn overlapping_panes_paint_each_line_once() {
    let mut c = colorizer();
    let bid = c.open("a.rs", &"#ff0000\n".repeat(200));
    let first = c.ed.view.focused_pane();
    let second = vsplit_focused(&mut c.ed);
    seek_to_line(&mut c.ed, 30);
    c.render();

    let visible = |pid: PaneId| {
        let (_, range) = c.ed.last_visible_range[&pid];
        range.start.index()..range.end.index()
    };
    let (top, bottom) = (visible(first), visible(second));
    assert!(
        top.start < bottom.start && bottom.start < top.end && top.end < bottom.end,
        "premise: {top:?} and {bottom:?} overlap"
    );
    let expected: Vec<_> = (top.start..bottom.end)
        .map(|line| span(8 * line, 8 * line + 7, "colorizer.bg.ff0000"))
        .collect();
    assert_eq!(c.spans(bid), expected);
}

#[test]
fn a_render_of_the_painted_text_and_view_sets_nothing() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "#ff0000\n");
    let first = c.ed.view.focused_pane();
    let second = vsplit_focused(&mut c.ed);
    c.render();
    let renders = c.calls(SPANS_MARKER);

    for pid in [first, second] {
        c.ed.queue_viewport_change(pid);
    }
    c.ed.settle();
    assert_eq!(c.calls(SPANS_MARKER), renders, "the same text and view");

    c.press([key('i'), key(' '), key_esc()]);
    c.wait();
    for pid in [first, second] {
        c.ed.queue_viewport_change(pid);
    }
    c.ed.settle();
    assert_eq!(c.calls(SPANS_MARKER), renders + 1, "one render of the edit");
    assert_eq!(c.spans(bid), [span(1, 8, "colorizer.bg.ff0000")]);
}

/// Lines of `#ff0000 #00ff00`, 16 chars with the newline. The view moves
/// down 5 lines, so its lines above 64 come from the first render's table
/// with both pieces; then up past the first view's top by 5, onto lines no
/// render walked yet.
#[test]
fn a_scroll_paints_lines_from_the_table() {
    let mut c = colorizer();
    let bid = c.open("a.rs", &"#ff0000 #00ff00\n".repeat(200));
    let pid = c.ed.view.focused_pane();
    let painted = |lines: std::ops::Range<usize>| -> Vec<_> {
        lines
            .flat_map(|line| {
                [
                    span(16 * line, 16 * line + 7, "colorizer.bg.ff0000"),
                    span(16 * line + 8, 16 * line + 15, "colorizer.bg.00ff00"),
                ]
            })
            .collect()
    };

    for (cursor, shown) in [(60, 40..64), (65, 45..69), (38, 35..59)] {
        seek_to_line(&mut c.ed, cursor);
        c.render();
        let (_, range) = c.ed.last_visible_range[&pid];
        assert_eq!(
            range.start.index()..range.end.index(),
            shown,
            "premise: the view after line {cursor}"
        );
        assert_eq!(c.spans(bid), painted(shown));
    }
}

#[test]
fn edits_repaint_after_the_debounce() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "a\n");
    c.render();
    assert_eq!(c.spans(bid), []);

    c.press([key('i')]);
    c.press(keys_for("#ff0000 "));
    c.press([key_esc()]);
    assert_eq!(c.spans(bid), [], "nothing paints before the debounce");
    c.wait();
    assert_eq!(c.spans(bid), [span(0, 7, "colorizer.bg.ff0000")]);

    c.press([key('u')]);
    c.wait();
    assert_eq!(c.spans(bid), []);

    let renders = c.calls(SPANS_MARKER);
    c.press([key('i')]);
    c.press(keys_for("bcdef"));
    c.press([key_esc()]);
    c.wait();
    assert_eq!(c.calls(SPANS_MARKER), renders + 1);
}

#[test]
fn toggle_clears_and_restores() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "#ff0000\n");
    c.render();
    let painted = [span(0, 7, "colorizer.bg.ff0000")];
    assert_eq!(c.spans(bid), painted);

    c.ed.execute_typed("toggle-colorizer", None).unwrap();
    c.ed.settle();
    assert_eq!(c.spans(bid), []);
    c.ed.execute_typed("toggle-colorizer", None).unwrap();
    c.ed.settle();
    assert_eq!(c.spans(bid), painted);

    run(
        &mut c.ed,
        c.tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane) (call! "colorizer/toggle" pane)))"#,
    );
    c.ed.execute_typed("go", None).unwrap();
    c.ed.settle();
    assert_eq!(c.spans(bid), []);
    assert_eq!(error_log(&c.ed), Vec::<String>::new());
}

/// Once the session holds as many scope names as it lets in, a color the
/// plugin has painted before still paints and a new one does not, with one
/// warning for the session.
#[test]
fn at_the_scope_name_limit_only_painted_colors_paint() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "#ff0000\n");
    c.render();
    c.ed.view.registry.fill_to_limit();
    let warnings = |c: &Colorizer| {
        c.ed.state
            .message_log
            .entries()
            .filter(|e| e.text.contains("as many scope names as it allows"))
            .count()
    };

    let red = "colorizer.bg.ff0000";
    for (typed, expected) in [
        ("#ff0000 #00ff00 ", vec![span(0, 7, red), span(16, 23, red)]),
        (
            "#0000ff #ff0000 ",
            vec![span(8, 15, red), span(16, 23, red), span(32, 39, red)],
        ),
    ] {
        c.press([key('i')]);
        c.press(keys_for(typed));
        c.press([key_esc()]);
        c.wait();
        assert_eq!(c.spans(bid), expected);
        assert_eq!(error_log(&c.ed), Vec::<String>::new());
        assert_eq!(warnings(&c), 1);
    }
}

#[test]
fn painting_can_start_off() {
    let mut c = colorizer_with(Some("(hash 'enabled #f)"));
    let bid = c.open("a.rs", "#ff0000\n");
    c.render();
    assert_eq!(c.spans(bid), []);

    c.ed.execute_typed("toggle-colorizer", None).unwrap();
    c.ed.settle();
    assert_eq!(c.spans(bid), [span(0, 7, "colorizer.bg.ff0000")]);
}

#[test]
fn named_languages_come_from_the_config() {
    let mut c = colorizer_with(Some("(hash 'named-languages '(\"rust\"))"));
    let rust = c.open("a.rs", "red\n");
    c.render();
    let css = c.open("b.css", "red\n");
    c.render();

    assert_eq!(c.spans(rust), [span(0, 3, "colorizer.bg.ff0000")]);
    assert_eq!(c.spans(css), []);
}

#[test]
fn a_language_change_renders_at_once() {
    let mut c = colorizer();
    let bid = c.open("a.txt", "red\n");
    c.render();
    assert_eq!(
        c.spans(bid),
        [],
        "premise: a buffer with no language paints no names"
    );

    run_set(&mut c.ed, "buffer language=css").unwrap();
    c.ed.settle();

    assert_eq!(c.spans(bid), [span(0, 3, "colorizer.bg.ff0000")]);
}

#[test]
fn a_long_line_is_skipped() {
    let mut c = colorizer_with(Some("(hash 'max-render-chars 20)"));
    let long = format!("x #00ff00 {}", "x".repeat(20));
    assert_eq!(long.chars().count(), 30, "premise: the first line is long");
    let bid = c.open("a.rs", &format!("{long}\n#ff0000\n"));
    c.render();

    assert_eq!(c.spans(bid), [span(31, 38, "colorizer.bg.ff0000")]);
}

/// A budget of 20: the first line claims 7, the second needs 16 of the 13
/// left and is skipped, the third claims 7 of them.
#[test]
fn the_render_budget_skips_and_continues() {
    let mut c = colorizer_with(Some("(hash 'max-render-chars 20)"));
    let skipped = "x #0000ff xxxxxx";
    assert_eq!(
        skipped.chars().count(),
        16,
        "premise: the second line needs 16"
    );
    let bid = c.open("a.rs", &format!("#ff0000\n{skipped}\n#00ff00\n"));
    c.render();

    assert_eq!(
        c.spans(bid),
        [
            span(0, 7, "colorizer.bg.ff0000"),
            span(25, 32, "colorizer.bg.00ff00")
        ]
    );
}

#[test]
fn styles_survive_a_theme_switch() {
    let mut c = colorizer();
    c.open("a.rs", "#ff0000\n");
    c.render();
    let red = c.style("colorizer.bg.ff0000");
    assert_eq!((red.bg, red.fg), (Some(Rgb(0xff, 0, 0)), Some(BLACK)));

    type_cmd(&mut c.ed, ":theme dracula");
    c.ed.settle();

    assert_eq!(c.ed.state.shown_theme, "dracula");
    assert_eq!(c.style("colorizer.bg.ff0000"), red);
}

#[test]
fn a_render_after_close_does_nothing() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "#ff0000\n");
    c.render();
    assert!(!c.spans(bid).is_empty(), "premise: the buffer paints");

    c.press([key('i'), key('x'), key_esc()]);
    c.ed.execute_typed("bd!", None).unwrap();
    c.wait();

    assert_eq!(error_log(&c.ed), Vec::<String>::new());
    assert_eq!(c.spans(bid), []);
}

/// Line 1 holds `"#ff0000"` from char 24 to 33 and line 2 `"#00ff00"` from
/// 47 to 56, each a Rust string literal.
const SOURCES: &str = "fn main() {\n    let a = \"#ff0000\";\n    let b = \"#00ff00\";\n}\n";

/// A source sorting after `colorizer` paints `"#ff` on line 1, starting one
/// char before the colorizer's span, and all of `#00ff00` on line 2,
/// starting with it.
const OTHER_SOURCES: &str = r##"(define-typed-command! "go" "" (lambda (pane)
  (set-scope-styles! (list (hash 'scope "test.early" 'fg "#010203" 'bg "#040506")
                           (hash 'scope "test.same" 'fg "#070809" 'bg "#0a0b0c")))
  (set-extra-highlights! pane "zz-early" (list (hash 'start-char 24 'end-char 28 'scope "test.early")))
  (set-extra-highlights! pane "zz-same" (list (hash 'start-char 48 'end-char 55 'scope "test.same")))))"##;

#[test]
fn spans_over_syntax_and_among_sources() {
    require_grammars(&["rust"]);
    let mut c = colorizer();
    attach_fixture_grammar(&mut c.ed, "rust", "tree_sitter_rust");
    let bid = c.open("a.rs", SOURCES);
    poll_until(
        &mut c.ed,
        |ed| {
            draw(ed);
            ed.settle();
        },
        |ed| ed.state.buffers.get(bid).current_layers().is_some(),
    );
    c.render();
    run(&mut c.ed, c.tmp.path(), OTHER_SOURCES);
    c.ed.execute_typed("go", None).unwrap();
    draw(&mut c.ed);

    let grid = c.ed.render_to_buf(crate::editor::run::HEADLESS_TERMINAL);
    let pid = c.ed.view.focused_pane();
    let size =
        c.ed.view
            .pane_hit(Position::new(0, 0))
            .expect("the pane starts at the origin")
            .size();
    let gutter = c.ed.state.pane_geometry(&c.ed.view, pid, size).gutter_width;
    let cell = |line: u16, column: u16| grid[(gutter + column, line)].style();
    let colors = |style: ResolvedStyle| (style.fg, style.bg);

    let string = c.style("string").fg;
    let red = colors(c.style("colorizer.bg.ff0000"));
    let green = colors(c.style("colorizer.bg.00ff00"));
    let early = colors(c.style("test.early"));
    assert_ne!(
        string, red.0,
        "premise: the literal's own text color differs"
    );

    // Line 1 starts at char 12: the quote at column 12, `#ff0000` at 13-19.
    assert_eq!(colors(cell(1, 12)), early, "the other source alone");
    for column in 13..16 {
        assert_eq!(colors(cell(1, column)), early, "the earlier start wins");
    }
    for column in 16..20 {
        assert_eq!(colors(cell(1, column)), red, "the colorizer over syntax");
    }
    assert_eq!(cell(1, 20).fg, string, "syntax alone");
    // Line 2 starts at char 35: `#00ff00` at columns 13-19.
    for column in 13..20 {
        assert_eq!(colors(cell(2, column)), green, "the same start: colorizer");
    }
    assert_eq!(cell(2, 20).fg, string, "syntax alone");
}

/// `lines`, each followed by `\n`, and the span of each line paired with a color:
/// the whole line, scoped `colorizer.bg.<color>`.
fn whole_lines(lines: &[(&str, Option<&str>)]) -> (String, Vec<(usize, usize, String)>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    for (line, color) in lines {
        let start = text.chars().count();
        text.push_str(line);
        if let Some(color) = color {
            spans.push(span(
                start,
                text.chars().count(),
                &format!("colorizer.bg.{color}"),
            ));
        }
        text.push('\n');
    }
    (text, spans)
}

/// Opens `lines` as one `.rs` buffer and asserts the spans `whole_lines`
/// gives for them.
fn assert_whole_lines(lines: &[(&str, Option<&str>)]) {
    let (text, expected) = whole_lines(lines);
    let mut c = colorizer();
    let bid = c.open("a.rs", &text);
    c.render();
    assert_eq!(c.spans(bid), expected);
}

#[test]
fn rgb_calls() {
    assert_whole_lines(&[
        ("rgb(255, 0, 0)", Some("ff0000")),
        ("rgba(255,0,0,0.5)", Some("ff0000")),
        ("rgb(255 0 0 / 50%)", Some("ff0000")),
        ("RGB(100%, 0%, 0%)", Some("ff0000")),
        ("rgb(100%, 50%, 0%)", Some("ff8000")),
        ("rgb(300 -5 0)", Some("ff0000")),
        ("rgb(255, 0%, 0)", None),
    ]);

    let mut c = colorizer();
    c.open("a.rs", "rgb(100%, 50%, 0%)\n");
    c.render();
    assert_eq!(c.style("colorizer.bg.ff8000").bg, Some(Rgb(0xff, 0x80, 0)));
}

#[test]
fn hsl_calls() {
    assert_whole_lines(&[
        ("hsl(120, 100%, 50%)", Some("00ff00")),
        ("hsl(120 100 50)", Some("00ff00")),
        ("hsl(0, 100%, 25%)", Some("800000")),
        ("hsl(240deg 100% 50%)", Some("0000ff")),
        ("hsl(0.6667turn 100% 50%)", Some("0000ff")),
        ("hsl(-120 100% 50%)", Some("0000ff")),
        ("hsl(4.18879rad 100% 50%)", Some("0000ff")),
        ("hsl(266.6667grad 100% 50%)", Some("0000ff")),
        ("hsla(120, 100%, 50%, 0.5)", Some("00ff00")),
    ]);
}

/// The three bytes a `colorizer.bg.rrggbb` scope names.
fn channels(scope: &str) -> [u8; 3] {
    let hex = scope
        .strip_prefix("colorizer.bg.")
        .expect("a background scope");
    [0, 2, 4].map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
}

#[test]
fn oklch_in_gamut() {
    let (text, expected) = whole_lines(&[
        ("oklch(1 0 0)", Some("ffffff")),
        ("oklch(0% 0 0)", Some("000000")),
        ("oklch(0.5 none 0)", Some("636363")),
        ("oklch(62.796% 0.25768 29.2339)", Some("ff0000")),
        ("oklch(86.644% 0.294827 142.4953)", Some("00ff00")),
        ("oklch(45.201% 0.313214 264.052)", Some("0000ff")),
        ("oklch(62.796% 64.42% 29.2339)", Some("ff0000")),
    ]);
    let mut c = colorizer();
    let bid = c.open("a.rs", &text);
    c.render();

    let spans = c.spans(bid);
    assert_eq!(spans.len(), expected.len());
    for (got, want) in spans.iter().zip(&expected) {
        assert_eq!((got.0, got.1), (want.0, want.1));
        let close = channels(&got.2)
            .iter()
            .zip(channels(&want.2))
            .all(|(g, w)| g.abs_diff(w) <= 1);
        assert!(close, "{} is within one per channel of {}", got.2, want.2);
    }
}

#[test]
fn oklch_gamut_mapped() {
    assert_whole_lines(&[
        ("oklch(63.7% 0.237 25.331)", Some("fb2c36")),
        ("oklch(62.3% 0.214 259.815)", Some("2b7fff")),
        ("oklch(72.3% 0.219 149.579)", Some("00c950")),
        ("oklch(70% 0.4 150)", Some("00c248")),
        ("oklch(90% 0.3 100)", Some("ffdf00")),
        ("oklch(120% 0.2 30)", Some("ffffff")),
    ]);
}

/// The start of each line: 0, 13, 36, 50, 57 and 63. The last four lines hold one
/// call: its name at char 41, its first argument's line ending at 49, the
/// next two lines' `0` at 54 and 61, its `)` at 65.
const SYNTAX: &str = "rgb(1e2 0 0)\nrgb(255 /* red */ 0 0)\n";

#[test]
fn css_syntax_details() {
    let mut c = colorizer();
    let bid = c.open("a.rs", SYNTAX);
    c.render();

    let red = "colorizer.bg.ff0000";
    assert_eq!(
        c.spans(bid),
        [span(0, 12, "colorizer.bg.640000"), span(13, 35, red)]
    );
}

#[test]
fn unpainted_calls() {
    assert_whole_lines(&[
        ("rgb(var(--r), 0, 0)", None),
        ("rgb(calc(1 + 1) 0 0)", None),
        ("rgb(from red r g b)", None),
        ("rgb (255 0 0)", None),
        ("myrgb(1, 2, 3)", None),
        ("rgb(1, 2)", None),
        ("rgb(255, 0 0)", None),
        ("hsl(none, 100%, 50%)", None),
        ("rgb(1e999 0 0)", None),
        ("rgb(255 , 0 , 0)", Some("ff0000")),
        ("rgb(255 0 0", None),
    ]);
}

#[test]
fn a_one_line_call_spans_name_through_paren() {
    let mut c = colorizer();
    let bid = c.open("a.rs", "color: rgb(0, 0, 255);\n");
    c.render();

    assert_eq!(c.spans(bid), [span(7, 21, "colorizer.bg.0000ff")]);
}
