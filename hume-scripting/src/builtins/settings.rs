//! `(set-option! key value)` / `(get-option key)` / `(set-buffer-option! pane
//! key value)` / `(get-buffer-option pane key)` builtins.

use steel::rvals::SteelVal;

use hume_core::{
    BufferId, BufferIntrinsicOption, LanguageOptionText, OptionInput, OptionType, OptionValue,
    StatuslineSections,
};

use crate::SteelCtx;
use crate::host::option_value_to_steel;
use crate::types::{Effect, QueuedEffect};

use super::SteelResult;
use super::args::{hash_entry, list_to_strings, string_list, symbol_hash};
use super::errors::{generic_err, message};

/// `value` as the editor's option layer takes it. Nothing is refused here: a
/// list or hash whose contents do not decode, and a value of no option type,
/// reach the editor as such, which refuses them once it knows the option. An
/// unknown option is then reported as unknown, and a list given to a
/// one-value option as the wrong type, whatever the list holds.
fn option_input(value: SteelVal) -> OptionInput {
    let decoded = |ty, value: Result<OptionValue, _>| match value {
        Ok(value) => OptionInput::Value(value),
        Err(e) => OptionInput::Unreadable {
            ty,
            error: message(&e),
        },
    };
    match value {
        SteelVal::BoolV(b) => OptionInput::Value(OptionValue::Bool(b)),
        SteelVal::IntV(n) => OptionInput::Value(OptionValue::Int(n as i64)),
        SteelVal::StringV(s) => OptionInput::Value(OptionValue::Str(s.to_string())),
        value @ SteelVal::ListV(_) => decoded(
            OptionType::List,
            list_to_strings(value, "value").map(OptionValue::List),
        ),
        value @ SteelVal::HashMapV(_) => decoded(
            OptionType::Sections,
            decode_sections(value).map(OptionValue::Sections),
        ),
        SteelVal::SymbolV(s) => OptionInput::Foreign(format!("the symbol '{s}")),
        value => OptionInput::Foreign(value.to_string()),
    }
}

/// The keys of the statusline's `(hash 'left 'center 'right)`, each a list of
/// element names.
const SECTION_KEYS: &[&str] = &["left", "center", "right"];

/// Decodes a statusline sections hash; every section is required.
fn decode_sections(value: SteelVal) -> Result<StatuslineSections, steel::rerrs::SteelErr> {
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

/// Whether `key` names the `language` option, which a script's write queues
/// and a script's read ranks against that queue, so neither reaches the host.
fn is_language(key: &str) -> bool {
    key == BufferIntrinsicOption::Language.name()
}

/// `(set-option! key value)`
///
/// Sets the global setting `key` to `value`, in the type `get-option` reads
/// it back in: a boolean, an integer, a string, a list of strings for a
/// word-list setting (`shell`), or the statusline's sections hash. It is
/// forwarded to the editor's settings layer, which is the single validating chokepoint
/// (`editor::settings::ops::apply_global`) regardless of caller, so this is
/// callable from any context: `init.scm`, plugin load, plugin activation, or
/// a plain command/hook body. Use `:set buffer …` from the command line, or
/// `(set-buffer-option! pane key value)` from a script, to override a setting
/// for a specific buffer instead. A `BufferIntrinsicOption` (`language`,
/// `line-ending`) has no global value and raises, naming
/// `set-buffer-option!`.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    let origin = ctx.registration_origin();
    ctx.host
        .settings()
        .set_global_option(&key, option_input(value), origin)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-buffer-option! pane key value)`
///
/// Sets `key`'s per-buffer override on `pane`'s buffer to `value`, in the
/// type `set-option!` takes it in. The override persists on the buffer until
/// overwritten, same as `:set buffer key=value`.
///
/// `"line-ending"` takes `"lf"` or `"crlf"` and sets the line ending a save writes; it errors on a read-only buffer.
///
/// `"language"` takes a string, `""` meaning no language, and is queued as
/// `Effect::SetBufferLanguage` rather than written here: setting it can
/// activate plugins, which must not happen inside the current eval.
///
/// A global-only option has no buffer value and raises, naming
/// `set-option!`.
///
/// Command/hook context only (`cmd` kind). The idiomatic caller is an
/// `on-language-set` hook handler, which receives a pane naming the target
/// buffer as an explicit argument rather than relying on `(focused-pane)` (a
/// live read that may differ from the buffer whose language just changed).
pub(crate) fn set_buffer_option(
    ctx: &mut SteelCtx,
    bid: BufferId,
    key: String,
    value: SteelVal,
) -> SteelResult {
    let origin = ctx.registration_origin();
    if is_language(&key) {
        let SteelVal::StringV(language) = value else {
            return Err(generic_err(format!(
                "invalid value for '{key}': expected {}, got {value}",
                OptionType::Str.name()
            )));
        };
        let language = LanguageOptionText::decode(&language).map(str::to_string);
        ctx.push_effect(Effect::SetBufferLanguage {
            buffer: bid,
            language,
            origin,
        });
    } else {
        ctx.host
            .settings()
            .set_buffer_option(&key, option_input(value), bid, origin)
            .map_err(generic_err)?;
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
    ctx.host
        .settings()
        .get_global_option(&key)
        .map(option_value_to_steel)
        .map_err(generic_err)
}

/// `(get-buffer-option pane key)`: the effective value of `key` for `pane`'s buffer:
/// its buffer override if one is set, else the global default. `"language"`
/// reads the buffer's language name (`""` for none), including a change
/// queued earlier in this same eval. `"line-ending"` reads `"lf"` or
/// `"crlf"`; it has no global default. A global-only option has no buffer
/// value and raises, naming `get-option`.
///
/// Command/hook context only (`cmd` kind). The idiomatic caller is an
/// `on-language-set` hook handler, which receives a pane naming the target
/// buffer as an explicit argument rather than relying on `(focused-pane)` (a
/// live read that may differ from the buffer whose language just changed), same
/// reasoning as `set-buffer-option!`.
pub(crate) fn get_buffer_option(ctx: &mut SteelCtx, bid: BufferId, key: String) -> SteelResult {
    if is_language(&key) {
        let language = effective_language(ctx, bid);
        return Ok(SteelVal::StringV(
            LanguageOptionText::encode(language.as_deref()).into(),
        ));
    }
    ctx.host
        .settings()
        .get_buffer_option(&key, bid)
        .map(option_value_to_steel)
        .map_err(generic_err)
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
