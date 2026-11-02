// A picker's `#:render`: the picker asks it for the rows of the visible
// window it has not shown at the panel's current width and query, once per
// row, before the frame is drawn, passing the query the rows answer.
// Ranking, `.` and bindings stay on the items.
//
// The picker is full-modal and eats typed keys, so the log of render calls
// is read through a keymap command (`requests`), not a typed one.

use std::path::Path;

use hume_grid::Rect;

use super::*;
use hume_engine::pipeline::RenderContext;

const WIDTH: u16 = 40;
const HEIGHT: u16 = 16;

/// Every render call is appended to `requests` as `count:budget:query;`, and
/// every row it renders to `rendered` as `display,`. Rows read `<display>`.
/// `on-select` logs its argument, and `#:on-close` counts in `closes`.
const PRELUDE: &str = r##"
(define requests "")
(define rendered "")
(define closes 0)
(define tok #f)
(define (render rows budget query)
  (set! requests (string-append requests (number->string (length rows)) ":"
                                (number->string budget) ":" query ";"))
  (map (lambda (row)
         (set! rendered (string-append rendered (car row) ","))
         (string-append "<" (car row) ">"))
       rows))
(define (items from to)
  (map (lambda (i) (cons (string-append "item" (number->string i)) i)) (range from to)))
(define (on-close) (set! closes (+ closes 1)))
(define-command! "requests" "" (lambda (pane) (log! 'info requests)))
(define-command! "rendered" "" (lambda (pane) (log! 'info rendered)))
(define-command! "closes" "" (lambda (pane) (log! 'info (number->string closes))))
(define-command! "push-more" "" (lambda (pane) (picker-push! tok (items 500 503))))
(define-command! "replace-all" "" (lambda (pane) (picker-replace! tok (items 900 903))))
(define-command! "indent-width" "" (lambda (pane)
  (log! 'info (to-string (get-buffer-option pane "indent-width")))))
"##;

/// Defines `:go`, which opens a picker over `items` with `extra` keywords
/// (`#:render` among them), and runs it.
fn open(ed: &mut Editor, tmp: &Path, items: &str, extra: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"{PRELUDE}
(define-typed-command! "go" "" (lambda (pane)
  (set! tok (picker! pane {items}
                     (test-picker-keys (lambda (x) (log! 'info (to-string x))))
                     #:on-close on-close {extra}))))"#
        ),
    );
    type_cmd(ed, ":go");
}

/// The status line after running the keymap command `name`.
fn shown(ed: &mut Editor, name: &str) -> String {
    ed.execute_keymap_command(name, Count::UNTYPED, false);
    ed.settle();
    ed.state.status_msg.clone().unwrap()
}

/// `(count, budget, query)` per render call, in call order.
fn requests(ed: &mut Editor) -> Vec<(usize, usize, String)> {
    shown(ed, "requests")
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut fields = s.splitn(3, ':');
            let mut field = || fields.next().unwrap();
            (
                field().parse().unwrap(),
                field().parse().unwrap(),
                field().to_owned(),
            )
        })
        .collect()
}

/// Every display the render proc was given, in call order.
fn rendered(ed: &mut Editor) -> Vec<String> {
    shown(ed, "rendered")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The picker view's rows: `None` for a row not rendered yet.
fn view_rows(ed: &Editor) -> Vec<Option<String>> {
    let guard = ed.state.views.picker.read();
    guard
        .as_ref()
        .expect("picker must be showing")
        .rows
        .iter()
        .map(|row| row.as_ref().map(|row| row.text().to_owned()))
        .collect()
}

/// The panel's list rows and the cells one row gets, as this frame laid it
/// out.
fn panel(ed: &Editor) -> (usize, usize) {
    let guard = ed.state.views.picker.read();
    let state = guard.as_ref().expect("picker must be showing");
    (state.list_rows, state.rect.inset(1, 1).width as usize)
}

fn row(i: usize) -> Option<String> {
    Some(format!("<item{i}>"))
}

#[test]
fn only_visible_rows_are_rendered() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);

    let (list_rows, budget) = panel(&ed);
    assert!(list_rows < 50, "setup: the panel cannot show every row");
    assert_eq!(requests(&mut ed), vec![(list_rows, budget, String::new())]);
    assert_eq!(view_rows(&ed), (0..list_rows).map(row).collect::<Vec<_>>());
}

#[test]
fn a_second_frame_renders_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_eq!(requests(&mut ed).len(), 1);
}

#[test]
fn scrolling_renders_only_newly_visible_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);
    let (list_rows, _) = panel(&ed);

    for _ in 0..list_rows + 3 {
        ed.feed_key(key_down());
        frame(&mut ed, WIDTH, HEIGHT);
    }

    let expected: Vec<String> = (0..list_rows + 4).map(|i| format!("item{i}")).collect();
    assert_eq!(rendered(&mut ed), expected);
    assert_eq!(
        view_rows(&ed),
        (4..list_rows + 4).map(row).collect::<Vec<_>>()
    );
}

#[test]
fn a_resize_renders_the_visible_rows_again_at_the_new_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);
    let (_, narrow) = panel(&ed);

    frame(&mut ed, WIDTH + 20, HEIGHT);

    let (list_rows, wide) = panel(&ed);
    assert_ne!(narrow, wide, "setup: the panel grew wider");
    assert_eq!(
        requests(&mut ed),
        vec![
            (list_rows, narrow, String::new()),
            (list_rows, wide, String::new())
        ]
    );
}

#[test]
fn a_taller_panel_renders_its_newly_visible_rows_in_the_same_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);
    let (short, budget) = panel(&ed);

    frame(&mut ed, WIDTH, HEIGHT * 2);

    let (tall, tall_budget) = panel(&ed);
    assert!(tall > short && tall < 50, "setup: the panel grew taller");
    assert_eq!(tall_budget, budget, "setup: the panel kept its width");
    assert_eq!(view_rows(&ed), (0..tall).map(row).collect::<Vec<_>>());
    assert_eq!(
        requests(&mut ed),
        vec![
            (short, budget, String::new()),
            (tall - short, budget, String::new())
        ]
    );
}

#[test]
fn typing_renders_the_visible_rows_again_with_the_new_query() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);
    for ch in "item1".chars() {
        ed.feed_key(key(ch));
        frame(&mut ed, WIDTH, HEIGHT);
    }

    let queries: Vec<String> = requests(&mut ed)
        .into_iter()
        .map(|(_, _, query)| query)
        .collect();
    assert_eq!(queries, ["", "i", "it", "ite", "item", "item1"]);
    assert!(
        view_rows(&ed).iter().all(Option::is_some),
        "every visible row is rendered"
    );
}

#[test]
fn pushed_and_replaced_rows_render_when_shown() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "'()", "#:render render");
    frame(&mut ed, WIDTH, HEIGHT);

    ed.execute_keymap_command("push-more", Count::UNTYPED, false);
    frame(&mut ed, WIDTH, HEIGHT);
    assert_eq!(view_rows(&ed), vec![row(500), row(501), row(502)]);

    ed.execute_keymap_command("replace-all", Count::UNTYPED, false);
    frame(&mut ed, WIDTH, HEIGHT);
    assert_eq!(view_rows(&ed), vec![row(900), row(901), row(902)]);
}

#[test]
fn render_gets_display_payload_pairs_and_the_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 2)",
        r#"#:render (lambda (rows budget query)
             (map (lambda (row)
                    (string-append (car row) "=" (number->string (cdr row))
                                   "@" (number->string budget)))
                  rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);

    let (_, budget) = panel(&ed);
    assert_eq!(
        view_rows(&ed),
        vec![
            Some(format!("item0=0@{budget}")),
            Some(format!("item1=1@{budget}"))
        ]
    );
}

#[test]
fn matching_ranking_and_bindings_use_the_original_item() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 50)",
        r#"#:render (lambda (rows budget query) (map (lambda (row) "zzz") rows))"#,
    );
    for ch in "item37".chars() {
        ed.feed_key(key(ch));
    }
    frame(&mut ed, WIDTH, HEIGHT);

    let session = ed.state.input.picker().expect("picker open");
    assert_eq!(session.window(1).collect::<Vec<_>>(), ["item37"]);
    assert_eq!(view_rows(&ed)[0].as_deref(), Some("zzz"));

    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "37");
}

