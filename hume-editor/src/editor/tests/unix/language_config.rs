//! Per-language configuration: values `configure-language!` sets and values
//! filled from a language's bundled data file (`<runtime>/scheme/languages/`)
//! the first time a buffer uses it.

use super::*;

use super::super::scripting_grammar::{runtime_scheme_dir, shipped_grammar};
use hume_treesitter::registry::LanguageId;

/// A session over a [`ReloadFixture`] whose runtime holds the real
/// `prelude.scm`, a caller-supplied `languages.scm` and one bundled data file
/// per `(name, body)`; `plugin` becomes the user plugin `user/tp`.
fn boot(
    languages_scm: &str,
    bundled: &[(&str, &str)],
    init: &str,
    plugin: &str,
) -> (Editor, ReloadFixture) {
    let fixture = stage(languages_scm, bundled, init, plugin);
    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = fixture.dirs();
    ed.init_scripting(&mut Default::default());
    (ed, fixture)
}

/// The files [`boot`] runs against.
fn stage(languages_scm: &str, bundled: &[(&str, &str)], init: &str, plugin: &str) -> ReloadFixture {
    let fixture = ReloadFixture::new(init);

    let plugin_dir = fixture.data_dir.join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin).unwrap();

    let scheme_dir = fixture.runtime_dir.join("scheme");
    let languages_dir = scheme_dir.join("languages");
    std::fs::create_dir_all(&languages_dir).unwrap();
    let prelude = std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap();
    std::fs::write(scheme_dir.join("prelude.scm"), prelude).unwrap();
    std::fs::write(scheme_dir.join("languages.scm"), languages_scm).unwrap();
    for (name, body) in bundled {
        std::fs::write(languages_dir.join(format!("{name}.scm")), body).unwrap();
    }
    fixture
}

/// Start a session the way `hume <file>` does: `Editor::open` with `file`
/// as the startup buffer, then `init_scripting` over `dirs`.
fn boot_with_file(file: &str, dirs: Dirs, fixture: &ReloadFixture) -> Editor {
    let path = fixture.data_dir.join(file);
    std::fs::create_dir_all(&fixture.data_dir).unwrap();
    std::fs::write(&path, "x\n").unwrap();
    let mut ed =
        Editor::open(Some(path), std::sync::Arc::new(|| {}), dirs, test_cwd()).expect("open");
    ed.init_scripting(&mut Default::default());
    ed
}

fn queued_hook_names(ed: &Editor) -> Vec<&'static str> {
    ed.state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e.name()),
            crate::editor::event::PendingWork::Call { .. } => None,
        })
        .filter(|name| matches!(*name, "on-language-set" | "on-buffer-open"))
        .collect()
}

const TSX_LANGUAGES: &str = r#"(define-language! "tsx" '("tsx"))"#;
const TSX_BUNDLED: (&str, &str) = (
    "tsx",
    r#"((language-id . "typescriptreact") (roots "package.json"))"#,
);
const DECLARE_TSX_PLUGIN: &str = r#"(declare-plugin! "user/tp" #:languages '("tsx"))"#;

/// [`boot`] with `tsx` defined and its bundled file holding a language id
/// and roots.
fn boot_tsx(init: &str, plugin: &str) -> (Editor, ReloadFixture) {
    boot(TSX_LANGUAGES, &[TSX_BUNDLED], init, plugin)
}

fn messages(ed: &Editor, severity: Severity) -> Vec<String> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == severity)
        .map(|e| e.text.clone())
        .collect()
}

fn assert_no_errors(ed: &Editor) {
    assert_eq!(messages(ed, Severity::Error), Vec::<String>::new());
}

fn tsx(ed: &Editor) -> LanguageId {
    ed.state
        .config
        .languages
        .id_of("tsx")
        .expect("tsx must be defined")
}

/// Assign `tsx` to the focused buffer as a user would, which loads its
/// bundled data and activates `#:languages` plugins.
fn open_as_tsx(ed: &mut Editor) {
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some("tsx"));
}

