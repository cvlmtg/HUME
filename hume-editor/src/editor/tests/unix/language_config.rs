//! Per-language configuration: values `configure-language!` sets and values
//! filled from a language's bundled data file (`<runtime>/scheme/languages/`)
//! the first time a buffer uses it.

use super::*;

use super::super::scripting_grammar::{runtime_scheme_dir, shipped_grammar};
use hume_treesitter::registry::LanguageId;

/// A session over isolated `config`, `data` and `runtime` dirs. The runtime
/// holds the real `prelude.scm`, a caller-supplied `languages.scm` and one
/// bundled data file per `(name, body)`; `plugin` becomes the user plugin
/// `user/tp`. The config dir stays alive so a test can `:reload-config`.
struct LanguageFixture {
    init_path: PathBuf,
    _config_tmp: tempfile::TempDir,
    _data_tmp: tempfile::TempDir,
    _runtime_tmp: tempfile::TempDir,
    _path: PathReader,
}

impl LanguageFixture {
    fn boot(
        languages_scm: &str,
        bundled: &[(&str, &str)],
        init: &str,
        plugin: &str,
    ) -> (Editor, Self) {
        let path = path_reader();
        let config_tmp = tempfile::tempdir().unwrap();
        let data_tmp = tempfile::tempdir().unwrap();
        let runtime_tmp = tempfile::tempdir().unwrap();

        let hume_config = config_tmp.path().join("hume");
        std::fs::create_dir_all(&hume_config).unwrap();
        let init_path = hume_config.join("init.scm");
        std::fs::write(&init_path, init).unwrap();

        let hume_data = data_tmp.path().join("hume");
        let plugin_dir = hume_data.join("plugins").join("user").join("tp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.scm"), plugin).unwrap();

        let scheme_dir = runtime_tmp.path().join("scheme");
        let languages_dir = scheme_dir.join("languages");
        std::fs::create_dir_all(&languages_dir).unwrap();
        let prelude = std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap();
        std::fs::write(scheme_dir.join("prelude.scm"), prelude).unwrap();
        std::fs::write(scheme_dir.join("languages.scm"), languages_scm).unwrap();
        for (name, body) in bundled {
            std::fs::write(languages_dir.join(format!("{name}.scm")), body).unwrap();
        }

        let mut ed = editor_from("-[a]>b\n");
        ed.state.dirs = Dirs {
            config: Some(hume_config),
            data: Some(hume_data),
            runtime: Some(runtime_tmp.path().to_path_buf()),
            tmp: None,
        };
        ed.init_scripting(&mut Default::default());

        let fixture = Self {
            init_path,
            _config_tmp: config_tmp,
            _data_tmp: data_tmp,
            _runtime_tmp: runtime_tmp,
            _path: path,
        };
        (ed, fixture)
    }

    fn write_init(&self, init: &str) {
        std::fs::write(&self.init_path, init).unwrap();
    }
}

const TSX_LANGUAGES: &str = r#"(define-language! "tsx" '("tsx"))"#;
const TSX_BUNDLED: (&str, &str) = (
    "tsx",
    r#"((language-id . "typescriptreact") (roots "package.json"))"#,
);
const DECLARE_TSX_PLUGIN: &str = r#"(declare-plugin! "user/tp" #:languages '("tsx"))"#;

fn messages(ed: &Editor, severity: Severity) -> Vec<String> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == severity)
        .map(|e| e.text.clone())
        .collect()
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
    let (mut ed, _fixture) = LanguageFixture::boot(TSX_LANGUAGES, &[TSX_BUNDLED], "", "");
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
    let (mut ed, _fixture) = LanguageFixture::boot(
        TSX_LANGUAGES,
        &[TSX_BUNDLED],
        r#"(configure-language! "tsx" #:roots '("deno.json"))"#,
        "",
    );

    open_as_tsx(&mut ed);

    assert_eq!(messages(&ed, Severity::Error), Vec::<String>::new());
    assert_eq!(roots(&ed), ["deno.json"]);
    assert_eq!(
        lsp_language_id(&ed),
        "typescriptreact",
        "a field the configuration leaves unset still comes from the bundled file"
    );
}

#[test]
fn a_value_configured_by_a_plugin_activated_after_the_bundled_load_beats_it() {
    let (mut ed, _fixture) = LanguageFixture::boot(
        TSX_LANGUAGES,
        &[TSX_BUNDLED],
        DECLARE_TSX_PLUGIN,
        r#"(configure-language! "tsx" #:roots '("deno.json"))"#,
    );

    open_as_tsx(&mut ed);

    assert_eq!(messages(&ed, Severity::Error), Vec::<String>::new());
    assert_eq!(roots(&ed), ["deno.json"]);
}

#[test]
fn init_beats_a_plugin_and_the_ignored_plugin_value_is_a_warning() {
    let (mut ed, _fixture) = LanguageFixture::boot(
        TSX_LANGUAGES,
        &[TSX_BUNDLED],
        &format!("{DECLARE_TSX_PLUGIN}\n(configure-language! \"tsx\" #:roots '(\"user.json\"))"),
        r#"(configure-language! "tsx" #:roots '("plugin.json"))"#,
    );

    open_as_tsx(&mut ed);

    assert_eq!(roots(&ed), ["user.json"]);
    let warnings = messages(&ed, Severity::Warning);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("'tsx'") && w.contains("roots")),
        "the plugin's ignored roots must be reported; warnings: {warnings:?}"
    );
}

#[test]
fn reload_config_drops_configured_values_and_reloads_bundled_ones() {
    let (mut ed, fixture) = LanguageFixture::boot(
        TSX_LANGUAGES,
        &[TSX_BUNDLED],
        r#"(configure-language! "tsx" #:roots '("deno.json"))"#,
        "",
    );
    open_as_tsx(&mut ed);
    assert_eq!(roots(&ed), ["deno.json"]);

    fixture.write_init("");
    type_cmd(&mut ed, ":reload-config");

    assert_eq!(messages(&ed, Severity::Error), Vec::<String>::new());
    assert!(
        ed.state.config.languages.bundled_loaded(tsx(&ed)),
        "the buffer kept tsx across the reload, so its bundled file is read again"
    );
    assert_eq!(roots(&ed), ["package.json"]);
}

#[test]
fn a_malformed_bundled_file_is_an_error_and_the_buffer_keeps_its_language() {
    let (mut ed, _fixture) =
        LanguageFixture::boot(TSX_LANGUAGES, &[("tsx", "((roots 1))")], "", "");

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
    let (ed, _fixture) = LanguageFixture::boot(
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
    let (ed, _fixture) = LanguageFixture::boot(
        "",
        &[],
        "(define-language! \"mine\" '(\"mn\"))\n(configure-language! \"mine\" #:roots '(\"mine.toml\"))",
        "",
    );

    assert_eq!(messages(&ed, Severity::Error), Vec::<String>::new());
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