#[test]
fn a_row_wider_than_the_budget_is_cut_at_the_truncate_end() {
    let long = r#"#:render (lambda (rows budget query)
                   (map (lambda (row) (string-append "START" (make-string 60 #\-) "END")) rows))"#;
    let first_row = |cut: &str| {
        let tmp = tempfile::tempdir().unwrap();
        let mut ed = editor_from("-[x]>abc\n");
        open(
            &mut ed,
            tmp.path(),
            "(items 0 1)",
            &format!("#:truncate '{cut} {long}"),
        );
        frame(&mut ed, WIDTH, HEIGHT);
        view_rows(&ed)[0].clone().unwrap()
    };

    insta::assert_snapshot!(first_row("head"), @"…--------------------------END");
    insta::assert_snapshot!(first_row("tail"), @"START------------------------…");
}

#[test]
fn a_plain_pickers_long_row_is_cut_at_the_truncate_end() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        r#"(list (cons (string-append "START" (make-string 60 #\-) "END") 'long))"#,
        "#:truncate 'tail",
    );
    frame(&mut ed, WIDTH, HEIGHT);

    insta::assert_snapshot!(view_rows(&ed)[0].clone().unwrap(), @"START------------------------…");
}

#[test]
fn render_rows_take_spans_and_a_selected_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 2)",
        r#"#:render (lambda (rows budget query)
             (map (lambda (row)
                    (if (= (cdr row) 0)
                        (hash 'text (car row) 'selected-scope "markup.bold"
                              'spans (list (hash 'start-char 0 'end-char 4 'scope "keyword"
                                                 'selected-scope "string")))
                        (car row)))
                  rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);

    assert_picker_styles(
        &mut ed,
        &[
            (
                Some("markup.bold"),
                &[(0, 4, "keyword", Some("string"))][..],
            ),
            (None, &[]),
        ],
    );
}

/// A failing render closes the picker and logs the failure, naming
/// `needle`, in the frame it fails in. The `#:on-close` it queues wakes the
/// loop at once, and the next settle fires it once.
fn assert_render_failure_closes(ed: &mut Editor, needle: &str) {
    assert!(ed.state.input.picker().is_none(), "the picker closed");
    assert!(
        error_log(ed)
            .iter()
            .any(|e| e.contains("picker render error") && e.contains(needle)),
        "errors: {:?}",
        error_log(ed)
    );
    assert_eq!(ed.wake_timeout(), Some(std::time::Duration::ZERO));
    ed.settle();
    assert_eq!(shown(ed, "closes"), "1");
}

fn failing_render(proc: &str, needle: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 30)",
        &format!("#:render {proc}"),
    );
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed, needle);
}