fn roots(ed: &Editor) -> Vec<String> {
    ed.state.config.languages.roots_of(tsx(ed)).to_vec()
}

fn lsp_language_id(ed: &Editor) -> String {
    ed.state
        .config
        .languages
        .lsp_language_id_of(tsx(ed))
        .to_owned()
}

#[test]
fn bundled_data_is_applied_when_a_buffer_first_uses_the_language() {
    let (mut ed, _fixture) = boot_tsx("", "");
    assert!(
        !ed.state.config.languages.bundled_loaded(tsx(&ed)),
        "the bundled file must not be read before a buffer uses the language"
    );
    assert_eq!(lsp_language_id(&ed), "tsx");

    open_as_tsx(&mut ed);

    assert!(ed.state.config.languages.bundled_loaded(tsx(&ed)));
    assert_eq!(lsp_language_id(&ed), "typescriptreact");
    assert_eq!(roots(&ed), ["package.json"]);
}

#[test]
fn a_value_configured_in_init_beats_the_bundled_one() {
    let (mut ed, _fixture) = boot_tsx(r#"(configure-language! "tsx" #:roots '("deno.json"))"#, "");

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(roots(&ed), ["deno.json"]);
    assert_eq!(
        lsp_language_id(&ed),
        "typescriptreact",
        "a field the configuration leaves unset still comes from the bundled file"
    );
}

#[test]
fn a_value_configured_by_a_plugin_activated_after_the_bundled_load_beats_it() {
    let (mut ed, _fixture) = boot_tsx(
        DECLARE_TSX_PLUGIN,
        r#"(configure-language! "tsx" #:roots '("deno.json"))"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(roots(&ed), ["deno.json"]);
}

fn tsx_warnings(ed: &Editor) -> Vec<String> {
    messages(ed, Severity::Warning)
        .into_iter()
        .filter(|w| w.contains("'tsx'"))
        .collect()
}

#[test]
fn init_beats_a_plugin_activated_later_without_a_warning() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(configure-language! \"tsx\" #:roots '(\"user.json\"))"),
        r#"(configure-language! "tsx" #:roots '("plugin.json"))"#,
    );

    open_as_tsx(&mut ed);

    assert_eq!(roots(&ed), ["user.json"]);
    assert_eq!(tsx_warnings(&ed), Vec::<String>::new());
}

