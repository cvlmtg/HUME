// Script calls that name a scope while the session already holds the most
// distinct scope names it lets in: every decoration setter, row list and
// popup format refuses a new name with an error naming its entry and keeps
// what it held, an existing name still works, and `:messages` still opens.

use test_fixtures::require_grammars;

use super::*;

/// The names this file uses as already interned.
const EXISTING: &[&str] = &["keyword", "string"];

/// The refusal the entry or row at `index` of a list raises, the list's
/// items labelled as `label`.
fn refusal(label: &str, index: usize) -> String {
    format!("{label} {index}: too many distinct scope names in this session (limit 65280)")
}

/// An editor on three lines running `source`, holding [`EXISTING`], and
/// then as many scope names as the session lets in.
fn editor_at_limit(tmp: &std::path::Path, source: &str) -> Editor {
    let mut ed = editor_from("-[a]>bc\ndef\nghi\n");
    run(&mut ed, tmp, source);
    fill(&mut ed);
    ed
}

/// Interns [`EXISTING`], then fills the registry.
fn fill(ed: &mut Editor) {
    for name in EXISTING {
        ed.view
            .registry
            .intern_name(name)
            .expect("an empty registry has room");
    }
    ed.view.registry.fill_to_limit();
}

fn names(ed: &Editor, scopes: impl IntoIterator<Item = ScopeId>) -> Vec<String> {
    scopes
        .into_iter()
        .map(|scope| ed.view.registry.name_of(scope).to_owned())
        .collect()
}

/// One decoration setter driven through Steel.
struct Setter {
    name: &'static str,
    /// The call, with `ENTRIES` standing for its entry list.
    call: &'static str,
    /// What the setter's error calls its entry list.
    label: &'static str,
    /// The `place`-th entry of a list, naming `scope`.
    entry: fn(scope: &str, place: usize) -> String,
    /// The scope names `source` `"probe"` holds for `bid`, in entry order.
    stored: fn(&Editor, BufferId) -> Vec<String>,
}

