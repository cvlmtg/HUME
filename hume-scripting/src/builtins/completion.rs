//! Completion: registering a source, answering an invocation, reading
//! the ranked view, accept/dismiss, plus trigger-character registration.

use std::sync::Arc;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{CompletionSourceTarget, EmittedWord};
use crate::json::json_to_steel;
use crate::types::TriggerScope;
use crate::{Callback, Effect, SteelCtx};
use hume_core::{AnswerEnd, BufferId, FirstTab, MatchKind, NamedPrefix, TriggerListener};

use super::SteelResult;
use super::args::{
    CHAR_RANGE_KEYS, bool_arg, chars_arg, int_arg, json_arg, list_items, optional_callback_arg,
    optional_hash_entry, optional_string_arg, server_arg, string_arg, symbol_enum_arg, token_arg,
    usize_arg,
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
    let ctx_name = "#:target";
    symbol_enum_arg(
        string_arg(target, ctx_name)?.as_str(),
        ctx_name,
        &CompletionSourceTarget::NAMED,
    )
}

/// `(set-hook-triggers! language source chars)`: `chars` is a list of chars,
/// registered for `(source, language)`. `chars` landing in Insert
/// mode fires the `on-trigger-char` hook for any listener named `source`.
/// `set-completion-triggers!` writes the same table for a completion
/// source, whose listener invokes the source instead of the hook.
pub(crate) fn set_hook_triggers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    source: String,
    chars: SteelVal,
) -> SteelResult {
    let scope = TriggerScope::Language(string_arg(language, "language")?);
    queue_triggers(ctx, TriggerListener::Hook, source, scope, chars)
}

/// `(set-attachment-hook-triggers! pane source server feature chars)`: the
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
    bid: BufferId,
    source: String,
    server: SteelVal,
    feature: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let scope = attachment_scope(bid, server, feature)?;
    queue_triggers(ctx, TriggerListener::Hook, source, scope, chars)
}

fn attachment_scope(
    bid: BufferId,
    server: SteelVal,
    feature: SteelVal,
) -> Result<TriggerScope, SteelErr> {
    Ok(TriggerScope::Attachment {
        buffer: bid,
        server: server_arg(&server, "server")?.id,
        feature: required_feature_arg(feature, "feature")?,
    })
}

/// The one decode behind every trigger-character builtin: queued as an
/// `Effect`, not applied here; see `Effect::SetTriggers`. `listener` builds
/// the listener the decoded `source` names. Argument decoding (a well-formed
/// `chars` list) fails synchronously here.
fn queue_triggers(
    ctx: &mut SteelCtx,
    listener: fn(Arc<str>) -> TriggerListener,
    source: String,
    scope: TriggerScope,
    chars: SteelVal,
) -> SteelResult {
    let chars = chars_arg(chars, "chars")?;
    ctx.push_effect(Effect::SetTriggers {
        listener: listener(source.into()),
        scope,
        chars,
    });
    Ok(SteelVal::Void)
}

/// `(%register-completion-source! name proc target match priority resolve
/// token-chars cancel)`:
/// the `register-completion-source!` Scheme wrapper supplies `#:match`/
/// `#:priority`/`#:resolve`/`#:token-chars`/`#:cancel`'s defaults;
/// `#:target` has none (every source states its choice explicitly). `proc`
/// is called as `(proc id bid prefix)` for a `'buffer` source, `(proc id
/// input cursor)` for a `'minibuf` one, and answers with
/// `(completion-emit! id …)`. `cancel`, `#f` or a procedure, is called as
/// `(cancel id)` when request `id` ends before its final answer. Queued as
/// an `Effect`, not applied here; see `Effect::RegisterCompletionSource`.
///
/// `#:resolve #t` is refused outright on a `'minibuf` source: only a
/// `'buffer` source's items can ever be a wire `CompletionItem` from a
/// buffer's attached LSP server (`accept.rs`'s `maybe_send_resolve`, the
/// only reader of this flag, is itself `Buffer`-target only), so a
/// `'minibuf` source claiming it is a caller error, not a silently-ignored
/// no-op. `#:token-chars` is refused there too: a `'minibuf` source's token
/// is its own argument span. So is a nonzero `#:priority`: a `'minibuf`
/// session runs one source, so there is no other source to rank against.
// Each param is a positional arg the `builtins!` table maps 1:1 from the
// `%register-completion-source!` wrapper call.
#[allow(clippy::too_many_arguments)]
pub(crate) fn register_completion_source(
    ctx: &mut SteelCtx,
    name: String,
    proc: Callback,
    target: SteelVal,
    match_kind: SteelVal,
    priority: SteelVal,
    resolve: SteelVal,
    token_chars: SteelVal,
    cancel: SteelVal,
) -> SteelResult {
    let cancel = optional_callback_arg(ctx, cancel, "#:cancel")?;
    let target = target_arg(target)?;
    let match_kind = match_kind_arg(match_kind, "#:match")?;
    let priority = int_arg(priority, "#:priority")?;
    if priority != 0 && target != CompletionSourceTarget::Buffer {
        steel::stop!(Generic =>
            "#:priority: only a 'buffer source can set this");
    }
    let resolve = bool_arg(resolve, "#:resolve")?;
    if resolve && target != CompletionSourceTarget::Buffer {
        steel::stop!(Generic =>
            "#:resolve: only a 'buffer source can set this");
    }
    let token_chars = string_arg(token_chars, "#:token-chars")?;
    if !token_chars.is_empty() && target != CompletionSourceTarget::Buffer {
        steel::stop!(Generic =>
            "#:token-chars: only a 'buffer source can set this");
    }
    hume_editing::word::WordChars::validate(&token_chars)
        .map_err(|e| generic_err(format!("#:token-chars: {e}")))?;
    let target = match target {
        CompletionSourceTarget::Buffer => crate::host::PendingTarget::Buffer {
            priority,
            resolve,
            token_chars,
        },
        CompletionSourceTarget::Minibuf => crate::host::PendingTarget::Minibuf,
    };
    ctx.push_effect(Effect::RegisterCompletionSource(
        crate::host::PendingCompletionSource {
            name,
            proc,
            target,
            match_kind,
            cancel,
        },
    ));
    Ok(SteelVal::Void)
}

