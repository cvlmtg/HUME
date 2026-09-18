//! Completion orchestration — session lifecycle for a single in-progress
//! completion, plus trigger-character registration that feeds it.

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{Interaction, MatchKind};
use crate::json::{json_to_steel, steel_to_json};

use super::SteelResult;
use super::args::{
    BidArg, bool_arg, chars_arg, int_arg, list_items, string_arg, symbol_enum_arg, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// Decodes `#:match` (`'fuzzy`/`'string`/`'delegated`) — a `String` source
/// is case-sensitive by default; no Steel caller needs case-insensitive
/// matching yet, so there's no second keyword for it (the native minibuffer
/// registry sets this directly in Rust, bypassing this decode entirely).
fn match_kind_arg(val: SteelVal, ctx_name: &str) -> Result<MatchKind, steel::rerrs::SteelErr> {
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

/// Decodes `#:interaction` (`'select`/`'cycle`).
fn interaction_arg(val: SteelVal, ctx_name: &str) -> Result<Interaction, steel::rerrs::SteelErr> {
    symbol_enum_arg(
        string_arg(val, ctx_name)?.as_str(),
        ctx_name,
        &[
            ("select", Interaction::SelectAccept),
            ("cycle", Interaction::CycleApply),
        ],
    )
}

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

/// `(%completion-begin! bid items incomplete source priority match
/// interaction)` — the `completion-begin!` Scheme wrapper supplies
/// `#:incomplete`/`#:priority`/`#:match`/`#:interaction`'s defaults;
/// `#:source` has none (see the wrapper's own doc). `items`: list of decoded
/// `CompletionItem` hashmaps. Returns the new session's token.
#[allow(clippy::too_many_arguments)]
pub(crate) fn completion_begin(
    ctx: &mut SteelCtx,
    bid: BidArg,
    items: SteelVal,
    incomplete: SteelVal,
    source: SteelVal,
    priority: SteelVal,
    match_kind: SteelVal,
    interaction: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let incomplete = bool_arg(incomplete, "completion-begin! #:incomplete")?;
    let source = string_arg(source, "completion-begin! #:source")?;
    let priority = int_arg(priority, "completion-begin! #:priority")?;
    let match_kind = match_kind_arg(match_kind, "completion-begin! #:match")?;
    let interaction = interaction_arg(interaction, "completion-begin! #:interaction")?;
    let mut parsed = Vec::new();
    for entry in list_items(items, "completion-begin! items")? {
        parsed.push(steel_to_json(&entry).map_err(generic_err)?);
    }
    require_cap(ctx.host.completions(), "completion-begin!")?
        .completion_begin(
            id,
            parsed,
            source,
            priority,
            match_kind,
            interaction,
            incomplete,
        )
        .map(|token| SteelVal::IntV(token as isize))
        .map_err(generic_err)
}

/// `(%completion-add-items! token items source priority match incomplete)`
/// — the `completion-add-items!` Scheme wrapper supplies `#:priority`/
/// `#:match`/`#:incomplete`'s defaults; `#:source` has none, same as
/// `%completion-begin!`. Returns whether the merge applied (`#f` on a stale
/// token). No `#:interaction` here — it's decided once, by whoever calls
/// `completion-begin!`.
pub(crate) fn completion_add_items(
    ctx: &mut SteelCtx,
    token: SteelVal,
    items: SteelVal,
    source: SteelVal,
    priority: SteelVal,
    match_kind: SteelVal,
    incomplete: SteelVal,
) -> SteelResult {
    let token = usize_arg(token, "completion-add-items! token")? as u64;
    let source = string_arg(source, "completion-add-items! #:source")?;
    let priority = int_arg(priority, "completion-add-items! #:priority")?;
    let match_kind = match_kind_arg(match_kind, "completion-add-items! #:match")?;
    let incomplete = bool_arg(incomplete, "completion-add-items! #:incomplete")?;
    let mut parsed = Vec::new();
    for entry in list_items(items, "completion-add-items! items")? {
        parsed.push(steel_to_json(&entry).map_err(generic_err)?);
    }
    let applied = require_cap(ctx.host.completions(), "completion-add-items!")?
        .completion_add_items(token, parsed, source, priority, match_kind, incomplete);
    Ok(SteelVal::BoolV(applied))
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
    require_cap(ctx.host.completions(), "completion-dismiss!")?
        .completion_dismiss()
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}
