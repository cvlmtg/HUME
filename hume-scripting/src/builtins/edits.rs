//! BufferText-edit application and cursor navigation primitives fed by LSP
//! responses (code actions, rename, go-to-definition); `insert-key!`,
//! `paste-text!` and `edit-text!` too, which aren't LSP-driven but share
//! `EditHost`'s capability gate (see that trait's own doc).

use std::rc::Rc;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{CharTextEdit, FocusedHandle, PasteAudience, ShownHandle};
use crate::visibility::Audience;
use crate::{Callback, SteelCtx};
use hume_core::{PaneHandle, Severity};

use super::SteelResult;
use super::args::{
    CHAR_OFFSET, END_CHAR, START_CHAR, hash_entry, hash_list, list_items, list_of,
    optional_generation_arg, optional_string_arg, optional_text_ref_arg, single_key_arg,
    string_arg, string_list, strings_arg, under_cwd,
};
use super::errors::generic_err;
use super::ids::{SteelPane, pane_or_false};
use super::wire::{tagged_json_arg, wire_text_edit_arg};

/// `(%apply-text-edits! pane edits expect-gen)`. `edits` is a list of
/// `JsonHandle`s onto wire `TextEdit`s (unconverted response elements, e.g.
/// from `textDocument/formatting`); see `wire_text_edit_arg`.
///
/// `edits` decodes manually via `wire_text_edit_arg` per entry rather than a
/// typed `Vec<WireTextEdit>` param: steel-core's blanket
/// `FromSteelVal for Vec<T>` impl discards the inner per-element error on
/// failure, replacing it with a generic message; decoding manually keeps
/// `wire_text_edit_arg`'s specific shape-error text.
pub(crate) fn apply_text_edits(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    edits: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let expect_gen = optional_generation_arg(expect_gen, pane.buffer(), "#:expect-generation")?;
    let parsed = list_items(edits, "edits")?
        .into_iter()
        .map(wire_text_edit_arg)
        .collect::<Result<Vec<_>, SteelErr>>()?;
    ctx.host
        .require(|h| h.edits())?
        .apply_text_edits(pane, parsed, expect_gen)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%apply-workspace-edit! pane wsedit expect-gen)`. `wsedit` is a JSON
/// handle onto a `WorkspaceEdit` from a server response. Its positions
/// decode using the handle's own tagged encoding (the server that produced
/// it; see `JsonHandle::position_encoding`), so unlike `goto-location!`'s
/// char-indexed shape a hand-built value raises: there is no server to have
/// negotiated an encoding with. Returns the number of buffers modified.
pub(crate) fn apply_workspace_edit(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    wsedit: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let (handle, encoding) = tagged_json_arg(&wsedit, "wsedit")?;
    let expect_gen = optional_generation_arg(expect_gen, pane.buffer(), "#:expect-generation")?;
    let count = ctx
        .host
        .require(|h| h.edits())?
        .apply_workspace_edit(pane, handle.value(), encoding, expect_gen)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(goto-location! pane loc)` → the pane, now showing the buffer it jumped
/// into, or `#f` when the place `loc` names no longer exists. `loc` is one
/// of three shapes, dispatched here (not in Scheme):
///
/// - a JSON handle onto a raw `Location`/`LocationLink` from a server
///   response: wire position, decoded and converted using the handle's own
///   tagged encoding. The server that produced the response negotiated it
///   for the request that's being answered, regardless of which file the
///   location points into (an LSP round-trip is async; the user is free to
///   switch panes while a request is in flight, which is exactly why the
///   jump lands in `pane`, not necessarily the focused one). Errors on a
///   hand-built value, same as `apply-workspace-edit!`; `#f` when the
///   location's line is past its file's last line.
/// - `(hash 'target path 'line l 'char-col c)`, already char-indexed: `path`
///   is a path string, never a URI, and the file is opened if it is not
///   already. `#f` when `l` is past the file's last line.
/// - `(hash 'target pane 'char-offset o)`, a char offset in `pane`'s buffer.
///   Raises when `o` is past the end of the buffer.
///
/// A hashmap is one of the char-indexed shapes and a handle is the wire one.
/// Keys that belong to the other char-indexed shape raise.
pub(crate) fn goto_location(ctx: &mut SteelCtx, pane: ShownHandle, loc: SteelVal) -> SteelResult {
    match &loc {
        SteelVal::HashMapV(map)
            if map.iter().any(
                |(k, _)| matches!(k, SteelVal::SymbolV(name) if name.as_str() == "target"),
            ) =>
        {
            let fields = hash_entry(
                loc.clone(),
                "loc",
                &["target", "line", "char-col", CHAR_OFFSET],
            )?;
            let target = fields.required("target")?;
            if let Some(handle) = super::ids::downcast_pane(&target) {
                for key in ["line", "char-col"] {
                    if fields.optional(key).is_some() {
                        steel::stop!(Generic =>
                            "a pane target takes 'char-offset, not '{}", key);
                    }
                }
                let char_offset = fields.usize(CHAR_OFFSET)?;
                ctx.host
                    .require(|h| h.edits())?
                    .goto_location_buffer(pane, handle.buffer(), char_offset)
                    .map(|shown| SteelPane::new(shown).into_steel_val())
                    .map_err(generic_err)
            } else {
                if fields.optional(CHAR_OFFSET).is_some() {
                    steel::stop!(Generic =>
                        "a path target takes 'line and 'char-col, not 'char-offset");
                }
                let path = string_arg(target, "target")?;
                if has_uri_scheme(&path) {
                    steel::stop!(Generic =>
                        "target must be a pane or a path, not the URI {}", path);
                }
                let path = under_cwd(
                    std::path::Path::new(&path),
                    &ctx.host.buffers().cwd(),
                    "target",
                )?;
                let line = fields.usize("line")?;
                let char_col = fields.usize("char-col")?;
                ctx.host
                    .require(|h| h.edits())?
                    .goto_location_path(pane, path, line, char_col)
                    .map(pane_or_false)
                    .map_err(generic_err)
            }
        }
        SteelVal::Custom(_) => {
            let (handle, encoding) = tagged_json_arg(&loc, "loc")?;
            ctx.host
                .require(|h| h.edits())?
                .goto_location_value(pane, handle.value(), encoding)
                .map(pane_or_false)
                .map_err(generic_err)
        }
        _ => steel::stop!(TypeMismatch =>
            "expected a Location handle from a server response or (hash 'target t 'line l 'char-col c) or (hash 'target pane 'char-offset o)"),
    }
}

