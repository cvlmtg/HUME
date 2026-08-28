use super::super::testing::*;
use super::*;
use hume_treesitter::registry::LanguageRegistry;

// ── SetCompleter: scope phase ─────────────────────────────────────────────

fn set_result(input: &str) -> CompletionResult {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    SetCompleter.complete(input, input.len(), &ctx)
}

fn names_of(result: &CompletionResult) -> Vec<&str> {
    result
        .candidates
        .iter()
        .map(|c| c.replacement.as_str())
        .collect()
}

#[test]
fn set_completer_scope_empty_prefix_lists_all_scopes() {
    let result = set_result("set ");
    assert_eq!(result.span_start, 4);
    assert_eq!(names_of(&result), vec!["buffer", "global", "pane"]);
}

#[test]
fn set_completer_scope_prefix_filters() {
    let result = set_result("set g");
    assert_eq!(result.span_start, 4);
    assert_eq!(names_of(&result), vec!["global"]);
}

#[test]
fn set_completer_scope_exact_match_excluded() {
    let result = set_result("set global");
    assert!(result.candidates.is_empty());
}

// ── SetCompleter: key phase ───────────────────────────────────────────────

#[test]
fn set_completer_keys_for_global_scope() {
    let result = set_result("set global ");
    assert_eq!(result.span_start, 11);
    let names = names_of(&result);
    assert!(!names.is_empty());
    assert!(
        names.contains(&"scrolloff"),
        "global-only key should appear"
    );
    assert!(
        names.contains(&"tab-width"),
        "global+buffer key should appear"
    );
    assert!(
        names.contains(&"wrap-mode"),
        "global+buffer+pane key should appear"
    );
    assert!(names.contains(&"statusline"), "hand-listed global key");
    assert!(!names.contains(&"language"), "language has no global scope");
}

#[test]
fn set_completer_keys_for_buffer_scope_includes_language() {
    let result = set_result("set buffer ");
    assert_eq!(result.span_start, 11);
    let names = names_of(&result);
    assert!(names.contains(&"language"), "language is buffer-only");
    assert!(names.contains(&"tab-width"), "buffer-overridable key");
    assert!(
        names.contains(&"wrap-mode"),
        "wrap-mode is buffer-overridable too, not just global+pane"
    );
    assert!(
        !names.contains(&"scrolloff"),
        "global-only key must not appear under buffer scope"
    );
}

#[test]
fn set_completer_keys_for_pane_scope_only_wrap_mode() {
    let result = set_result("set pane ");
    assert_eq!(result.span_start, 9);
    assert_eq!(names_of(&result), vec!["wrap-mode"]);
}

#[test]
fn set_completer_key_prefix_filters() {
    let result = set_result("set global tab");
    assert_eq!(result.span_start, 11);
    let names = names_of(&result);
    assert!(names.contains(&"tab-width"));
    assert!(names.contains(&"tab-style"));
    assert!(names.iter().all(|n| n.starts_with("tab")));
}

#[test]
fn set_completer_key_exact_match_excluded() {
    let result = set_result("set global tab-width");
    assert!(!names_of(&result).contains(&"tab-width"));
}

// ── SetCompleter: value phase (static enums / bools) ──────────────────────

#[test]
fn set_completer_value_bool_offers_true_false() {
    let result = set_result("set global mouse-enabled=");
    assert_eq!(result.span_start, "set global mouse-enabled=".len());
    assert_eq!(names_of(&result), vec!["false", "true"]);
}

#[test]
fn set_completer_value_tab_style() {
    let result = set_result("set buffer tab-style=");
    assert_eq!(names_of(&result), vec!["hard", "soft"]);
}

#[test]
fn set_completer_value_wrap_mode() {
    let result = set_result("set global wrap-mode=");
    assert_eq!(names_of(&result), vec!["indent", "none", "soft", "word"]);
}

#[test]
fn set_completer_value_whitespace_render() {
    let result = set_result("set buffer whitespace-space=");
    assert_eq!(names_of(&result), vec!["all", "none", "trailing"]);
}

#[test]
fn set_completer_value_signcolumn() {
    let result = set_result("set buffer signcolumn=");
    assert_eq!(
        names_of(&result),
        vec!["always", "always:1", "always:2", "auto", "auto:1", "auto:2"]
    );
}

#[test]
fn set_completer_value_whitespace_newline() {
    // Newline has no "trailing" axis — only none/all.
    let result = set_result("set buffer whitespace-newline=");
    assert_eq!(names_of(&result), vec!["all", "none"]);
}

#[test]
fn set_completer_value_prefix_filters() {
    let result = set_result("set buffer tab-style=s");
    assert_eq!(result.span_start, "set buffer tab-style=".len());
    assert_eq!(names_of(&result), vec!["soft"]);
}

