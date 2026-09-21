//! Completion — registering a source, answering an invocation, reading
//! the ranked view, accept/dismiss, plus trigger-character registration.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{BufferToken, CompletionSourceTarget, MatchKind, MinibufToken};
use crate::json::{json_to_steel, steel_to_json};
use crate::{Effect, SteelCtx};

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, chars_arg, int_arg, list_items, optional_pair_fields, string_arg,
    symbol_enum_arg, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// Decodes `#:match` (`'fuzzy`/`'string`/`'delegated`) — a `String` source
/// is case-sensitive by default; no Steel caller needs case-insensitive
/// matching yet, so there's no second keyword for it (the native minibuffer
/// sources set this directly in Rust, bypassing this decode entirely).
fn match_kind_arg(val: SteelVal, ctx_name: &str) -> Result<MatchKind, SteelErr> {
    symbol_enum_arg(
        string_arg(val, ctx_name)?.as_str(),
        ctx_name,
        &[
            ("fuzzy", MatchKind::Fuzzy),
            (
                "string",
                MatchKind::String {
                    case_sensitive: true,
                },
            ),
            ("delegated", MatchKind::Delegated),
        ],
    )
}

/// Decodes the `#:target`/`#:token` pair as one value — the token
/// vocabulary depends on the target (`'word`/`'cursor`/`'custom` for a
/// buffer, `'arg`/`'custom` for the minibuffer), so a token from the wrong
/// target's vocabulary is rejected here, by name.
fn target_arg(target: SteelVal, token: SteelVal) -> Result<CompletionSourceTarget, SteelErr> {
    let target = string_arg(target, "register-completion-source! #:target")?;
    let token = string_arg(token, "register-completion-source! #:token")?;
    match target.as_str() {
        "buffer" => symbol_enum_arg(
            &token,
            "register-completion-source! #:token (for #:target 'buffer)",
            &[
                ("word", BufferToken::Word),
                ("cursor", BufferToken::Cursor),
                ("custom", BufferToken::Custom),
            ],
        )
        .map(CompletionSourceTarget::Buffer),
        "minibuf" => symbol_enum_arg(
            &token,
            "register-completion-source! #:token (for #:target 'minibuf)",
            &[("arg", MinibufToken::Arg), ("custom", MinibufToken::Custom)],
        )
        .map(CompletionSourceTarget::Minibuf),
        other => steel::stop!(Generic =>
            "register-completion-source! #:target: expected 'buffer or 'minibuf, got '{}", other),
    }
}

/// `(register-trigger-chars! source language chars)` — `chars` is a list of
/// 1-char strings, registered for exactly `(source, language)`. Callable
/// from any context, including command bodies and hook handlers —
/// completion/signature-help register a server's trigger characters from
/// inside an `on-lsp-attach` handler, which runs as plain command context
/// (no `EvalMode` gate applies here, unlike `register-hook!` /
/// `on-lsp-notification`). A completion source registered under `source`'s
/// name is invoked directly when one of `chars` lands in Insert mode.
pub(crate) fn register_trigger_chars(
    ctx: &mut SteelCtx,
    source: SteelVal,
    language: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "register-trigger-chars! source")?;
    let language = string_arg(language, "register-trigger-chars! language")?;
    let chars = chars_arg(chars, "register-trigger-chars! chars")?;
    ctx.host
        .language()
        .register_trigger_chars(source, language, chars);
    Ok(SteelVal::Void)
}

/// `(%register-completion-source! name proc target token match priority)`
/// — the `register-completion-source!` Scheme wrapper supplies `#:match`/
/// `#:priority`'s defaults; `#:target`/`#:token` have none (every source
/// states its choice explicitly). `proc` is called as `(proc id bid
/// prefix)` for a `'buffer` source, `(proc id input cursor)` for a
/// `'minibuf` one, and answers with `(completion-emit! id …)`. Queued as an
/// `Effect`, not applied here — see `Effect::RegisterCompletionSource`.
pub(crate) fn register_completion_source(
    ctx: &mut SteelCtx,
    name: String,
    proc: SteelVal,
    target: SteelVal,
    token: SteelVal,
    match_kind: SteelVal,
    priority: SteelVal,
) -> SteelResult {
    let proc = callable_arg(proc, "register-completion-source! proc")?;
    let target = target_arg(target, token)?;
    let match_kind = match_kind_arg(match_kind, "register-completion-source! #:match")?;
    let priority = int_arg(priority, "register-completion-source! #:priority")?;
    ctx.push_effect(Effect::RegisterCompletionSource(
        crate::host::PendingCompletionSource {
            name,
            proc,
            target,
            match_kind,
            priority,
        },
    ));
    Ok(SteelVal::Void)
}

/// `(%completion-emit! id items incomplete span)` — the `completion-emit!`
/// Scheme wrapper supplies `#:incomplete`/`#:span`'s `#f` defaults.
/// `items`: list of decoded `CompletionItem` hashmaps. Returns whether the
/// answer applied (`#f` for a superseded or already-closed invocation).
pub(crate) fn completion_emit(
    ctx: &mut SteelCtx,
    id: SteelVal,
    items: SteelVal,
    incomplete: SteelVal,
    span: SteelVal,
) -> SteelResult {
    let id = usize_arg(id, "completion-emit! id")? as u64;
    let incomplete = bool_arg(incomplete, "completion-emit! #:incomplete")?;
    let span = optional_pair_fields(span, "completion-emit! #:span", "(start . end)")?
        .map(|(start, end)| -> Result<(usize, usize), SteelErr> {
            Ok((
                usize_arg(start, "completion-emit! #:span start")?,
                usize_arg(end, "completion-emit! #:span end")?,
            ))
        })
        .transpose()?;
    let mut parsed = Vec::new();
    for entry in list_items(items, "completion-emit! items")? {
        parsed.push(steel_to_json(&entry).map_err(generic_err)?);
    }
    require_cap(ctx.host.completions(), "completion-emit!")?
        .completion_emit(id, parsed, incomplete, span)
        .map(SteelVal::BoolV)
        .map_err(generic_err)
}

/// `(completion-top n)`.
pub(crate) fn completion_top(ctx: &mut SteelCtx, n: SteelVal) -> SteelResult {
    let n = usize_arg(n, "completion-top")?;
    let items = ctx
        .host
        .completions()
        .map(|c| c.completion_top(n))
        .unwrap_or_default();
    let list: Vec<SteelVal> = items.iter().map(json_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `(completion-accept! idx)` — `idx` indexes the ranked/filtered list
/// (`completion-top`'s order), not the raw response order.
pub(crate) fn completion_accept(ctx: &mut SteelCtx, idx: SteelVal) -> SteelResult {
    let idx = usize_arg(idx, "completion-accept!")?;
    require_cap(ctx.host.completions(), "completion-accept!")?
        .completion_accept(idx)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(completion-dismiss!)`.
pub(crate) fn completion_dismiss(ctx: &mut SteelCtx) -> SteelResult {
    require_cap(ctx.host.completions(), "completion-dismiss!")?
        .completion_dismiss()
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;
