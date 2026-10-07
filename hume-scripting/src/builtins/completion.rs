//! Completion: registering a source, answering an invocation, reading
//! the ranked view, accept/dismiss, plus trigger-character registration.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{CompletionSourceTarget, MatchKind};
use crate::json::json_to_steel;
use crate::types::{PaneHandle, TriggerKind, TriggerScope};
use crate::{Effect, SteelCtx};

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, chars_arg, int_arg, json_arg, list_items, server_arg, string_arg,
    symbol_enum_arg, usize_arg,
};
use super::errors::{generic_err, require_cap};
use super::lsp::required_feature_arg;

/// Decodes `#:match` (`'fuzzy`/`'string`/`'delegated`). A `String` source
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

/// Decodes `#:target` (`'buffer`/`'minibuf`). Each target has exactly one
/// token rule (see `CompletionSourceTarget`'s doc), so there is no separate
/// `#:token` to decode.
fn target_arg(target: SteelVal) -> Result<CompletionSourceTarget, SteelErr> {
    let ctx_name = "register-completion-source! #:target";
    symbol_enum_arg(
        string_arg(target, ctx_name)?.as_str(),
        ctx_name,
        &[
            ("buffer", CompletionSourceTarget::Buffer),
            ("minibuf", CompletionSourceTarget::Minibuf),
        ],
    )
}

/// `(set-hook-triggers! source language chars)`: `chars` is a list of 1-char
/// strings, registered for `(source, language)`. `chars` landing in Insert
/// mode fires the `on-trigger-char` hook for any listener named `source`, a
/// shared, listener-agnostic table, *not* how a completion source's own
/// trigger chars are joined (that's `set-completion-triggers!`, a
/// completion source's own routing table, checked before invoking sources
/// directly).
pub(crate) fn set_hook_triggers(
    ctx: &mut SteelCtx,
    source: SteelVal,
    language: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let name = "set-hook-triggers!";
    let scope = TriggerScope::Language(string_arg(language, &format!("{name} language"))?);
    queue_triggers(ctx, name, TriggerKind::Hook, source, scope, chars)
}

/// `(set-attachment-hook-triggers! source pane server feature chars)`: the
/// same table for the one attachment of `pane`'s buffer to `server`, for
/// `feature`. Callable
/// from any context, including command bodies and hook handlers: signature
/// help sets its trigger characters from inside an `on-lsp-attach` handler,
/// which runs as plain command context (no `EvalMode` gate applies here,
/// unlike `register-hook!`). Nothing is set when this applies unless the
/// buffer is attached to `server`, the attachment's list entry admits
/// `feature` and `server` advertises it.
pub(crate) fn set_attachment_hook_triggers(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    server: SteelVal,
    feature: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let name = "set-attachment-hook-triggers!";
    let scope = attachment_scope(pane, server, feature, name)?;
    queue_triggers(ctx, name, TriggerKind::Hook, source, scope, chars)
}

fn attachment_scope(
    pane: PaneHandle,
    server: SteelVal,
    feature: SteelVal,
    name: &str,
) -> Result<TriggerScope, SteelErr> {
    Ok(TriggerScope::Attachment {
        buffer: pane.buffer(),
        server: server_arg(&server, &format!("{name} server"))?.id,
        feature: required_feature_arg(feature, &format!("{name} feature"))?,
    })
}

/// The one decode behind every trigger-character builtin: queued as an
/// `Effect`, not applied here; see `Effect::SetTriggers`. Argument decoding
/// (a well-formed `chars` list) fails synchronously here.
fn queue_triggers(
    ctx: &mut SteelCtx,
    name: &str,
    kind: TriggerKind,
    source: SteelVal,
    scope: TriggerScope,
    chars: SteelVal,
) -> SteelResult {
    let source = string_arg(source, &format!("{name} source"))?;
    let chars = chars_arg(chars, &format!("{name} chars"))?;
    ctx.push_effect(Effect::SetTriggers {
        kind,
        source,
        scope,
        chars,
    });
    Ok(SteelVal::Void)
}