#[test]
fn init_beats_a_plugin_loaded_before_the_init_line() {
    let (mut ed, _fixture) = boot_tsx(
        "(load-plugin! \"user/tp\")\n(configure-language! \"tsx\" #:roots '(\"user.json\"))",
        r#"(configure-language! "tsx" #:roots '("plugin.json"))"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(roots(&ed), ["user.json"]);
    assert_eq!(tsx_warnings(&ed), Vec::<String>::new());
}

#[test]
fn a_second_init_value_is_ignored_with_a_warning() {
    let (mut ed, _fixture) = boot_tsx(
        "(configure-language! \"tsx\" #:roots '(\"first.json\"))\n\
         (configure-language! \"tsx\" #:roots '(\"second.json\"))",
        "",
    );

    open_as_tsx(&mut ed);

    assert_eq!(roots(&ed), ["first.json"]);
    let warnings = tsx_warnings(&ed);
    assert!(
        warnings.iter().any(|w| w.contains("roots")),
        "the ignored roots must be reported; warnings: {warnings:?}"
    );
}

#[test]
fn a_second_plugin_value_is_ignored_with_a_warning() {
    let (mut ed, _fixture) = boot_tsx(
        DECLARE_TSX_PLUGIN,
        "(configure-language! \"tsx\" #:roots '(\"first.json\"))\n\
         (configure-language! \"tsx\" #:roots '(\"second.json\"))",
    );

    open_as_tsx(&mut ed);

    assert_eq!(roots(&ed), ["first.json"]);
    let warnings = tsx_warnings(&ed);
    assert!(
        warnings.iter().any(|w| w.contains("roots")),
        "the ignored roots must be reported; warnings: {warnings:?}"
    );
}

const TSX_COMMENTS_BUNDLED: (&str, &str) = (
    "tsx",
    r#"((comment-tokens "//") (block-comment-tokens ("/*" . "*/")))"#,
);

/// The line tokens and block pairs configured or bundled for `tsx`, as
/// plain strings.
fn comment_tokens(ed: &Editor) -> (Vec<String>, Vec<(String, String)>) {
    let tokens = ed.state.config.languages.comment_tokens_of(tsx(ed));
    let line = tokens
        .line
        .map(|l| l.iter().map(str::to_owned).collect())
        .unwrap_or_default();
    let block = tokens
        .block
        .map(|b| {
            b.iter()
                .map(|p| (p.start().to_owned(), p.end().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    (line, block)
}

fn pair(start: &str, end: &str) -> (String, String) {
    (start.to_owned(), end.to_owned())
}

#[test]
fn bundled_comment_tokens_are_applied_when_a_buffer_first_uses_the_language() {
    let (mut ed, _fixture) = boot(TSX_LANGUAGES, &[TSX_COMMENTS_BUNDLED], "", "");

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(
        comment_tokens(&ed),
        (vec!["//".to_owned()], vec![pair("/*", "*/")])
    );
}

#[test]
fn comment_tokens_configured_in_init_beat_the_bundled_ones() {
    let (mut ed, _fixture) = boot(
        TSX_LANGUAGES,
        &[TSX_COMMENTS_BUNDLED],
        r##"(configure-language! "tsx" #:comment-tokens '("#" ";") #:block-comment-tokens '(("<!--" . "-->")))"##,
        "",
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(
        comment_tokens(&ed),
        (
            vec!["#".to_owned(), ";".to_owned()],
            vec![pair("<!--", "-->")]
        )
    );
}

#[test]
fn a_second_init_comment_token_value_is_ignored_with_a_warning() {
    let (mut ed, _fixture) = boot(
        TSX_LANGUAGES,
        &[],
        "(configure-language! \"tsx\" #:comment-tokens '(\"#\"))\n\
         (configure-language! \"tsx\" #:comment-tokens '(\";\") #:block-comment-tokens '((\"(*\" . \"*)\")))\n\
         (configure-language! \"tsx\" #:block-comment-tokens '((\"{-\" . \"-}\")))",
        "",
    );

    open_as_tsx(&mut ed);

    assert_eq!(
        comment_tokens(&ed),
        (vec!["#".to_owned()], vec![pair("(*", "*)")])
    );
    let warnings = tsx_warnings(&ed);
    assert!(
        warnings.iter().any(|w| w.contains("comment-tokens")),
        "the ignored comment-tokens must be reported; warnings: {warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("block-comment-tokens")),
        "the ignored block-comment-tokens must be reported; warnings: {warnings:?}"
    );
}

#[test]
fn a_malformed_comment_token_is_an_error_at_the_call() {
    let (mut ed, _fixture) = boot(
        TSX_LANGUAGES,
        &[],
        r#"(configure-language! "tsx" #:comment-tokens '(""))"#,
        "",
    );

    open_as_tsx(&mut ed);

    let errors = messages(&ed, Severity::Error);
    assert!(
        errors.iter().any(|e| e.contains("comment-tokens")),
        "the malformed field must be named; errors: {errors:?}"
    );
    assert_eq!(comment_tokens(&ed), (vec![], vec![]));
}

#[test]
fn reload_config_drops_configured_values_and_reloads_bundled_ones() {
    let (mut ed, fixture) = boot_tsx(r#"(configure-language! "tsx" #:roots '("deno.json"))"#, "");
    open_as_tsx(&mut ed);
    assert_eq!(roots(&ed), ["deno.json"]);

    fixture.write_init("");
    type_cmd(&mut ed, ":reload-config");

    assert_no_errors(&ed);
    assert!(
        ed.state.config.languages.bundled_loaded(tsx(&ed)),
        "the buffer kept tsx across the reload, so its bundled file is read again"
    );
    assert_eq!(roots(&ed), ["package.json"]);
}

#[test]
fn a_malformed_bundled_file_is_an_error_and_the_buffer_keeps_its_language() {
    let (mut ed, _fixture) = boot(TSX_LANGUAGES, &[("tsx", "((roots 1))")], "", "");

    open_as_tsx(&mut ed);

    let errors = messages(&ed, Severity::Error);
    assert!(
        errors.iter().any(|e| e.contains("tsx.scm")),
        "the malformed file must be named; errors: {errors:?}"
    );
    let bid = ed.focused_buffer_id();
    assert_eq!(ed.state.buffers.get(bid).language, Some(tsx(&ed)));
    assert!(roots(&ed).is_empty());
}

#[test]
fn configuring_an_undefined_language_is_an_error() {
    let (ed, _fixture) = boot(
        TSX_LANGUAGES,
        &[],
        r#"(configure-language! "nope" #:roots '("x"))"#,
        "",
    );

    let errors = messages(&ed, Severity::Error);
    assert!(
        errors.iter().any(|e| e.contains("'nope'")),
        "errors: {errors:?}"
    );
}

#[test]
fn configuring_a_language_defined_earlier_in_the_same_init_succeeds() {
    let (ed, _fixture) = boot(
        "",
        &[],
        "(define-language! \"mine\" '(\"mn\"))\n(configure-language! \"mine\" #:roots '(\"mine.toml\"))",
        "",
    );

    assert_no_errors(&ed);
    let languages = &ed.state.config.languages;
    let mine = languages.id_of("mine").expect("mine must be defined");
    assert_eq!(languages.roots_of(mine), ["mine.toml"]);
}

/// The shipped data gives a language that borrows another's grammar the
/// borrowed grammar's source and C symbol.
#[test]
fn the_shipped_jsx_file_carries_the_javascript_grammar() {
    let jsx = shipped_grammar("jsx");

    assert_eq!(jsx[2], "tree_sitter_javascript");
    assert_eq!(jsx, shipped_grammar("javascript"));
}

#[test]
fn the_startup_buffer_queues_on_language_set_before_on_buffer_open() {
    let fixture = stage(TSX_LANGUAGES, &[TSX_BUNDLED], "", "");
    let ed = boot_with_file("main.tsx", fixture.dirs(), &fixture);

    assert_no_errors(&ed);
    assert_eq!(
        queued_hook_names(&ed),
        ["on-language-set", "on-buffer-open"]
    );
}

#[test]
fn a_lazy_language_plugin_activates_for_the_startup_buffer_without_a_runtime_dir() {
    let init = r#"(%define-language! "tsx" '("tsx") '() '())
                  (declare-plugin! "user/tp" #:languages '("tsx"))"#;
    let plugin = r#"(%configure-language! "tsx" (hash 'roots '("plugin.json")))"#;
    let fixture = stage("", &[], init, plugin);
    let dirs = Dirs {
        runtime: None,
        ..fixture.dirs()
    };
    let ed = boot_with_file("main.tsx", dirs, &fixture);

    assert_no_errors(&ed);
    assert_eq!(roots(&ed), ["plugin.json"]);
}

#[test]
fn reload_config_leaves_a_read_only_view_without_a_language() {
    let (mut ed, _fixture) = boot(
        r#"(define-language! "python" '("py") '() '("python"))"#,
        &[],
        "",
        "",
    );
    let label = ViewLabel::named("report").expect("view name");
    ed.open_read_only_view(
        FocusedPane::current(&ed.state),
        &label,
        "#!/usr/bin/env python\nprint(1)\n",
        None,
    );
    ed.settle();

    type_cmd(&mut ed, ":reload-config");

    assert_no_errors(&ed);
    assert_eq!(ed.doc().display_name(), "[report]");
    assert_eq!(ed.doc().language, None);
}