fn setters() -> Vec<Setter> {
    vec![
        Setter {
            name: "set-signs!",
            call: r#"(register-sign-source! pane "probe" 0) (set-signs! pane "probe" ENTRIES)"#,
            label: "signs",
            entry: |scope, place| format!(r#"(hash 'line {place} 'text "S" 'scope "{scope}")"#),
            stored: |ed, bid| {
                let entries = ed.state.config.decorations.signs_for("probe", bid);
                names(ed, entries.iter().map(|e| e.scope))
            },
        },
        Setter {
            name: "set-virtual-lines! 'scope",
            call: r#"(set-virtual-lines! pane "probe" ENTRIES)"#,
            label: "lines",
            entry: |scope, place| format!(r#"(hash 'line {place} 'text "v" 'scope "{scope}")"#),
            stored: |ed, bid| {
                let entries = ed.state.config.decorations.virtual_lines_for("probe", bid);
                names(ed, entries.iter().map(|e| e.scope))
            },
        },
        Setter {
            name: "set-virtual-lines! 'segments",
            call: r#"(set-virtual-lines! pane "probe" ENTRIES)"#,
            label: "lines",
            entry: |scope, place| {
                format!(
                    r#"(hash 'line {place} 'text "vv" 'segments (list (hash 'start-char-col 0 'end-char-col 1 'scope "{scope}")))"#
                )
            },
            stored: |ed, bid| {
                let entries = ed.state.config.decorations.virtual_lines_for("probe", bid);
                let segments = entries
                    .iter()
                    .flat_map(|e| e.content.spans().iter().map(|(_, scope)| *scope));
                names(ed, segments)
            },
        },
        Setter {
            name: "set-eol-text!",
            call: r#"(set-eol-text! pane "probe" ENTRIES)"#,
            label: "lines",
            entry: |scope, place| format!(r#"(hash 'line {place} 'text "e" 'scope "{scope}")"#),
            stored: |ed, bid| {
                let entries = ed.state.config.decorations.eol_text_for_buffer(bid);
                names(
                    ed,
                    entries
                        .filter(|(source, _)| *source == "probe")
                        .map(|(_, e)| e.scope),
                )
            },
        },
        Setter {
            name: "set-extra-highlights!",
            call: r#"(set-extra-highlights! pane "probe" ENTRIES)"#,
            label: "spans",
            entry: |scope, place| {
                let start = place * 2;
                format!(
                    r#"(hash 'start-char {start} 'end-char {} 'scope "{scope}")"#,
                    start + 1
                )
            },
            stored: |ed, bid| {
                let entries = ed
                    .state
                    .config
                    .decorations
                    .extra_highlights_for("probe", bid);
                names(ed, entries.iter().map(|e| e.scope))
            },
        },
        Setter {
            name: "set-line-backgrounds!",
            call: r#"(set-line-backgrounds! pane "probe" ENTRIES)"#,
            label: "entries",
            entry: |scope, place| format!(r#"(hash 'line {place} 'scope "{scope}")"#),
            stored: |ed, bid| {
                let entries = ed
                    .state
                    .config
                    .decorations
                    .line_backgrounds_for("probe", bid);
                names(ed, entries.iter().map(|e| e.scope))
            },
        },
        Setter {
            name: "set-conceals!",
            call: r#"(set-conceals! pane "probe" ENTRIES)"#,
            label: "entries",
            entry: |scope, place| {
                let start = place * 2;
                format!(
                    r#"(hash 'start-char {start} 'end-char {} 'text "C" 'scope "{scope}")"#,
                    start + 1
                )
            },
            stored: |ed, bid| {
                let entries = ed.state.config.decorations.conceals_for("probe", bid);
                let scopes = entries.iter().filter_map(|e| match &e.span {
                    hume_decorations::ConcealEntrySpan::Hidden { text, .. } => {
                        text.as_ref().and_then(|t| t.scope)
                    }
                    hume_decorations::ConcealEntrySpan::Inserted { text, .. } => text.scope,
                });
                names(ed, scopes)
            },
        },
    ]
}

/// Each setter stores an entry naming an existing scope, and refuses a list
/// whose entry 1 names a new one, naming that entry and storing nothing of
/// the list.
#[test]
fn a_decoration_naming_a_new_scope_is_refused_and_stores_nothing() {
    for setter in setters() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = |scopes: &[&str]| {
            let entries: Vec<String> = scopes
                .iter()
                .enumerate()
                .map(|(place, scope)| (setter.entry)(scope, place))
                .collect();
            setter
                .call
                .replace("ENTRIES", &format!("(list {})", entries.join(" ")))
        };
        let source = format!(
            r#"(define-typed-command! "existing" "" (lambda (pane) {}))
               (define-typed-command! "refused" "" (lambda (pane) {}))"#,
            entries(&["keyword"]),
            entries(&["string", "brand.new.scope"]),
        );
        let mut ed = editor_at_limit(tmp.path(), &source);
        let bid = ed.focused_buffer_id();

        type_cmd(&mut ed, ":existing");
        assert_eq!(error_log(&ed), Vec::<String>::new(), "{}", setter.name);
        assert_eq!((setter.stored)(&ed, bid), ["keyword"], "{}", setter.name);

        type_cmd(&mut ed, ":refused");
        let expected = refusal(setter.label, 1);
        assert!(
            has_error(&ed, &expected),
            "{}: expected an error containing {expected:?}, got {:?}",
            setter.name,
            error_log(&ed)
        );
        assert_eq!((setter.stored)(&ed, bid), ["keyword"], "{}", setter.name);
    }
}

/// A row naming `scope` in a span over its first char.
fn row(text: &str, scope: &str) -> String {
    format!(
        r#"(hash 'text "{text}" 'spans (list (hash 'start-char 0 'end-char 1 'scope "{scope}")))"#
    )
}

#[test]
fn show_menu_keeps_the_open_menu_when_a_row_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let source = format!(
        r#"(define-typed-command! "menus" "" (lambda (pane)
             (show-menu! pane (list (cons {first} 'first)) (lambda (choice) (log! 'info "first answered")))
             (show-menu! pane (list (cons {kept} 'kept) (cons {new} 'new)) (lambda (choice) (log! 'info "second answered")))))"#,
        first = row("first", "keyword"),
        kept = row("kept", "string"),
        new = row("new", "brand.new.menu"),
    );
    let mut ed = editor_at_limit(tmp.path(), &source);

    type_cmd(&mut ed, ":menus");

    let expected = refusal("items", 1);
    assert!(
        has_error(&ed, &expected),
        "expected an error containing {expected:?}, got {:?}",
        error_log(&ed)
    );
    let menu = ed.state.input.menu().expect("the first menu stays open");
    let rows: Vec<&str> = menu.rows().iter().map(|row| row.main.text()).collect();
    assert_eq!(rows, ["first"]);
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("first answered")),
        "the first menu's callback must not run"
    );
}

/// A refused menu opened by code outside a change in flight, here a timer,
/// leaves that change recording: its menus' later answers are part of it.
#[test]
fn a_refused_menu_leaves_the_change_in_flight_recording() {
    let tmp = tempfile::tempdir().unwrap();
    let source = format!(
        r#"(define-command! "pick" "" (lambda (pane)
             (show-menu! pane (list (cons "one" #\o))
               (lambda (c)
                 (show-menu! (focused-pane) (list (cons "two" #\t))
                   (lambda (c) (when c (call! "replace" (focused-pane) c))))))
             (after! 0 (lambda ()
               (show-menu! (focused-pane) (list (cons {new} 'new)) (lambda (c) #t)))))
           #:repeatable #t)"#,
        new = row("new", "brand.new.timer"),
    );
    let mut ed = editor_at_limit(tmp.path(), &source);
    ed.execute_keymap_command("pick", Count::typed(1), false);
    ed.settle();
    let expected = refusal("items", 0);
    assert!(
        has_error(&ed, &expected),
        "expected an error containing {expected:?}, got {:?}",
        error_log(&ed)
    );

    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(ed.doc().text().to_string(), "tbc\ndef\nghi\n");
    let recorded = ed
        .state
        .last_action()
        .map(|action| (action.command.to_string(), action.answers.len()));
    assert_eq!(recorded, Some(("pick".to_owned(), 2)));
}

/// A row a `#:render` proc returns naming a new scope closes the drawer,
/// naming the row by its place in the list.
#[test]
fn a_rendered_row_naming_a_new_scope_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let source = format!(
        r#"(define-typed-command! "drawer" "" (lambda (pane)
             (show-drawer-list! pane (list 'kept 'new) (lambda (row tok) #t)
               #:render (lambda (rows budget)
                          (map (lambda (r) (if (eq? r 'new) {new} {kept})) rows)))))"#,
        kept = row("kept", "string"),
        new = row("new", "brand.new.render"),
    );
    let mut ed = editor_at_limit(tmp.path(), &source);

    type_cmd(&mut ed, ":drawer");
    frame(&mut ed, 80, 24);

    let expected = refusal("row", 1);
    assert!(
        has_error(&ed, &expected),
        "expected an error containing {expected:?}, got {:?}",
        error_log(&ed)
    );
    assert!(ed.state.input.drawer().is_none());
}

#[test]
fn show_drawer_list_refuses_a_new_scope_row() {
    let tmp = tempfile::tempdir().unwrap();
    let source = format!(
        r#"(define-typed-command! "drawer" "" (lambda (pane)
             (show-drawer-list! pane (list (cons {first} 'first)) (lambda (row tok) (log! 'info "first answered")))))
           (define-typed-command! "refused" "" (lambda (pane)
             (show-drawer-list! pane (list (cons {kept} 'kept) (cons {new} 'new)) (lambda (row tok) #t))))"#,
        first = row("first", "keyword"),
        kept = row("kept", "string"),
        new = row("new", "brand.new.drawer"),
    );
    let mut ed = editor_at_limit(tmp.path(), &source);
    type_cmd(&mut ed, ":drawer");
    assert_eq!(drawer_rows(&ed), ["first"]);

    type_cmd(&mut ed, ":refused");

    let expected = refusal("items", 1);
    assert!(
        has_error(&ed, &expected),
        "expected an error containing {expected:?}, got {:?}",
        error_log(&ed)
    );
    assert_eq!(drawer_rows(&ed), ["first"]);
}

#[test]
fn update_drawer_list_leaves_the_rows_when_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let source = format!(
        r#"(define token #f)
           (define-typed-command! "drawer" "" (lambda (pane)
             (set! token (show-drawer-list! pane (list (cons {first} 'first)) (lambda (row tok) #t)))))
           (define-typed-command! "refused" "" (lambda (pane)
             (update-drawer-list! token (list (cons {kept} 'kept) (cons {new} 'new)) (lambda (row tok) #t) 0)))"#,
        first = row("first", "keyword"),
        kept = row("kept", "string"),
        new = row("new", "brand.new.update"),
    );
    let mut ed = editor_at_limit(tmp.path(), &source);
    type_cmd(&mut ed, ":drawer");
    assert_eq!(drawer_rows(&ed), ["first"]);

    type_cmd(&mut ed, ":refused");

    let expected = refusal("items", 1);
    assert!(
        has_error(&ed, &expected),
        "expected an error containing {expected:?}, got {:?}",
        error_log(&ed)
    );
    assert_eq!(drawer_rows(&ed), ["first"]);
}

#[test]
fn a_popup_format_with_a_new_scope_logs_and_shows_plain_text() {
    require_grammars(&["rust"]);
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "rust", "tree_sitter_rust");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "format-text/rust" "" (lambda (text parsed)
             (hash 'highlights (list (hash 'start-char 0 'end-char 1 'scope "brand.new.popup")))))
           (define-typed-command! "go" "" (lambda (pane) (show-popup! pane "ab" #:lang "rust")))"#,
    );
    fill(&mut ed);

    type_cmd(&mut ed, ":go");
    frame(&mut ed, 80, 24);

    assert!(
        has_error(
            &ed,
            "'highlights entry 0: too many distinct scope names in this session (limit 65280)"
        ),
        "expected the format's refusal, got {:?}",
        error_log(&ed)
    );
    let texts = ed
        .state
        .views
        .popup
        .read()
        .as_ref()
        .map(|popup| popup.rows.lines.texts())
        .expect("the popup stays open");
    assert_eq!(texts, ["ab"]);
}

/// `:messages` highlights with HUME's own scope names, which the limit does
/// not refuse.
#[test]
fn messages_opens_at_the_limit() {
    let mut ed = editor_from("-[a]>bc\n");
    fill(&mut ed);
    ed.state
        .message_log
        .push(Severity::Warning, "bad key".to_string());

    ed.execute_typed("messages", None).unwrap();

    let bid = ed.focused_buffer_id();
    let spans = ed
        .state
        .config
        .decorations
        .extra_highlights_for("messages", bid);
    assert_eq!(
        names(&ed, spans.iter().map(|e| e.scope)),
        [
            "diagnostic.warning.message",
            "diagnostic.warning.message-text"
        ]
    );
}
