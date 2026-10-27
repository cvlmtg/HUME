//! `(set-option! key value)` / `(get-option key)` / `(set-buffer-option! pane
//! key value)` / `(get-buffer-option pane key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use hume_core::{
    BufferId, BufferIntrinsicOption, LanguageOptionText, OptionInput, PaneHandle,
    StatuslineSections,
};

use crate::SteelCtx;
use crate::host::OptionValue;
use crate::types::{Effect, QueuedEffect};

use super::SteelResult;
use super::args::{hash_entry, list_to_strings, string_list, symbol_hash};
use super::errors::generic_err;

/// What an option takes besides the text `:set` would carry: nothing
/// (`One`), a list of words, or the statusline's sections.
#[derive(Clone, Copy)]
enum Takes {
    One,
    Words,
    Sections,
}

impl Takes {
    /// What `key` takes: an option reads back in the shape it is set in, so
    /// its global value tells. Raises for an unknown `key`.
    fn of(ctx: &mut SteelCtx, key: &str) -> Result<Self, SteelErr> {
        let current = ctx
            .host
            .settings()
            .get_global_option(key)
            .map_err(generic_err)?;
        Ok(match current {
            OptionValue::List(_) => Takes::Words,
            OptionValue::Sections(_) => Takes::Sections,
            OptionValue::Bool(_) | OptionValue::Int(_) | OptionValue::Str(_) => Takes::One,
        })
    }
}

/// Coerce a Steel string/bool/int/list-of-strings/sections-hash value for
/// option `key`, which takes `takes`, to what the settings layer takes: the
/// text `:set` would carry, a list's words, or the statusline's sections. A
/// list or a hash is refused for an option that does not take one, before
/// what it holds is read. A named value is a string, the spelling
/// `get-option` reads back, so a symbol is refused.
fn coerce_option_value(value: &SteelVal, key: &str, takes: Takes) -> Result<OptionInput, SteelErr> {
    match (value, takes) {
        (SteelVal::StringV(s), _) => Ok(OptionInput::Text(s.to_string())),
        (SteelVal::BoolV(b), _) => Ok(OptionInput::Text(b.to_string())),
        (SteelVal::IntV(n), _) => Ok(OptionInput::Text(n.to_string())),
        (SteelVal::ListV(_), Takes::Words) => {
            list_to_strings(value.clone(), "value").map(OptionInput::Words)
        }
        (SteelVal::HashMapV(_), Takes::Sections) => {
            decode_sections(value.clone()).map(OptionInput::Sections)
        }
        (SteelVal::ListV(_), _) => Err(generic_err(format!("'{key}' takes one value, not a list"))),
        (SteelVal::HashMapV(_), _) => {
            Err(generic_err(format!("'{key}' takes one value, not a hash")))
        }
        _ => steel::stop!(TypeMismatch =>
            "value must be a string, bool, integer, list of strings, or statusline sections hash, got {:?}", value),
    }
}

/// The keys of the statusline's `(hash 'left 'center 'right)`, each a list of
/// element names.
const SECTION_KEYS: &[&str] = &["left", "center", "right"];

/// Decodes a statusline sections hash; every section is required.
fn decode_sections(value: SteelVal) -> Result<StatuslineSections, SteelErr> {
    let entry = hash_entry(value, "value", SECTION_KEYS)?;
    let section = |key: &str| list_to_strings(entry.required(key)?, &entry.label(key));
    Ok(StatuslineSections {
        left: section("left")?,
        center: section("center")?,
        right: section("right")?,
    })
}

/// The statusline's sections as `get-option` reads them back:
/// [`decode_sections`]'s inverse.
pub(crate) fn sections_value(sections: StatuslineSections) -> SteelVal {
    symbol_hash([
        ("left", string_list(sections.left)),
        ("center", string_list(sections.center)),
        ("right", string_list(sections.right)),
    ])
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
    let takes = Takes::of(ctx, &key)?;
    let value = coerce_option_value(&value, &key, takes)?;

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
            let value = coerce_option_value(&value, &key, Takes::One)?;
            ctx.host
                .settings()
                .set_buffer_line_ending(bid, value, origin)
                .map_err(generic_err)?;
        }
        None => {
            let takes = Takes::of(ctx, &key)?;
            let value = coerce_option_value(&value, &key, takes)?;
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