/// `(%completion-emit! id items incomplete more span prefix)`: the
/// `completion-emit!` Scheme wrapper supplies the defaults of
/// `#:incomplete`, of `#:more` ([`AnswerEnd::More`] when `#t`) and of
/// [`EmittedWord`]'s `#:span` (a char range hash,
/// or `#f`) and `#:prefix` (`#t` when left out, which is
/// [`FirstTab::Common`]; `#f` is [`FirstTab::Nothing`] and a non-empty
/// string [`FirstTab::Named`]). `items` is a list
/// whose elements are `CompletionItem` hashmaps, bare-string labels, or
/// whole LSP `textDocument/completion` responses passed straight through;
/// each crosses `json_arg` (the funnel every JSON-taking builtin uses) with
/// no deep copy. What each element is only matters to the host
/// implementation, which decodes them. Returns whether the answer applied
/// (`#f` for a superseded or already-closed invocation, or an `id` of `#f`).
pub(crate) fn completion_emit(
    ctx: &mut SteelCtx,
    id: SteelVal,
    items: SteelVal,
    incomplete: SteelVal,
    more: SteelVal,
    span: SteelVal,
    prefix: SteelVal,
) -> SteelResult {
    let id = token_arg(id, "id")?;
    let incomplete = bool_arg(incomplete, "#:incomplete")?;
    let end = if bool_arg(more, "#:more")? {
        AnswerEnd::More
    } else {
        AnswerEnd::Final
    };
    let span = optional_hash_entry(span, "#:span", CHAR_RANGE_KEYS)?
        .map(|range| range.char_range())
        .transpose()?
        .map(|(start, end)| start..end);
    let prefix = match prefix {
        SteelVal::BoolV(true) => FirstTab::Common,
        other => match optional_string_arg(other, "#:prefix")? {
            None => FirstTab::Nothing,
            Some(text) => FirstTab::Named(NamedPrefix::new(text).ok_or_else(|| {
                generic_err(
                    "#:prefix would blank the word; send #f to propose \
                     nothing, or leave #:prefix out to let the editor propose one",
                )
            })?),
        },
    };
    let parts = list_items(items, "items")?
        .into_iter()
        .map(|item| json_arg(item, "items"))
        .collect::<Result<Vec<_>, _>>()?;
    let applied = require_cap(ctx.host.completions())?
        .completion_emit(id, parts, incomplete, end, EmittedWord { span, prefix })
        .map_err(generic_err)?;
    Ok(SteelVal::BoolV(applied))
}

/// `(completion-top n)`.
pub(crate) fn completion_top(ctx: &mut SteelCtx, n: SteelVal) -> SteelResult {
    let n = usize_arg(n, "n")?;
    let items = require_cap(ctx.host.completions())?.completion_top(n);
    let list: Vec<SteelVal> = items.iter().map(json_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `(completion-accept! idx)`: `idx` indexes the ranked/filtered list
/// (`completion-top`'s order), not the raw response order.
pub(crate) fn completion_accept(ctx: &mut SteelCtx, idx: SteelVal) -> SteelResult {
    let idx = usize_arg(idx, "idx")?;
    require_cap(ctx.host.completions())?
        .completion_accept(idx)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(completion-dismiss!)`.
pub(crate) fn completion_dismiss(ctx: &mut SteelCtx) -> SteelResult {
    require_cap(ctx.host.completions())?
        .completion_dismiss()
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(set-completion-triggers! language source chars)`: a `'buffer`
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
    language: SteelVal,
    source: String,
    chars: SteelVal,
) -> SteelResult {
    let scope = TriggerScope::Language(string_arg(language, "language")?);
    queue_triggers(ctx, TriggerListener::Completion, source, scope, chars)
}

/// `(set-attachment-completion-triggers! pane source server feature chars)`:
/// the same for the one attachment of `pane`'s buffer to `server`, as
/// `set-attachment-hook-triggers!` is to `set-hook-triggers!`.
pub(crate) fn set_attachment_completion_triggers(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    server: SteelVal,
    feature: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let scope = attachment_scope(bid, server, feature)?;
    queue_triggers(ctx, TriggerListener::Completion, source, scope, chars)
}

#[cfg(test)]
mod tests;
