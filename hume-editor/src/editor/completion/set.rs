use std::ops::Range;

use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceRender, WrapMode};

use super::{CompletionCtx, CompletionItem, arg_prefix, arg_span, theme_name_candidates};
use crate::editor::settings::{
    SHOW_NEWLINE_VALUES, Scope, SettingText, SignColumnConfig, THEME_KEY, WRAP_MODE_KEY,
    all_setting_keys, setting_scopes,
};
use hume_editing::tab_style::TabStyle;
use hume_editing::text::LineEnding;
use hume_scripting::host::BufferIntrinsicOption;

// ── :set arguments ────────────────────────────────────────────────────────────

pub(in crate::editor) const SET_SOURCE: &str = "set";

/// Prefix-filter `items` and wrap each into a `CompletionItem`, sorted
/// alphabetically. A `Delegated` source's own order is what the session's
/// rank key preserves (see `MatchKind::Delegated`'s doc), so the order is
/// established right here rather than left to a tiebreak. A fully-typed
/// value is filtered out later, by `SlotSet::rank_with`'s own no-op
/// check, not here.
fn prefix_completions<'a>(
    items: impl Iterator<Item = &'a str>,
    prefix: &str,
) -> Vec<CompletionItem> {
    let mut candidates: Vec<CompletionItem> = items
        .filter(|s| s.starts_with(prefix))
        .map(|s| CompletionItem::plain(s.to_owned(), s.to_owned()))
        .collect();
    candidates.sort_unstable_by(|a, b| a.label.cmp(&b.label));
    candidates
}

/// Static value candidates for enum/bool keys. Returns `None` for keys whose
/// values are dynamic (`language`, `theme`) or free-form (numbers,
/// `statusline`); those are handled in [`complete_set`].
fn static_value_candidates(key: &str) -> Option<&'static [&'static str]> {
    // Bool keys are derived from `define_settings!`'s `parser: bool` (not
    // hand-listed), so a new bool setting gets value completion for free.
    if crate::editor::settings::is_bool_setting(key) {
        return Some(&["true", "false"]);
    }
    Some(match key {
        "tab-style" => TabStyle::VALUES,
        "line-number-style" => LineNumberStyle::VALUES,
        "object-jump-align" => crate::editor::settings::ObjectJumpAlign::VALUES,
        "cursor-shape-insert" => crate::editor::settings::CursorShape::VALUES,
        WRAP_MODE_KEY => WrapMode::VALUES,
        "whitespace-space" | "whitespace-tab" => WhitespaceRender::VALUES,
        "whitespace-newline" => SHOW_NEWLINE_VALUES,
        "signcolumn" => SignColumnConfig::VALUES,
        "tabline" => crate::editor::settings::TablineVisibility::VALUES,
        "command-completion" => crate::editor::settings::CommandCompletion::VALUES,
        "lsp.diagnostics-severity-floor" => crate::editor::lsp::diagnostics::DiagSeverity::VALUES,
        _ => return None,
    })
}

/// Completes the scope token (`global`/`buffer`/`pane`).
fn complete_set_scope(prefix: &str) -> Vec<CompletionItem> {
    prefix_completions(Scope::ALL.iter().map(|s| s.as_str()), prefix)
}

/// Completes the key. Surfaces every declared key whose scopes
/// include `scope`; the buffer-intrinsic keys have no macro entry and are
/// valid only for buffer, so they're chained in when the scope matches. An
/// unparseable `scope` token (mid-typing garbage) yields no candidates, same
/// as any real key that doesn't accept it.
fn complete_set_key(scope: &str, rest: &str) -> Vec<CompletionItem> {
    let Ok(scope) = scope.parse::<Scope>() else {
        return Vec::new();
    };
    let scope_keys = all_setting_keys()
        .iter()
        .copied()
        .filter(|k| setting_scopes(k).contains(&scope));
    let intrinsic = (scope == Scope::Buffer)
        .then_some(BufferIntrinsicOption::ALL)
        .into_iter()
        .flatten()
        .map(|opt| opt.key());
    prefix_completions(scope_keys.chain(intrinsic), rest)
}

