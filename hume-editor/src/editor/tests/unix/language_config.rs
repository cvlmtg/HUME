//! Per-language configuration: values `configure-language!` sets and values
//! filled from a language's bundled data file (`<runtime>/scheme/languages/`)
//! the first time a buffer uses it.

use super::*;

use super::super::scripting_grammar::{
    grammar_fixture, grammar_warnings, prelude_source, shipped_grammar,
};
use hume_core::{BlockCommentTokens, CommentTokens, LineCommentTokens};
use hume_treesitter::registry::LanguageId;

/// A session over a [`ReloadFixture`] whose runtime holds the real
/// `prelude.scm`, a caller-supplied `languages.scm` and one bundled data file
/// per `(name, body)`; `plugin` becomes the user plugin `user/tp`.
pub(super) fn boot(
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
    let prelude = prelude_source();
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
    let mut ed = Editor::open(
        Some(AbsolutePath::rooted(path)),
        crate::editor::run::HEADLESS_TERMINAL,
        no_op_wake(),
        dirs,
        test_cwd(),
    )
    .expect("open");
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

/// `text` was reported at Trace.
fn assert_traced(ed: &Editor, text: &str) {
    let traced = messages(ed, Severity::Trace);
    assert!(traced.iter().any(|m| m == text), "{text:?} in {traced:?}");
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
    ed.set_buffer_language_by_name(bid, Some("tsx"), hume_core::ConfigOrigin::User);
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

/// The line tokens and block pairs configured or bundled for `tsx`.
fn comment_tokens(ed: &Editor) -> (Option<LineCommentTokens>, Option<BlockCommentTokens>) {
    let tokens = ed.state.config.languages.comment_tokens_of(tsx(ed));
    (
        tokens.and_then(CommentTokens::line).cloned(),
        tokens.and_then(CommentTokens::block).cloned(),
    )
}

#[test]
fn bundled_comment_tokens_are_applied_when_a_buffer_first_uses_the_language() {
    let (mut ed, _fixture) = boot(TSX_LANGUAGES, &[TSX_COMMENTS_BUNDLED], "", "");

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(
        comment_tokens(&ed),
        (
            Some(LineCommentTokens::from_strs(&["//"])),
            Some(BlockCommentTokens::from_pairs(&[("/*", "*/")]))
        )
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
            Some(LineCommentTokens::from_strs(&["#", ";"])),
            Some(BlockCommentTokens::from_pairs(&[("<!--", "-->")]))
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
        (
            Some(LineCommentTokens::from_strs(&["#"])),
            Some(BlockCommentTokens::from_pairs(&[("(*", "*)")]))
        )
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
    assert_eq!(comment_tokens(&ed), (None, None));
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

    assert_eq!(jsx.symbol, "tree_sitter_javascript");
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
        FocusedPane::current(&ed.view),
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

// ── Detection and server registrations, ranked by origin ─────────────────

fn detected(ed: &Editor, file_name: &str) -> Option<LanguageId> {
    ed.state.config.languages.by_file_name(file_name)
}

#[test]
fn init_detection_beats_a_plugin_activated_later() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(define-language! \"tsx\" '(\"tsx\" \"utsx\"))"),
        r#"(define-language! "tsx" '("ptsx"))"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(detected(&ed, "a.utsx"), Some(tsx(&ed)));
    assert_eq!(detected(&ed, "a.ptsx"), None);
    assert_eq!(tsx_warnings(&ed), Vec::<String>::new());
    assert_traced(&ed, SHADOWED_TSX_DEFINITION);
}

#[test]
fn a_plugin_definition_replaces_the_bundled_one() {
    let (mut ed, _fixture) = boot_tsx(DECLARE_TSX_PLUGIN, r#"(define-language! "tsx" '("ptsx"))"#);

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(detected(&ed, "a.ptsx"), Some(tsx(&ed)));
    assert_eq!(detected(&ed, "a.tsx"), None);
}

#[test]
fn a_second_init_definition_is_ignored_with_a_warning() {
    let (ed, _fixture) = boot_tsx(
        "(define-language! \"tsx\" '(\"first\"))\n(define-language! \"tsx\" '(\"second\"))",
        "",
    );

    assert_eq!(detected(&ed, "a.first"), Some(tsx(&ed)));
    assert_eq!(detected(&ed, "a.second"), None);
    let warnings = tsx_warnings(&ed);
    assert!(
        warnings.iter().any(|w| w.contains("define-language!")),
        "the ignored definition must be reported; warnings: {warnings:?}"
    );
}

#[test]
fn a_value_configured_in_a_bundled_file_ranks_below_init() {
    let (mut ed, _fixture) = boot(
        &format!("{TSX_LANGUAGES}\n(configure-language! \"tsx\" #:roots '(\"bundled.json\"))"),
        &[TSX_BUNDLED],
        r#"(configure-language! "tsx" #:roots '("user.json"))"#,
        "",
    );

    open_as_tsx(&mut ed);

    assert_eq!(roots(&ed), ["user.json"]);
    assert_eq!(tsx_warnings(&ed), Vec::<String>::new());
}

/// `(register-grammar! "tsx" …)` over the json fixture grammar, which loads.
fn register_json_as_tsx() -> String {
    let (library, highlights) = grammar_fixture("json");
    format!(
        "(register-grammar! \"tsx\" {:?} \"tree_sitter_json\" {:?})",
        library.display().to_string(),
        highlights.display().to_string()
    )
}

const SHADOWED_TSX_GRAMMAR: &str = "register-grammar!: a higher origin's 'tsx' grammar applies";

const SHADOWED_TSX_DEFINITION: &str =
    "define-language!: a higher origin's 'tsx' definition applies";

const REGISTER_BOGUS_TSX: &str = r#"(register-grammar! "tsx" "/nonexistent/tsx.so" "tree_sitter_tsx" "/nonexistent/highlights.scm")"#;

/// The focused buffer, opened as `tsx`, has the json fixture grammar loaded
/// and nothing reported a grammar failure.
fn assert_tsx_has_the_users_grammar(ed: &Editor) {
    assert_no_errors(ed);
    assert_eq!(grammar_warnings(ed, "tsx"), Vec::<String>::new());
    assert!(
        ed.state.config.languages.loaded_grammar(tsx(ed)).is_some(),
        "the user's grammar loaded"
    );
}

#[test]
fn init_grammar_beats_a_plugin_activated_later() {
    test_fixtures::require_grammars(&["json"]);
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n{}", register_json_as_tsx()),
        REGISTER_BOGUS_TSX,
    );

    open_as_tsx(&mut ed);

    assert_tsx_has_the_users_grammar(&ed);
    assert_traced(&ed, SHADOWED_TSX_GRAMMAR);
}

#[test]
fn init_grammar_beats_a_plugin_loaded_after_it() {
    test_fixtures::require_grammars(&["json"]);
    let (mut ed, _fixture) = boot_tsx(
        &format!("{}\n(load-plugin! \"user/tp\")", register_json_as_tsx()),
        REGISTER_BOGUS_TSX,
    );

    open_as_tsx(&mut ed);

    assert_tsx_has_the_users_grammar(&ed);
    assert_traced(&ed, SHADOWED_TSX_GRAMMAR);
}

/// The command `K` runs in Normal mode.
fn k_command(ed: &Editor) -> Option<String> {
    ed.state
        .config
        .keymap
        .lookup_command(hume_core::BindMode::Normal, &[key('K')])
        .map(|(name, _)| name)
}

#[test]
fn init_binding_beats_a_plugin_activated_later() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(bind-key! 'normal \"K\" \"move-right\")"),
        r#"(bind-key! 'normal "K" "move-left")"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(k_command(&ed).as_deref(), Some("move-right"));
}

#[test]
fn a_plugins_unbind_leaves_the_users_binding() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(bind-key! 'normal \"K\" \"move-right\")"),
        r#"(unbind-key! 'normal "K")"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(k_command(&ed).as_deref(), Some("move-right"));
}

/// The focused buffer's `indent-width` override, `None` when it has none.
fn indent_override(ed: &Editor) -> Option<u8> {
    ed.state
        .buffers
        .get(ed.focused_buffer_id())
        .overrides
        .indent_width
}

#[test]
fn init_option_beats_a_plugin_activated_later_and_fires_no_change() {
    let (mut ed, _fixture) = boot_tsx(
        &format!(
            "{DECLARE_TSX_PLUGIN}\n\
             (set-option! \"indent-width\" 2)\n\
             (register-hook! 'on-option-change\n\
               (lambda (key value) (log! 'warning (string-append \"changed:\" key))))"
        ),
        r#"(set-option! "indent-width" 8)"#,
    );

    open_as_tsx(&mut ed);
    ed.settle();

    assert_no_errors(&ed);
    assert_eq!(ed.state.settings.indent_width, 2);
    let changes = messages(&ed, Severity::Warning)
        .into_iter()
        .filter(|m| m == "changed:indent-width")
        .count();
    assert_eq!(changes, 1, "only the user's own write fires the hook");
}

#[test]
fn a_set_command_replaces_a_plugins_option() {
    let (mut ed, _fixture) = boot_tsx(DECLARE_TSX_PLUGIN, r#"(set-option! "indent-width" 8)"#);
    open_as_tsx(&mut ed);
    assert_eq!(ed.state.settings.indent_width, 8);

    type_cmd(&mut ed, ":set global indent-width=3");

    assert_eq!(ed.state.settings.indent_width, 3);
}

#[test]
fn a_plugin_command_the_user_runs_replaces_the_users_option() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(set-option! \"indent-width\" 2)"),
        r#"(set-option! "indent-width" 8)
           (define-typed-command! "tp-indent" ""
             (lambda () (set-option! "indent-width" 3)))"#,
    );
    open_as_tsx(&mut ed);
    assert_eq!(ed.state.settings.indent_width, 2);

    type_cmd(&mut ed, ":tp-indent");

    assert_no_errors(&ed);
    assert_eq!(ed.state.settings.indent_width, 3);
}

#[test]
fn a_users_buffer_override_beats_a_plugin_hook_that_runs_later() {
    let (mut ed, _fixture) = boot_tsx(
        "(register-hook! 'on-language-set\n\
           (lambda (pane lang) (set-buffer-option! pane \"indent-width\" 2)))\n\
         (load-plugin! \"user/tp\")",
        r#"(register-hook! 'on-language-set
             (lambda (pane lang) (set-buffer-option! pane "indent-width" 8)))"#,
    );

    open_as_tsx(&mut ed);
    ed.settle();

    assert_no_errors(&ed);
    assert_eq!(indent_override(&ed), Some(2));
}

#[test]
fn a_plugins_buffer_override_beats_the_users_global_value() {
    let (mut ed, _fixture) = boot_tsx(
        "(set-option! \"indent-width\" 2)\n(load-plugin! \"user/tp\")",
        r#"(register-hook! 'on-language-set
             (lambda (pane lang) (set-buffer-option! pane "indent-width" 8)))"#,
    );

    open_as_tsx(&mut ed);
    ed.settle();

    assert_no_errors(&ed);
    assert_eq!(ed.state.settings.indent_width, 2);
    assert_eq!(indent_override(&ed), Some(8));
}

