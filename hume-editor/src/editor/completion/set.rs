use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceRender, WrapMode};

use super::{Completer, Completion, CompletionCtx, CompletionResult, theme_name_candidates};
use crate::settings::{
    LANGUAGE_KEY, SHOW_NEWLINE_VALUES, Scope, SignColumnConfig, THEME_KEY, WRAP_MODE_KEY,
    all_setting_keys, setting_scopes,
};
use hume_editing::tab_style::TabStyle;

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
///   candidates — the user types them and `write_global`/`write_buffer`
///   validates.
///
/// Value lists are completion *hints* mirrored from each setting's parser;
/// `write_global`/`write_buffer` remain the validation SSOT, so the two can
/// drift only in what's offered, never in what's accepted.
pub(crate) struct SetCompleter;

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
        WRAP_MODE_KEY => WrapMode::VALUES,
        "whitespace-space" | "whitespace-tab" => WhitespaceRender::VALUES,
        "whitespace-newline" => SHOW_NEWLINE_VALUES,
        "signcolumn" => SignColumnConfig::VALUES,
        "lsp.diagnostics-severity-floor" => crate::editor::lsp::diagnostics::DiagSeverity::VALUES,
        _ => return None,
    })
}

/// Phase 1: completing the scope token (`global`/`buffer`/`pane`).
fn complete_set_scope(prefix: &str, span_start: usize) -> CompletionResult {
    let candidates = prefix_completions(Scope::ALL.iter().map(|s| s.as_str()), prefix);
    CompletionResult::sorted(span_start, candidates)
}

/// Phase 2: completing the key. Surface every declared key whose scopes
/// include `scope`; `language` is the one key with no macro entry — valid
/// only for buffer, so it's chained in when the scope matches. An unparseable
/// `scope` token (mid-typing garbage) yields no candidates, same as any real
/// key that doesn't accept it.
fn complete_set_key(scope: &str, rest: &str, span_start: usize) -> CompletionResult {
    let Ok(scope) = scope.parse::<Scope>() else {
        return CompletionResult::sorted(span_start, Vec::new());
    };
    let scope_keys = all_setting_keys()
        .iter()
        .copied()
        .filter(|k| setting_scopes(k).contains(&scope));
    let language = (scope == Scope::Buffer).then_some(LANGUAGE_KEY);
    let candidates = prefix_completions(scope_keys.chain(language), rest);
    CompletionResult::sorted(span_start, candidates)
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
    // generic gate below. An unparseable `scope` token falls through both
    // branches to the same empty result as a real key rejecting that scope.
    let scope = scope.parse::<Scope>().ok();
    let candidates = if key == LANGUAGE_KEY {
        if scope == Some(Scope::Buffer) {
            prefix_completions(ctx.languages.iter_names(), value_prefix)
        } else {
            Vec::new()
        }
    } else if !scope.is_some_and(|s| setting_scopes(key).contains(&s)) {
        Vec::new()
    } else if let Some(values) = static_value_candidates(key) {
        prefix_completions(values.iter().copied(), value_prefix)
    } else if key == THEME_KEY {
        theme_name_candidates(value_prefix)
    } else {
        Vec::new()
    };
    CompletionResult::sorted(span_start, candidates)
}

impl Completer for SetCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let up_to = &input[..cursor.min(input.len())];
        // Argument region begins after the command word ("set ").
        let Some(arg_start) = up_to.find(' ').map(|i| i + 1) else {
            return CompletionResult::sorted(up_to.len(), Vec::new());
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
mod tests;