/// Completes the value. Static enum/bool lists come from
/// [`static_value_candidates`]; `language` and `theme` are dynamic.
///
/// Every key checks its scope before offering values (the same gate
/// `typed_set` enforces at execution time), so e.g. `:set pane tab-style=`
/// (tab-style isn't pane-scoped) never dangles a completion that would error
/// on Enter.
fn complete_set_value(
    scope: &str,
    key: &str,
    value_prefix: &str,
    ctx: &CompletionCtx<'_>,
) -> Vec<CompletionItem> {
    // Buffer-intrinsic options have no `setting_scopes` entry by design (see
    // settings.rs): valid only for buffer scope, checked directly instead of
    // through the generic gate below. An unparseable `scope` token falls
    // through to the same empty result as a real key rejecting that scope.
    let scope = scope.parse::<Scope>().ok();
    if let Some(opt) = BufferIntrinsicOption::from_key(key) {
        if scope != Some(Scope::Buffer) {
            return Vec::new();
        }
        match opt {
            BufferIntrinsicOption::Language => {
                prefix_completions(ctx.languages.iter_names(), value_prefix)
            }
            BufferIntrinsicOption::LineEnding => {
                prefix_completions(LineEnding::VALUES.iter().copied(), value_prefix)
            }
        }
    } else if !scope.is_some_and(|s| setting_scopes(key).contains(&s)) {
        Vec::new()
    } else if let Some(values) = static_value_candidates(key) {
        prefix_completions(values.iter().copied(), value_prefix)
    } else if key == THEME_KEY {
        prefix_completions(
            theme_name_candidates(ctx.dirs).iter().map(String::as_str),
            value_prefix,
        )
    } else {
        Vec::new()
    }
}

/// Completes `:set <scope> <key>=<value>` arguments. `Delegated`: the
/// candidate universe changes shape at each phase boundary
/// (scope/key/value), so this takes the live input directly rather than
/// enumerating a stable universe for the session to filter: same
/// invocation contract as `complete_path`.
///
/// Three phases, selected by cursor position within the argument:
/// - **scope** (no space yet): offers `global`/`buffer`/`pane`.
/// - **key** (space present, no `=` yet): offers every setting key whose
///   declared scopes include the chosen scope, plus `language` for `buffer`.
/// - **value** (`=` present): offers the valid value set for enum/bool keys,
///   registered language names for `language`, installed theme names for
///   `theme`. Numeric/free-form keys (e.g. `scroll-margin`, `statusline`) get no
///   candidates: the user types them and `write_global`/`write_buffer`
///   validates.
///
/// Value lists are completion *hints* mirrored from each setting's parser;
/// `write_global`/`write_buffer` remain the validation SSOT, so the two can
/// drift only in what's offered, never in what's accepted.
pub(super) fn complete_set(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    // Argument region begins after the command word ("set "). `arg_start ==
    // 0` means `arg_prefix` found no space yet, still typing "set" itself,
    // not its argument. Unreachable via `resolve_minibuf_source` (which
    // only invokes this completer once the cursor is past the command
    // name, which requires a space), kept for callers that hand this
    // function malformed input directly.
    let (arg_start, arg) = arg_prefix(input, cursor);
    if arg_start == 0 {
        let at = cursor.min(input.len());
        return (at..at, Vec::new());
    }
    let arg = arg.trim_start();

    match arg.split_once(' ') {
        None => {
            // Scope token: bounded by whitespace only. No '=' can occur
            // yet, so the last space before the cursor is always correct,
            // robust to stray extra whitespace. `'='` is an extra stop on
            // the forward side purely for symmetry with the key phase below.
            // A scope name never contains one, so it never fires here.
            let span = arg_span(input, cursor, ' ', &[' ', '=']);
            let candidates = complete_set_scope(arg);
            (span, candidates)
        }
        Some((scope, rest)) => {
            let rest = rest.trim_start();
            match rest.split_once('=') {
                None => {
                    // Key token: same start reasoning as the scope case.
                    // The forward scan must also stop at `'='` (not just
                    // whitespace), or completing `:set global th|eme=x`
                    // would swallow the `=x` into the replaced span instead
                    // of leaving it after the completed key.
                    let span = arg_span(input, cursor, ' ', &[' ', '=']);
                    let candidates = complete_set_key(scope, rest);
                    (span, candidates)
                }
                Some((key, value)) => {
                    // Value token: bounded by '=' on the start, whitespace
                    // only on the end: a value can legitimately contain an
                    // internal '=' or spaces (e.g. a `statusline` format
                    // string), so only whitespace ends it.
                    let span = arg_span(input, cursor, '=', &[' ']);
                    let candidates = complete_set_value(scope, key, value, ctx);
                    (span, candidates)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