const ALPHA_BETA_LANGUAGES: &str = r#"(define-language! "tsx" '("tsx"))
(define-language! "alpha" '("alpha"))
(define-language! "beta" '("beta"))"#;

/// An `on-language-set` hook that sets `key` to `value` on a buffer that
/// becomes `tsx`.
fn on_tsx(key: &str, value: &str) -> String {
    format!(
        "(register-hook! 'on-language-set\n\
           (lambda (pane lang)\n\
             (when (equal? lang \"tsx\") (set-buffer-option! pane {key:?} {value:?}))))"
    )
}

/// The focused buffer's language name.
fn language(ed: &Editor) -> Option<String> {
    let buf = ed.state.buffers.get(ed.focused_buffer_id());
    buf.language
        .map(|id| ed.state.config.languages.name_of(id).to_owned())
}

/// Detect the focused buffer as `tsx`, as opening a `.tsx` file does.
fn detect_as_tsx(ed: &mut Editor) {
    let bid = ed.focused_buffer_id();
    let tsx = tsx(ed);
    ed.set_buffer_language(bid, Some(tsx));
    ed.settle();
}

#[test]
fn a_users_language_beats_a_plugin_hook_that_runs_later() {
    let (mut ed, _fixture) = boot(
        ALPHA_BETA_LANGUAGES,
        &[],
        &format!(
            "{}\n(load-plugin! \"user/tp\")",
            on_tsx("language", "alpha")
        ),
        &on_tsx("language", "beta"),
    );

    detect_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(language(&ed).as_deref(), Some("alpha"));
}

#[test]
fn a_set_command_replaces_a_plugins_language() {
    let (mut ed, _fixture) = boot(
        ALPHA_BETA_LANGUAGES,
        &[],
        "(load-plugin! \"user/tp\")",
        &on_tsx("language", "beta"),
    );
    detect_as_tsx(&mut ed);
    assert_eq!(language(&ed).as_deref(), Some("beta"));

    type_cmd(&mut ed, ":set buffer language=alpha");

    assert_eq!(language(&ed).as_deref(), Some("alpha"));
}

#[test]
fn a_users_line_ending_beats_a_plugin_hook_that_runs_later() {
    let (mut ed, _fixture) = boot(
        ALPHA_BETA_LANGUAGES,
        &[],
        &format!(
            "{}\n(load-plugin! \"user/tp\")",
            on_tsx("line-ending", "crlf")
        ),
        &on_tsx("line-ending", "lf"),
    );

    detect_as_tsx(&mut ed);

    assert_no_errors(&ed);
    let buf = ed.state.buffers.get(ed.focused_buffer_id());
    assert_eq!(
        buf.text().line_ending(),
        hume_editing::text::LineEnding::CrLf
    );
}

#[test]
fn a_reload_keeps_the_origin_of_an_asserted_language() {
    let (mut ed, _fixture) = boot(ALPHA_BETA_LANGUAGES, &[], "", "");
    type_cmd(&mut ed, ":set buffer language=alpha");

    type_cmd(&mut ed, ":reload-config");

    assert_eq!(language(&ed).as_deref(), Some("alpha"));
    assert_eq!(
        ed.state
            .buffers
            .get(ed.focused_buffer_id())
            .language_by
            .by(),
        Some(hume_core::ConfigOrigin::User)
    );
}

#[test]
fn init_server_registration_beats_a_plugin_activated_later() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(register-lsp-server! \"tsls\" #:command \"user-tsls\")"),
        r#"(register-lsp-server! "tsls" #:command "plugin-tsls")
           (unregister-lsp-server! "tsls")
           (register-lsp-server! "tsls" #:command "plugin-tsls")"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    assert_eq!(
        ed.state.lsp.registered_command_for_test("tsls").as_deref(),
        Some("user-tsls")
    );
}

#[test]
fn init_server_list_beats_a_plugin_activated_later() {
    let (mut ed, _fixture) = boot_tsx(
        &format!(
            "{DECLARE_TSX_PLUGIN}\n\
             (register-lsp-server! \"tsls\" #:command \"tsls\")\n\
             (register-lsp-server! \"eslint\" #:command \"eslint\")\n\
             (set-language-servers! \"tsx\" '(\"tsls\"))"
        ),
        r#"(set-language-servers! "tsx" '("eslint"))
           (set-language-servers! "tsx" #f)
           (set-language-servers! "tsx" '("eslint"))"#,
    );

    open_as_tsx(&mut ed);

    assert_no_errors(&ed);
    let servers: Vec<String> =
        crate::editor::lsp::introspect::language_servers(&ed.state.lsp, "tsx")
            .into_iter()
            .map(|entry| entry.name.to_string())
            .collect();
    assert_eq!(servers, ["tsls"]);
}

#[test]
fn a_plugins_unregister_leaves_the_users_registration_and_the_probe_agrees() {
    let (mut ed, _fixture) = boot_tsx(
        &format!("{DECLARE_TSX_PLUGIN}\n(register-lsp-server! \"tsls\" #:command \"user-tsls\")"),
        r#"(unregister-lsp-server! "tsls")
           (log! 'warning (if (lsp-server-registered? "tsls") "probe:registered" "probe:gone"))"#,
    );

    open_as_tsx(&mut ed);

    assert_eq!(
        ed.state.lsp.registered_command_for_test("tsls").as_deref(),
        Some("user-tsls")
    );
    assert!(
        messages(&ed, Severity::Warning).contains(&"probe:registered".to_string()),
        "{:?}",
        messages(&ed, Severity::Warning)
    );
}
