//! Completion orchestration — session lifecycle for a single in-progress
//! completion, plus trigger-character registration that feeds it.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::json::{json_to_steel, steel_to_json};

use super::args::{BidArg, bool_arg, chars_arg, list_items, string_arg, usize_arg};
use super::errors::{generic_err, require_cap};

type SteelResult = Result<SteelVal, SteelErr>;

/// `(register-trigger-chars! source language chars)` — `chars` is a list of
/// 1-char strings, registered for exactly `(source, language)`. Callable
/// from any context, including command bodies and hook handlers —
/// completion/signature-help register a server's trigger characters from
/// inside an `on-lsp-attach` handler, which runs as plain command context
/// (no `EvalMode` gate applies here, unlike `register-hook!` /
/// `on-lsp-notification`).
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

/// `(%completion-begin! bid items incomplete)` — the `completion-begin!`
/// Scheme wrapper supplies `#:incomplete`'s default. `items`: list of
/// decoded `CompletionItem` hashmaps.
pub(crate) fn completion_begin(
    ctx: &mut SteelCtx,
    bid: BidArg,
    items: SteelVal,
    incomplete: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let incomplete = bool_arg(incomplete, "completion-begin! #:incomplete")?;
    let mut parsed = Vec::new();
    for entry in list_items(items, "completion-begin! items")? {
        parsed.push(steel_to_json(&entry).map_err(generic_err)?);
    }
    require_cap(ctx.host.completions(), "completion-begin!")?
        .completion_begin(id, parsed, incomplete)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(completion-update-filter! text)`.
pub(crate) fn completion_update_filter(ctx: &mut SteelCtx, text: SteelVal) -> SteelResult {
    let text = string_arg(text, "completion-update-filter! text")?;
    require_cap(ctx.host.completions(), "completion-update-filter!")?
        .completion_update_filter(text)
        .map(|()| SteelVal::Void)
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
    if let Some(completions) = ctx.host.completions() {
        completions.completion_dismiss();
    }
    Ok(SteelVal::Void)
}