/// Whether `s` starts with an RFC 3986 scheme and its `:`: an ASCII letter,
/// then letters, digits, `+`, `-` or `.`. A one-letter scheme is a Windows
/// drive (`C:\…`), so it does not count. A relative path whose first segment
/// holds a colon (`foo:bar/x`) reads as a URI too; `./foo:bar/x` does not.
fn has_uri_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    scheme.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `(insert-key! pane key)`: `key` is a `bind-key!`-syntax spec naming
/// exactly one chord (see [`single_key_arg`]), decoded here rather than
/// left to the host: the host trait takes an already-parsed `KeyEvent`,
/// same as every other typed `EditHost` param.
pub(crate) fn insert_key(ctx: &mut SteelCtx, pane: FocusedHandle, key: SteelVal) -> SteelResult {
    let key = single_key_arg(key, "key")?;
    ctx.host
        .require(|h| h.edits())?
        .insert_key(pane, key)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(paste-text! pane text)`: paste `text`, a string or a list of strings,
/// into whatever has focus.
pub(crate) fn paste_text(ctx: &mut SteelCtx, pane: FocusedHandle, text: SteelVal) -> SteelResult {
    let values = strings_arg(text, "text")?;
    ctx.host
        .require(|h| h.edits())?
        .paste_text(pane, values)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%edit-text! pane edits expect-gen)`: replace char ranges of `pane`'s
/// buffer, each entry a `(hash 'start-char s 'end-char e 'text t)`, as one
/// edit; `bootstrap.scm`'s `edit-text!` supplies `#:expect-generation`'s `#f`
/// default.
pub(crate) fn edit_text(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    edits: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let expect_gen = optional_generation_arg(expect_gen, pane.buffer(), "#:expect-generation")?;
    let edits = hash_list(edits, "edits", &[START_CHAR, END_CHAR, "text"], |entry| {
        let (start, end) = entry.char_range()?;
        Ok(CharTextEdit {
            start,
            end,
            text: entry.string("text")?,
        })
    })?;
    ctx.host
        .require(|h| h.edits())?
        .edit_text(pane, edits, expect_gen)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(goto-revision! pane id)`: move `pane`'s buffer to revision `id` of its
/// undo history (a number from `(buffer-undo-tree pane)`), across branches.
/// Raises when the buffer has no such revision or is read-only.
pub(crate) fn goto_revision(ctx: &mut SteelCtx, pane: ShownHandle, revision: usize) -> SteelResult {
    ctx.host
        .require(|h| h.edits())?
        .goto_revision(pane, revision)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;

// ── on-paste ──────────────────────────────────────────────────────────────────

/// The error for an on-paste call made with no paste waiting.
fn no_paste() -> SteelErr {
    generic_err("no paste is waiting for its on-paste handlers")
}

/// Who handles the waiting paste, or the error for a call made with none.
fn waiting_paste(ctx: &mut SteelCtx) -> Result<PasteAudience, SteelErr> {
    ctx.host
        .require(|h| h.edits())?
        .paste_audience()
        .ok_or_else(no_paste)
}

/// The handlers `audience` reaches, in registration order.
fn paste_handlers<'a>(
    ctx: &'a SteelCtx,
    audience: &'a PasteAudience,
) -> impl Iterator<Item = &'a Callback> {
    ctx.registries
        .hooks
        .matching(
            audience.event,
            None,
            Audience::Seeing(audience.language.as_deref()),
        )
        .map(|entry| &entry.handler)
}

/// `(%paste-handler-failed! message)`: reports `message` about the
/// on-paste handler whose frame is open, as an error naming the code that
/// registered it.
pub(crate) fn paste_handler_failed(ctx: &mut SteelCtx, message: String) -> SteelResult {
    let owner = ctx.owner().to_string();
    ctx.log(
        Severity::Error,
        format!("on-paste handler from {owner}: {message}"),
    );
    Ok(SteelVal::Void)
}

/// `(%start-paste-finish!)`: starts finishing the paste the last call
/// parked; `#f` when none is parked, and for a host with no `EditHost`.
pub(crate) fn start_paste_finish(ctx: &mut SteelCtx) -> SteelResult {
    let started = ctx
        .host
        .edits()
        .is_some_and(|edits| edits.start_paste_finish());
    Ok(SteelVal::BoolV(started))
}

/// `(%paste-event-plugins)`: `(plugin file)` for every plugin entry waiting
/// to activate on the hook the waiting paste's handlers register on.
pub(crate) fn paste_event_plugins(ctx: &mut SteelCtx) -> SteelResult {
    let audience = waiting_paste(ctx)?;
    let entries = ctx.registries.lazy_registry.event_plugins(audience.event);
    Ok(list_of(entries.into_iter().map(|id| {
        string_list([id.plugin.to_string(), id.file.as_str().to_owned()])
    })))
}

/// `(%paste-plan)`: `(pane requests handlers)` for the waiting paste: the
/// pane it lands in, `(text replaced)` per selection (`replaced` is `#f`
/// at a cursor), and how many handlers it reaches.
pub(crate) fn paste_plan(ctx: &mut SteelCtx) -> SteelResult {
    let audience = waiting_paste(ctx)?;
    let q = ctx
        .host
        .require(|h| h.edits())?
        .paste_question()
        .ok_or_else(no_paste)?;
    ctx.paste_handlers = paste_handlers(ctx, &audience).cloned().collect();
    let handlers = ctx.paste_handlers.len();
    let requests = q.requests.into_iter().map(|(text, replaced)| {
        let replaced = replaced.map_or(SteelVal::BoolV(false), |r| SteelVal::StringV(r.into()));
        list_of([SteelVal::StringV(text.into()), replaced])
    });
    Ok(list_of([
        SteelPane(q.pane).into_steel_val(),
        list_of(requests),
        SteelVal::IntV(handlers as isize),
    ]))
}

/// `(%enter-paste-handler! i)`: enters the frame that runs the `i`th
/// handler `%paste-plan` found as the code that registered it, read-only until
/// `%leave-command!` leaves it, and returns `(proc depth)` as
/// `%enter-command!` does.
pub(crate) fn enter_paste_handler(ctx: &mut SteelCtx, i: usize) -> SteelResult {
    let (proc, owner) = ctx
        .paste_handlers
        .get(i)
        .map(|h| (h.proc().value().clone(), Rc::clone(&h.provenance().owner)))
        .ok_or_else(|| generic_err(format!("the waiting paste has no on-paste handler {i}")))?;
    let depth = ctx.host.enter_paste_handler_call(owner);
    Ok(list_of([proc, SteelVal::IntV(depth as isize)]))
}

/// `(%paste-result value)`: `value`, what the handler whose frame is open
/// returned, when it is text or `#f` and the handler made no write the
/// frame refused; otherwise reports why and returns `#f`.
pub(crate) fn paste_result(ctx: &mut SteelCtx, value: SteelVal) -> SteelResult {
    if let Some(refused) = ctx.host.take_refusal() {
        paste_handler_failed(ctx, refused)?;
        return Ok(SteelVal::BoolV(false));
    }
    if optional_text_ref_arg(&value, "on-paste").is_ok() {
        return Ok(value);
    }
    paste_handler_failed(ctx, format!("returned {value}, not a string or #f"))?;
    Ok(SteelVal::BoolV(false))
}

/// `(%apply-paste! answers)`: lands the waiting paste with `answers`, the
/// handlers' text per selection (`#f`: its own).
pub(crate) fn apply_paste(ctx: &mut SteelCtx, answers: SteelVal) -> SteelResult {
    let answers = list_items(answers, "answers")?
        .into_iter()
        .map(|answer| optional_string_arg(answer, "answer"))
        .collect::<Result<Vec<_>, _>>()?;
    let landed = ctx.host.require(|h| h.edits())?.apply_paste(answers);
    ctx.paste_handlers.clear();
    landed
        .ok_or_else(no_paste)?
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}