#[test]
fn a_raising_render_proc_closes_the_picker() {
    failing_render(r#"(lambda (rows budget query) (error "boom"))"#, "boom");
}

#[test]
fn a_render_proc_returning_too_few_rows_closes_the_picker() {
    failing_render(
        r#"(lambda (rows budget query) (list "only one"))"#,
        "expected a list of",
    );
}

#[test]
fn a_render_proc_returning_numbers_closes_the_picker() {
    failing_render(
        "(lambda (rows budget query) (map cdr rows))",
        "row 0: expected a string",
    );
}

/// The error names the failing row by its item's place in the list the
/// script gave, not by its place in the render call.
#[test]
fn a_render_error_for_invalid_spans_names_the_row() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 30)",
        r#"#:render (lambda (rows budget query)
             (map (lambda (row)
                    (if (equal? (car row) "item29")
                        (hash 'text "x" 'spans (list (hash 'start-char 0 'end-char 9 'scope "keyword")))
                        (car row)))
                  rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    assert!(
        ed.state.input.picker().is_some(),
        "setup: item29 is not shown yet"
    );
    for ch in "item29".chars() {
        ed.feed_key(key(ch));
    }
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(
        &mut ed,
        "row 29 'spans 0: end 9 is past the text's char length 1",
    );
}

/// The render proc reads the editor but cannot change it: a write is
/// refused even when the proc catches the error, and the picker closes.
#[test]
fn a_render_proc_that_changes_the_editor_closes_the_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 30)",
        r#"#:render (lambda (rows budget query)
             (with-handler (lambda (e) (begin))
                           (set-buffer-option! (focused-pane) "indent-width" 8))
             (map car rows))"#,
    );
    let before = shown(&mut ed, "indent-width");
    assert_ne!(before, "8", "setup: the write would change the option");
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed, "cannot be changed while rendering");
    assert_eq!(shown(&mut ed, "indent-width"), before);
}

