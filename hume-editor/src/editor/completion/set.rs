use std::ops::Range;

use super::{CompletionCtx, CompletionItem, arg_prefix, arg_span, theme_name_candidates};
use crate::editor::settings::{Scope, SettingKey, SettingText, THEME, TableKey};
use hume_core::BufferIntrinsicOption;
use hume_editing::text::LineEnding;

// ── :set arguments ────────────────────────────────────────────────────────────

pub(in crate::editor) const SET_SOURCE: &str = "set";

pub(in crate::editor) const GET_SOURCE: &str = "get";

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

/// Completes the scope token (`global`/`buffer`/`pane`).
fn complete_set_scope(prefix: &str) -> Vec<CompletionItem> {
    prefix_completions(Scope::ALL.iter().map(|s| s.as_str()), prefix)
}

/// Completes the key: every setting, the buffer-intrinsic ones included,
/// that takes `scope`.
fn complete_set_key(scope: Scope, rest: &str) -> Vec<CompletionItem> {
    let keys = SettingKey::all()
        .filter(|setting| setting.target(scope).is_some())
        .map(SettingKey::name);
    prefix_completions(keys, rest)
}

/// Completes the value. A settings-table key offers
/// [`TableKey::value_names`]; `language` and `theme` are dynamic.
///
/// Every key checks its scope before offering values, by the rule `:set`
/// enforces, so e.g. `:set pane indent-style=` (indent-style isn't pane-scoped) never
/// dangles a completion that would error on Enter.
fn complete_set_value(
    scope: Scope,
    key: &str,
    value_prefix: &str,
    ctx: &CompletionCtx<'_>,
) -> Vec<CompletionItem> {
    let Some(setting) = SettingKey::named(key).filter(|setting| setting.target(scope).is_some())
    else {
        return Vec::new();
    };
    match setting {
        SettingKey::Intrinsic(BufferIntrinsicOption::Language) => {
            prefix_completions(ctx.languages.iter_names(), value_prefix)
        }
        SettingKey::Intrinsic(BufferIntrinsicOption::LineEnding) => {
            prefix_completions(LineEnding::VALUES.iter().copied(), value_prefix)
        }
        SettingKey::Table(TableKey::Global(THEME)) => prefix_completions(
            theme_name_candidates(ctx.dirs).iter().map(String::as_str),
            value_prefix,
        ),
        SettingKey::Table(key) => {
            prefix_completions(key.value_names().iter().copied(), value_prefix)
        }
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
/// - **key** (space present, no `=` yet): offers every setting key that
///   takes the chosen scope.
/// - **value** (`=` present): offers the valid value set for enum/bool keys,
///   registered language names for `language`, installed theme names for
///   `theme`. Numeric/free-form keys (e.g. `scroll-margin`, `statusline`) get no
///   candidates: the user types them and `write_global`/`write_buffer`
///   validates.
///
/// A settings-table key's value list comes from the parser kind its
/// `define_settings!` entry declares, the same kind `write_global`/
/// `write_buffer` validate with.
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
            // `None` for mid-typing garbage, which takes no key or value.
            let scope = Scope::named(scope);
            let rest = rest.trim_start();
            match rest.split_once('=') {
                None => {
                    // Key token: same start reasoning as the scope case.
                    // The forward scan must also stop at `'='` (not just
                    // whitespace), or completing `:set global th|eme=x`
                    // would swallow the `=x` into the replaced span instead
                    // of leaving it after the completed key.
                    let span = arg_span(input, cursor, ' ', &[' ', '=']);
                    let candidates =
                        scope.map_or_else(Vec::new, |scope| complete_set_key(scope, rest));
                    (span, candidates)
                }
                Some((key, value)) => {
                    // Value token: bounded by '=' on the start, whitespace
                    // only on the end: a value can legitimately contain an
                    // internal '=' or spaces (e.g. a `statusline` format
                    // string), so only whitespace ends it.
                    let span = arg_span(input, cursor, '=', &[' ']);
                    let candidates = scope
                        .map_or_else(Vec::new, |scope| complete_set_value(scope, key, value, ctx));
                    (span, candidates)
                }
            }
        }
    }
}

/// Completes `:get <scope> <key>` arguments: [`complete_set`]'s scope and
/// key phases. `:get` takes no value, so text after a `=` gets no candidates.
pub(super) fn complete_get(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    if arg_prefix(input, cursor).1.contains('=') {
        let at = cursor.min(input.len());
        return (at..at, Vec::new());
    }
    complete_set(input, cursor, ctx)
}

#[cfg(test)]
mod tests;