#[test]
fn set_completer_value_exact_match_excluded() {
    let result = set_result("set buffer tab-style=hard");
    assert!(result.candidates.is_empty());
}

#[test]
fn set_completer_value_numeric_no_candidates() {
    let result = set_result("set global scrolloff=");
    assert!(result.candidates.is_empty());
}

#[test]
fn set_completer_value_static_enum_rejects_ineligible_scope() {
    // tab-style is global/buffer-scoped, not pane-scoped — completion
    // must not offer values for a scope the key doesn't accept, matching
    // the error `typed_set` would give on Enter.
    let result = set_result("set pane tab-style=");
    assert!(result.candidates.is_empty());
}

#[test]
fn set_completer_value_static_bool_rejects_unknown_scope() {
    let result = set_result("set bogus mouse-enabled=");
    assert!(result.candidates.is_empty());
}

#[test]
fn set_completer_value_span_start_stops_at_equals_not_internal_space() {
    // A value can legitimately contain spaces (e.g. a theme filename stem
    // like "my theme"). The replacement span must start right after '=',
    // not after the last internal space — otherwise completion would
    // replace only the tail after the space and duplicate the rest
    // (e.g. "set global theme=my my theme").
    let result = set_result("set global theme=my theme");
    assert_eq!(result.span_start, "set global theme=".len());
}

// ── SetCompleter: stray whitespace robustness ─────────────────────────────
//
// A naive first-space split collapses the parsed scope to "" when extra
// whitespace appears anywhere before the key token (e.g. a double
// space-bar tap), silently emptying the popup. These pin the fix.

#[test]
fn set_completer_double_space_after_set_still_lists_buffer_keys() {
    let result = set_result("set  buffer ");
    let names = names_of(&result);
    assert!(
        names.contains(&"language"),
        "buffer scope should resolve despite double space"
    );
    assert!(names.contains(&"tab-width"));
}

#[test]
fn set_completer_double_space_before_key_still_filters() {
    let result = set_result("set global  tab");
    let names = names_of(&result);
    assert!(
        !names.is_empty(),
        "scope should resolve despite double space"
    );
    assert!(names.iter().all(|n| n.starts_with("tab")));
}

#[test]
fn set_completer_double_space_before_value_still_offers_bools() {
    let result = set_result("set global  mouse-enabled=");
    assert_eq!(names_of(&result), vec!["false", "true"]);
}

// ── SetCompleter: value phase (language from registry) ────────────────────

#[test]
fn set_completer_value_language_from_registry() {
    let (reg, store, dir) = make_ctx_parts();
    let mut langs = LanguageRegistry::new();
    langs
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    langs
        .register_identity("ruby", &["rb"], &[], &[], None)
        .unwrap();
    let ctx = ctx_with(&reg, &store, dir.path(), &langs);
    let result = SetCompleter.complete("set buffer language=", 21, &ctx);
    let names = names_of(&result);
    assert!(names.contains(&"rust"));
    assert!(names.contains(&"ruby"));
    assert_eq!(result.span_start, "set buffer language=".len());
}

#[test]
fn set_completer_value_language_only_buffer_scope() {
    // `:set global language=` is invalid; the completer must not offer
    // language names under a non-buffer scope.
    let (reg, store, dir) = make_ctx_parts();
    let mut langs = LanguageRegistry::new();
    langs
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    let ctx = ctx_with(&reg, &store, dir.path(), &langs);
    let result = SetCompleter.complete("set global language=", 21, &ctx);
    assert!(result.candidates.is_empty());
}

#[test]
fn set_completer_value_language_prefix_filters() {
    let (reg, store, dir) = make_ctx_parts();
    let mut langs = LanguageRegistry::new();
    langs
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    langs
        .register_identity("ruby", &["rb"], &[], &[], None)
        .unwrap();
    let ctx = ctx_with(&reg, &store, dir.path(), &langs);
    let result = SetCompleter.complete("set buffer language=ru", 22, &ctx);
    let names = names_of(&result);
    assert!(names.contains(&"rust"));
    assert!(names.contains(&"ruby"));
}

#[test]
fn set_completer_value_language_excludes_exact_match() {
    // The completer must drop a language whose name equals the typed
    // prefix — Tab on a fully-typed value is a no-op.
    let (reg, store, dir) = make_ctx_parts();
    let mut langs = LanguageRegistry::new();
    langs.register_identity("ru", &[], &[], &[], None).unwrap();
    langs
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    let ctx = ctx_with(&reg, &store, dir.path(), &langs);
    let result = SetCompleter.complete("set buffer language=ru", 22, &ctx);
    let names = names_of(&result);
    assert!(names.contains(&"rust"));
    assert!(!names.contains(&"ru"));
}