/// A failed render closes the picker and every layer above it in the frame
/// it fails in, so no overlay synced earlier in that frame shows a layer
/// that is gone.
#[test]
fn a_failed_render_leaves_no_view_of_a_layer_above_the_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(items 0 2)",
        r#"#:render (lambda (rows budget query)
             (map (lambda (row)
                    (if (equal? (car row) "item500") (error "boom") (car row)))
                  rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    ed.state.push_layer(
        &ed.view,
        crate::editor::input_stack::MenuLayer::for_test(
            &["m0"],
            hume_scripting::Callback::for_test(steel::rvals::SteelVal::StringV("menu-cb".into())),
        ),
    );
    frame(&mut ed, WIDTH, HEIGHT);
    assert!(
        ed.state.views.menu.read().is_some(),
        "setup: the menu shows"
    );

    ed.execute_keymap_command("push-more", Count::UNTYPED, false);
    ed.settle();
    frame(&mut ed, WIDTH, HEIGHT);

    assert!(ed.state.input.picker().is_none(), "the picker closed");
    assert!(ed.state.input.menu().is_none(), "the menu above it closed");
    assert!(ed.state.views.menu.read().is_none(), "no menu shows");
}

/// Defines `:go`, which opens a live picker whose source answers a query
/// `q` with the one row `q`, with `extra` keywords, and runs it. Its render
/// shows a row as `<display@query>`.
fn open_live(ed: &mut Editor, tmp: &Path, extra: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"{PRELUDE}
(define-typed-command! "go" "" (lambda (pane)
  (live-picker! pane (test-picker-keys (lambda (x) (void)))
                #:source (lambda (token q) (picker-replace! token (list (cons q 'q))))
                #:render (lambda (rows budget query)
                           (map (lambda (row) (string-append "<" (car row) "@" query ">"))
                                rows))
                {extra})))"#
        ),
    );
    type_cmd(ed, ":go");
}

#[test]
fn live_picker_rows_go_through_render() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open_live(&mut ed, tmp.path(), "#:debounce-ms 0");
    for ch in "abc".chars() {
        ed.feed_key(key(ch));
    }
    ed.settle();
    frame(&mut ed, WIDTH, HEIGHT);

    assert_eq!(view_rows(&ed), vec![Some("<abc@abc>".to_string())]);
}

/// Rows shown again after a keystroke, before that query's search lands,
/// still get the query they were found for.
#[test]
fn a_live_pickers_old_rows_render_with_the_query_they_answer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open_live(&mut ed, tmp.path(), r#"#:query "a" #:debounce-ms 60000"#);
    ed.settle();
    frame(&mut ed, WIDTH, HEIGHT);
    assert_eq!(view_rows(&ed), vec![Some("<a@a>".to_string())]);

    ed.feed_key(key('b'));
    ed.settle();
    frame(&mut ed, WIDTH + 20, HEIGHT);

    assert_eq!(view_rows(&ed), vec![Some("<a@a>".to_string())]);
}

#[test]
fn snapshot_picker_with_rendered_styles() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();
    open(
        &mut ed,
        tmp.path(),
        "(items 0 3)",
        r#"#:render (lambda (rows budget query)
             (map (lambda (row)
                    (hash 'text (string-append "M  " (car row))
                          'spans (list (hash 'start-char 0 'end-char 1 'scope "keyword"))))
                  rows))"#,
    );

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 12));
    insta::assert_snapshot!(snap);
}

#[test]
fn a_plain_picker_shows_each_items_display_after_a_scroll() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(items 0 50)", "");
    frame(&mut ed, WIDTH, HEIGHT);
    let (list_rows, _) = panel(&ed);

    for _ in 0..list_rows + 3 {
        ed.feed_key(key_down());
        frame(&mut ed, WIDTH, HEIGHT);
    }

    assert_eq!(
        view_rows(&ed),
        (4..list_rows + 4)
            .map(|i| Some(format!("item{i}")))
            .collect::<Vec<_>>()
    );
}
