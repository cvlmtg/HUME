//! `(set-option! key value)` / `(get-option key)` / `(set-buffer-option! pane
//! key value)` / `(get-buffer-option pane key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use hume_core::{BufferId, BufferIntrinsicOption, LanguageOptionText, OptionInput, PaneHandle};

use crate::SteelCtx;
use crate::types::{Effect, QueuedEffect};

use super::SteelResult;
use super::args::list_to_strings;
use super::errors::generic_err;

/// Coerce a Steel string/bool/int/list-of-strings settings value to what the
/// settings layer takes: the text `:set` would carry, or a list's words.
/// A named value is a string, the spelling `get-option` reads back, so a
/// symbol is refused. `ctx_name` names the argument in the error.
fn coerce_option_value(value: &SteelVal, ctx_name: &str) -> Result<OptionInput, SteelErr> {
    match value {
        SteelVal::StringV(s) => Ok(OptionInput::Text(s.to_string())),
        SteelVal::BoolV(b) => Ok(OptionInput::Text(b.to_string())),
        SteelVal::IntV(n) => Ok(OptionInput::Text(n.to_string())),
        SteelVal::ListV(_) => list_to_strings(value.clone(), ctx_name).map(OptionInput::Words),
        _ => steel::stop!(TypeMismatch =>
            "{ctx_name}: value must be a string, bool, integer, or list of strings, got {:?}", value),
    }
}

/// The error a global-option builtin raises for a buffer-intrinsic option,
/// which has no global value: `builtin` is the per-buffer builtin to use.
fn per_buffer_err(opt: BufferIntrinsicOption, builtin: &str) -> SteelErr {
    generic_err(format!("'{}' is per-buffer: use {builtin}", opt.name()))
}

/// `(set-option! key value)`
///
/// Sets the global setting `key` to `value`. The value may be a Steel string,
/// boolean, integer, or list of strings; only a word-list setting
/// (`shell`) takes a list. It is forwarded to the editor's settings layer, which is the single validating chokepoint
/// (`editor::settings::ops::apply_global`) regardless of caller, so this is
/// callable from any context: `init.scm`, plugin load, plugin activation, or
/// a plain command/hook body. Use `:set buffer …` from the command line, or
/// `(set-buffer-option! pane key value)` from a script, to override a setting
/// for a specific buffer instead. A `BufferIntrinsicOption` (`language`,
/// `line-ending`) has no global value and raises, naming
/// `set-buffer-option!`.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    if let Some(opt) = BufferIntrinsicOption::from_name(&key) {
        return Err(per_buffer_err(opt, "set-buffer-option!"));
    }
    let value = coerce_option_value(&value, "value")?;

    let origin = ctx.registration_origin();
    ctx.host
        .settings()
        .set_global_option(&key, value, origin)
        .map_err(generic_err)?;

    Ok(SteelVal::Void)
}

/// `(set-buffer-option! pane key value)`
///
/// Sets `key`'s per-buffer override on `pane`'s buffer to `value` (same
/// coercion as `set-option!`). The override persists on the buffer until
/// overwritten, same as `:set buffer key=value`.
///
/// `"line-ending"` takes `"lf"` or `"crlf"` and sets the line ending a save writes; it errors on a read-only buffer.
///
/// `"language"` takes a string, `""` meaning no language, and is queued as
/// `Effect::SetBufferLanguage` rather than written here: setting it can
/// activate plugins, which must not happen inside the current eval.
///
/// Command/hook context only (`cmd` kind). The idiomatic caller is an
/// `on-language-set` hook handler, which receives the target buffer id as an
/// explicit argument rather than relying on `(focused-pane)` (a live read
/// that may differ from the buffer whose language just changed).
pub(crate) fn set_buffer_option(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    key: String,
    value: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let origin = ctx.registration_origin();
    match BufferIntrinsicOption::from_name(&key) {
        Some(BufferIntrinsicOption::Language) => {
            let SteelVal::StringV(language) = value else {
                steel::stop!(TypeMismatch =>
                    "\"language\" must be a string, got {:?}", value);
            };
            let language = LanguageOptionText::decode(&language).map(str::to_string);
            ctx.push_effect(Effect::SetBufferLanguage {
                buffer: bid,
                language,
                origin,
            });
        }
        Some(BufferIntrinsicOption::LineEnding) => {
            let value = coerce_option_value(&value, "value")?;
            ctx.host
                .settings()
                .set_buffer_line_ending(bid, value, origin)
                .map_err(generic_err)?;
        }
        None => {
            let value = coerce_option_value(&value, "value")?;
            ctx.host
                .settings()
                .set_buffer_option(&key, value, bid, origin)
                .map_err(generic_err)?;
        }
    }
    Ok(SteelVal::Void)
}

/// `(get-option key)`: `key`'s global value, ignoring any buffer override
/// even if one exists (mirrors `set-option!`, `open` kind: callable from
/// any context, including `init.scm`). Use `(get-buffer-option pane key)`
/// for a specific buffer's effective value instead. A
/// `BufferIntrinsicOption` has no global value and raises, naming
/// `get-buffer-option`.
pub(crate) fn get_option(ctx: &mut SteelCtx, key: String) -> SteelResult {
    if let Some(opt) = BufferIntrinsicOption::from_name(&key) {
        return Err(per_buffer_err(opt, "get-buffer-option"));
    }
    ctx.host
        .settings()
        .get_global_option(&key)
        .map(SteelVal::from)
        .map_err(generic_err)
}

/// `(get-buffer-option pane key)`: the effective value of `key` for `pane`'s buffer:
/// its buffer override if one is set, else the global default. `"language"`
/// reads the buffer's language name (`""` for none), including a change
/// queued earlier in this same eval. `"line-ending"` reads `"lf"` or
/// `"crlf"`; it has no global default.
///
/// Command/hook context only (`cmd` kind). The idiomatic caller is an
/// `on-language-set` hook handler, which receives the target buffer id as an
/// explicit argument rather than relying on `(focused-pane)` (a live read
/// that may differ from the buffer whose language just changed), same
/// reasoning as `set-buffer-option!`.
pub(crate) fn get_buffer_option(ctx: &mut SteelCtx, pane: PaneHandle, key: String) -> SteelResult {
    let bid = pane.buffer();
    match BufferIntrinsicOption::from_name(&key) {
        Some(BufferIntrinsicOption::Language) => {
            let language = effective_language(ctx, bid);
            Ok(SteelVal::StringV(
                LanguageOptionText::encode(language.as_deref()).into(),
            ))
        }
        Some(BufferIntrinsicOption::LineEnding) => ctx
            .host
            .settings()
            .buffer_line_ending(bid)
            .map(|text| SteelVal::StringV(text.into()))
            .map_err(generic_err),
        None => ctx
            .host
            .settings()
            .get_buffer_option(&key, bid)
            .map(SteelVal::from)
            .map_err(generic_err),
    }
}

/// `bid`'s language as a script sees it: the buffer's stored language with
/// every `"language"` write queued so far this eval ranked onto it as the
/// editor will apply them, `None` for none.
fn effective_language(ctx: &mut SteelCtx, bid: BufferId) -> Option<String> {
    let mut language = ctx.host.buffers().buffer_stored_language(bid);
    for QueuedEffect { effect, .. } in ctx.effects.iter() {
        if let Effect::SetBufferLanguage {
            buffer,
            language: written,
            origin,
        } = effect
            && *buffer == bid
            && language.by.claim(*origin).is_applied()
        {
            language.name.clone_from(written);
        }
    }
    language.name
}

#[cfg(test)]
mod tests;
