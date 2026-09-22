//! Completion — registering a source, answering an invocation, reading
//! the ranked view, accept/dismiss, plus trigger-character registration.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{CompletionSourceTarget, MatchKind};
use crate::json::{json_to_steel, steel_to_json};
use crate::{Effect, SteelCtx};

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, chars_arg, int_arg, list_items, string_arg, symbol_enum_arg, usize_arg,
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

/// Decodes `#:target` (`'buffer`/`'minibuf`) — each target has exactly one
/// token rule (see `CompletionSourceTarget`'s doc), so there is no separate
/// `#:token` to decode.
fn target_arg(target: SteelVal) -> Result<CompletionSourceTarget, SteelErr> {
    let target = string_arg(target, "register-completion-source! #:target")?;
    match target.as_str() {
        "buffer" => Ok(CompletionSourceTarget::Buffer),
        "minibuf" => Ok(CompletionSourceTarget::Minibuf),
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

/// `(%register-completion-source! name proc target match priority resolve)`
/// — the `register-completion-source!` Scheme wrapper supplies `#:match`/
/// `#:priority`/`#:resolve`'s defaults; `#:target` has none (every source
/// states its choice explicitly). `proc` is called as `(proc id bid
/// prefix)` for a `'buffer` source, `(proc id input cursor)` for a
/// `'minibuf` one, and answers with `(completion-emit! id …)`. Queued as an
/// `Effect`, not applied here — see `Effect::RegisterCompletionSource`.
///
/// `#:resolve #t` is refused outright on a `'minibuf` source: only a
/// `'buffer` source's items can ever be a wire `CompletionItem` from a
/// buffer's attached LSP server (`accept.rs`'s `maybe_send_resolve`, the
/// only reader of this flag, is itself `Buffer`-target only), so a
/// `'minibuf` source claiming it is a caller error, not a silently-ignored
/// no-op.
pub(crate) fn register_completion_source(
    ctx: &mut SteelCtx,
    name: String,
    proc: SteelVal,
    target: SteelVal,
    match_kind: SteelVal,
    priority: SteelVal,
    resolve: SteelVal,
) -> SteelResult {
    let proc = callable_arg(proc, "register-completion-source! proc")?;
    let target = target_arg(target)?;
    let match_kind = match_kind_arg(match_kind, "register-completion-source! #:match")?;
    let priority = int_arg(priority, "register-completion-source! #:priority")?;
    let resolve = bool_arg(resolve, "register-completion-source! #:resolve")?;
    if resolve && target != CompletionSourceTarget::Buffer {
        steel::stop!(Generic =>
            "register-completion-source! #:resolve: only a 'buffer source can set this");
    }
    ctx.push_effect(Effect::RegisterCompletionSource(
        crate::host::PendingCompletionSource {
            name,
            proc,
            target,
            match_kind,
            priority,
            resolve,
        },
    ));
    Ok(SteelVal::Void)
}

/// `(%completion-emit! id items incomplete)` — the `completion-emit!`
/// Scheme wrapper supplies `#:incomplete`'s `#f` default. `items`: list of
/// decoded `CompletionItem` hashmaps. Returns whether the answer applied
/// (`#f` for a superseded or already-closed invocation).
pub(crate) fn completion_emit(
    ctx: &mut SteelCtx,
    id: SteelVal,
    items: SteelVal,
    incomplete: SteelVal,
) -> SteelResult {
    let id = usize_arg(id, "completion-emit! id")? as u64;
    let incomplete = bool_arg(incomplete, "completion-emit! #:incomplete")?;
    let mut parsed = Vec::new();
    for entry in list_items(items, "completion-emit! items")? {
        parsed.push(steel_to_json(&entry).map_err(generic_err)?);
    }
    let applied = require_cap(ctx.host.completions(), "completion-emit!")?
        .completion_emit(id, parsed, incomplete);
    Ok(SteelVal::BoolV(applied))
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
