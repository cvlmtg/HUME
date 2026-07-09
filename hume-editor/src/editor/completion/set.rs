use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceRender, WrapMode};

use super::{Completer, Completion, CompletionCtx, CompletionResult, theme_name_candidates};
use crate::settings::{TabStyle, all_setting_keys, setting_scopes};

// ── SetCompleter ──────────────────────────────────────────────────────────────

/// Completes `:set <scope> <key>=<value>` arguments.
///
/// Three phases, selected by cursor position within the argument:
/// - **scope** (no space yet) — offers `global`/`buffer`/`pane`.
/// - **key** (space present, no `=` yet) — offers every setting key whose
///   declared scopes include the chosen scope, plus `language` for `buffer`.
/// - **value** (`=` present) — offers the valid value set for enum/bool keys,
///   registered language names for `language`, installed theme names for
///   `theme`. Numeric/free-form keys (e.g. `scrolloff`, `statusline`) get no
///   candidates — the user types them and `apply_setting` validates.
///
/// Value lists are completion *hints* mirrored from each setting's parser;
/// `apply_setting` remains the validation SSOT, so the two can drift only in
/// what's offered, never in what's accepted.
pub(crate) struct SetCompleter;

/// The three `:set` scopes. `pane` exists only because `wrap-mode` declares it.
const SET_SCOPES: &[&str] = &["global", "buffer", "pane"];

/// Prefix-filter `items`, dropping an exact match (Tab on a fully-typed value
/// is a no-op), and wrap each into a `Completion`. Caller sorts.
fn prefix_completions<'a>(items: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<Completion> {
    items
        .filter(|s| s.starts_with(prefix) && *s != prefix)
        .map(|s| Completion {
            replacement: s.to_owned(),
            display: s.to_owned(),
        })
        .collect()
}

/// Static value candidates for enum/bool keys. Returns `None` for keys whose
/// values are dynamic (`language`, `theme`) or free-form (numbers,
/// `statusline`) — those are handled in [`SetCompleter::complete`].
fn static_value_candidates(key: &str) -> Option<&'static [&'static str]> {
    // Bool keys are derived from `define_settings!`'s `parser: bool` — not
    // hand-listed — so a new bool setting gets value completion for free.
    if crate::settings::is_bool_setting(key) {
        return Some(&["true", "false"]);
    }
    Some(match key {
        "tab-style" => TabStyle::VALUES,
        "line-number-style" => LineNumberStyle::VALUES,
        "wrap-mode" => WrapMode::VALUES,
        "whitespace-space" | "whitespace-tab" => WhitespaceRender::VALUES,
        // Newline is inherently always at end-of-line — no "trailing" axis.
        "whitespace-newline" => &["none", "all"],
        _ => return None,
    })
}

/// Phase 1: completing the scope token (`global`/`buffer`/`pane`).
fn complete_set_scope(prefix: &str, span_start: usize) -> CompletionResult {
    let mut candidates = prefix_completions(SET_SCOPES.iter().copied(), prefix);
    candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
    CompletionResult {
        span_start,
        candidates,
    }
}

/// Phase 2: completing the key. Surface every declared key whose scopes
/// include `scope`; `language` is the one key with no macro entry — valid
/// only for buffer, so it's chained in when the scope matches.
fn complete_set_key(scope: &str, rest: &str, span_start: usize) -> CompletionResult {
    let scope_keys = all_setting_keys()
        .iter()
        .copied()
        .filter(|k| setting_scopes(k).contains(&scope));
    let language = (scope == "buffer").then_some("language");
    let mut candidates = prefix_completions(scope_keys.chain(language), rest);
    candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
    CompletionResult {
        span_start,
        candidates,
    }
}

/// Phase 3: completing the value. Static enum/bool lists come from
/// [`static_value_candidates`]; `language` and `theme` are dynamic.
///
/// Every key checks its scope before offering values — the same gate
/// `typed_set` enforces at execution time — so e.g. `:set pane tab-style=`
/// (tab-style isn't pane-scoped) never dangles a completion that would error
/// on Enter.
fn complete_set_value(
    scope: &str,
    key: &str,
    value_prefix: &str,
    span_start: usize,
    ctx: &CompletionCtx<'_>,
) -> CompletionResult {
    // `language` has no `setting_scopes` entry by design (see settings.rs) —
    // valid only for buffer scope, checked directly instead of through the
    // generic gate below.
    let mut candidates = if key == "language" {
        if scope == "buffer" {
            prefix_completions(ctx.languages.iter_names(), value_prefix)
        } else {
            Vec::new()
        }
    } else if !setting_scopes(key).contains(&scope) {
        Vec::new()
    } else if let Some(values) = static_value_candidates(key) {
        prefix_completions(values.iter().copied(), value_prefix)
    } else if key == "theme" {
        theme_name_candidates(value_prefix)
    } else {
        Vec::new()
    };
    candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
    CompletionResult {
        span_start,
        candidates,
    }
}

impl Completer for SetCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let up_to = &input[..cursor.min(input.len())];
        // Argument region begins after the command word ("set ").
        let Some(arg_start) = up_to.find(' ').map(|i| i + 1) else {
            return CompletionResult {
                span_start: up_to.len(),
                candidates: Vec::new(),
            };
        };
        let arg = up_to[arg_start..].trim_start();

        match arg.split_once(' ') {
            None => {
                // Scope token: bounded by whitespace only — no '=' can occur
                // yet, so the last space before the cursor is always correct,
                // robust to stray extra whitespace.
                let span_start = up_to.rfind(' ').map_or(0, |i| i + 1);
                complete_set_scope(arg, span_start)
            }
            Some((scope, rest)) => {
                let rest = rest.trim_start();
                match rest.split_once('=') {
                    None => {
                        // Key token: same reasoning as the scope case.
                        let span_start = up_to.rfind(' ').map_or(0, |i| i + 1);
                        complete_set_key(scope, rest, span_start)
                    }
                    Some((key, value)) => {
                        // Value token: bounded by '=' only, never by internal
                        // whitespace — a value can legitimately contain spaces
                        // (e.g. a theme filename stem like "my theme"), and
                        // replacing from the last *space* would drop
                        // everything before it instead of the whole value.
                        let span_start = up_to.rfind('=').map_or(0, |i| i + 1);
                        complete_set_value(scope, key, value, span_start, ctx)
                    }
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
            "global+pane key should appear"
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
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        langs.register_identity("ruby", &["rb"], &[], &[]).unwrap();
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
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        let ctx = ctx_with(&reg, &store, dir.path(), &langs);
        let result = SetCompleter.complete("set global language=", 21, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn set_completer_value_language_prefix_filters() {
        let (reg, store, dir) = make_ctx_parts();
        let mut langs = LanguageRegistry::new();
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        langs.register_identity("ruby", &["rb"], &[], &[]).unwrap();
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
        langs.register_identity("ru", &[], &[], &[]).unwrap();
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        let ctx = ctx_with(&reg, &store, dir.path(), &langs);
        let result = SetCompleter.complete("set buffer language=ru", 22, &ctx);
        let names = names_of(&result);
        assert!(names.contains(&"rust"));
        assert!(!names.contains(&"ru"));
    }
}
