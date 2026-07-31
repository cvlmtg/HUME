//! # Generated `.scm` header drift
//!
//! `scripts/sync-grammars.py` and `scripts/sync-lsp-sources.py` each hold a
//! Python triple-quoted `*_HEADER` template that becomes the leading comment
//! block of a generated `runtime/scheme/*.scm` file. Hand-editing the
//! generated file's header (as opposed to its data rows) drifts it from the
//! template with nothing to catch it — the next routine sync run silently
//! overwrites the hand-edit with the stale template text.
//! `generated_scm_headers_match_their_generator_templates` compares each
//! template against its generated file's leading lines verbatim, treating a
//! line containing a `{sha}`/`{tag}` format slot as matching on its prefix
//! before the `{` rather than byte-for-byte.

/// Pull `{const_name} = """\` … `"""`'s content out of a sync script's
/// source. None of these headers contain three consecutive `"`
/// characters, so searching for the raw `"""` delimiter (rather than
/// parsing Python string escapes) is exact here.
fn extract_header_template(script_src: &str, const_name: &str) -> String {
    let marker = format!("{const_name} = \"\"\"\\\n");
    let start = script_src
        .find(&marker)
        .unwrap_or_else(|| panic!("`{const_name} = \"\"\"\\` not found in sync script"))
        + marker.len();
    let rest = &script_src[start..];
    let end = rest
        .find("\"\"\"")
        .unwrap_or_else(|| panic!("no closing `\"\"\"` found for {const_name}"));
    rest[..end].to_string()
}

/// Compare `template` against `generated`'s leading lines. A template
/// line holding a `.format()` slot (`{sha}`, `{tag}`) matches on its
/// prefix up to the `{` rather than byte-for-byte, since the generated
/// file has the placeholder already substituted.
///
/// Also checks that `generated` has no *extra* `;;;` header lines beyond
/// what `template` accounts for — a hand-appended header paragraph the
/// template was never updated for is exactly the drift this lint exists
/// to catch, and comparing template lines only (with no check on
/// anything left over in `generated`) would miss it entirely.
fn header_drift(template: &str, generated: &str) -> Option<String> {
    let mut t_lines = template.lines().enumerate();
    let mut g_lines = generated.lines();
    for (i, t) in &mut t_lines {
        let lineno = i + 1;
        let Some(g) = g_lines.next() else {
            return Some(format!(
                "generated file has only {lineno} header line(s), template has more"
            ));
        };
        match t.find('{') {
            Some(brace) if !g.starts_with(&t[..brace]) => {
                return Some(format!(
                    "line {lineno}: template prefix {:?} not found in generated line {g:?}",
                    &t[..brace]
                ));
            }
            None if t != g => {
                return Some(format!("line {lineno}: template {t:?} != generated {g:?}"));
            }
            _ => {}
        }
    }
    if let Some(extra) = g_lines.next().filter(|line| line.starts_with(";;;")) {
        return Some(format!(
            "generated file has a header line the template doesn't account for: {extra:?}"
        ));
    }
    None
}

/// Fail oracle: append a header paragraph to a generated file without
/// touching its template — the false negative `header_drift` used to
/// have, since it only compared *template* lines and never checked
/// whether `generated` had anything left over. A template exactly as
/// long as the generated header (the common case) passed clean either
/// way, so this needs a case where `generated` genuinely has more.
#[test]
fn header_drift_catches_a_hand_appended_header_paragraph() {
    let template = ";;; one\n;;; two";
    let generated = ";;; one\n;;; two\n;;; three (hand-added, template never touched)";
    assert!(
        header_drift(template, generated).is_some(),
        "an extra ;;; header line beyond the template must be flagged as drift"
    );
}

/// A generated file's first non-header line (data, not `;;;` comment)
/// must not be misread as header drift — `header_drift` stops comparing
/// once the template runs out, so anything after the header proper is
/// out of scope for this check.
#[test]
fn header_drift_ignores_non_comment_lines_past_the_header() {
    let template = ";;; one\n;;; two";
    let generated = ";;; one\n;;; two\n(define-language! \"rust\" ...)";
    assert_eq!(
        header_drift(template, generated),
        None,
        "a data line past the header must not be flagged as header drift"
    );
}

/// Fail oracle: hand-edit a sentence in `runtime/scheme/languages.scm`'s
/// header without updating `LANGUAGES_HEADER` in `sync-grammars.py` —
/// this test must fail naming the diverging line.
#[test]
fn generated_scm_headers_match_their_generator_templates() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let workspace_root = std::path::Path::new(&manifest)
        .parent()
        .expect("workspace root");

    let triples = [
        (
            "scripts/sync-grammars.py",
            "LANGUAGES_HEADER",
            "runtime/scheme/languages.scm",
        ),
        (
            "scripts/sync-grammars.py",
            "GRAMMAR_SOURCES_HEADER",
            "runtime/scheme/grammar-sources.scm",
        ),
        (
            "scripts/sync-grammars.py",
            "LSP_SERVERS_HEADER",
            "runtime/scheme/lsp-servers.scm",
        ),
        (
            "scripts/sync-lsp-sources.py",
            "LSP_SOURCES_HEADER",
            "runtime/scheme/lsp-sources.scm",
        ),
    ];

    let mut violations: Vec<String> = Vec::new();
    for (script_rel, const_name, generated_rel) in triples {
        let script_path = workspace_root.join(script_rel);
        let script_src = std::fs::read_to_string(&script_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", script_path.display()));
        let template = extract_header_template(&script_src, const_name);

        let generated_path = workspace_root.join(generated_rel);
        let generated = std::fs::read_to_string(&generated_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", generated_path.display()));

        if let Some(reason) = header_drift(&template, &generated) {
            violations.push(format!(
                "{generated_rel} (template {const_name} in {script_rel}): {reason}"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "\nA generated runtime/scheme/*.scm header drifted from its sync script's \
         template — the next sync run will silently overwrite the hand-edit with \
         the stale template text. Update the *_HEADER constant to match.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