/// `(%register-completion-source! name proc target match priority resolve
/// token-chars)`:
/// the `register-completion-source!` Scheme wrapper supplies `#:match`/
/// `#:priority`/`#:resolve`'s defaults; `#:target` has none (every source
/// states its choice explicitly). `proc` is called as `(proc id bid
/// prefix)` for a `'buffer` source, `(proc id input cursor)` for a
/// `'minibuf` one, and answers with `(completion-emit! id …)`. Queued as an
/// `Effect`, not applied here; see `Effect::RegisterCompletionSource`.
///
/// `#:resolve #t` is refused outright on a `'minibuf` source: only a
/// `'buffer` source's items can ever be a wire `CompletionItem` from a
/// buffer's attached LSP server (`accept.rs`'s `maybe_send_resolve`, the
/// only reader of this flag, is itself `Buffer`-target only), so a
/// `'minibuf` source claiming it is a caller error, not a silently-ignored
/// no-op. `#:token-chars` is refused there too: a `'minibuf` source's token
/// is its own argument span.
// Each param is a positional arg the `builtins!` table maps 1:1 from the
// `%register-completion-source!` wrapper call.
#[allow(clippy::too_many_arguments)]
pub(crate) fn register_completion_source(
    ctx: &mut SteelCtx,
    name: String,
    proc: SteelVal,
    target: SteelVal,
    match_kind: SteelVal,
    priority: SteelVal,
    resolve: SteelVal,
    token_chars: SteelVal,
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
    let token_chars = string_arg(token_chars, "register-completion-source! #:token-chars")?;
    if !token_chars.is_empty() && target != CompletionSourceTarget::Buffer {
        steel::stop!(Generic =>
            "register-completion-source! #:token-chars: only a 'buffer source can set this");
    }
    hume_editing::word::WordChars::validate(&token_chars)
        .map_err(|e| generic_err(format!("register-completion-source! #:token-chars: {e}")))?;
    ctx.push_effect(Effect::RegisterCompletionSource(
        crate::host::PendingCompletionSource {
            name,
            proc,
            target,
            match_kind,
            priority,
            resolve,
            token_chars,
        },
    ));
    Ok(SteelVal::Void)
}

/// `(%completion-emit! id items incomplete)`: the `completion-emit!`
/// Scheme wrapper supplies `#:incomplete`'s `#f` default. `items` is a list
/// whose elements are `CompletionItem` hashmaps, bare-string labels, or
/// whole LSP `textDocument/completion` responses passed straight through;
/// each crosses `json_arg` (the funnel every JSON-taking builtin uses) with
/// no deep copy. What each element is only matters to the host
/// implementation, which decodes them. Returns whether the answer applied
/// (`#f` for a superseded or already-closed invocation).
pub(crate) fn completion_emit(
    ctx: &mut SteelCtx,
    id: SteelVal,
    items: SteelVal,
    incomplete: SteelVal,
) -> SteelResult {
    let id = usize_arg(id, "completion-emit! id")? as u64;
    let incomplete = bool_arg(incomplete, "completion-emit! #:incomplete")?;
    let parts = list_items(items, "completion-emit! items")?
        .into_iter()
        .map(|item| json_arg(item, "completion-emit! items"))
        .collect::<Result<Vec<_>, _>>()?;
    let applied = require_cap(ctx.host.completions(), "completion-emit!")?
        .completion_emit(id, parts, incomplete)
        .map_err(generic_err)?;
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

/// `(completion-accept! idx)`: `idx` indexes the ranked/filtered list
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

/// `(set-completion-triggers! source language chars)`: a `'buffer`
/// completion source's own trigger characters for `language`, replacing
/// that pair's previous set.
/// Callable from any context, same as `set-hook-triggers!`.
///
/// Whether `source` names a registered `Buffer` source can only be checked
/// once the effect applies (an earlier *queued*
/// `register-completion-source!` in the same eval may be the one supplying
/// it): a miss is reported as a log message there, never raised back to the
/// caller.
pub(crate) fn set_completion_triggers(
    ctx: &mut SteelCtx,
    source: SteelVal,
    language: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let name = "set-completion-triggers!";
    let scope = TriggerScope::Language(string_arg(language, &format!("{name} language"))?);
    queue_triggers(ctx, name, TriggerKind::Completion, source, scope, chars)
}

/// `(set-attachment-completion-triggers! source pane server feature chars)`:
/// the same for the one attachment of `pane`'s buffer to `server`, as
/// `set-attachment-hook-triggers!` is to `set-hook-triggers!`.
pub(crate) fn set_attachment_completion_triggers(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    server: SteelVal,
    feature: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let name = "set-attachment-completion-triggers!";
    let scope = attachment_scope(pane, server, feature, name)?;
    queue_triggers(ctx, name, TriggerKind::Completion, source, scope, chars)
}

#[cfg(test)]
mod tests;
