//! Language-identity and grammar Steel builtins.

use steel::rvals::SteelVal;

use hume_editing::comment::{BlockCommentTokens, LineCommentTokens};
use steel::rerrs::SteelErr;

use crate::context::EvalMode;
use crate::{ConfigOrigin, Effect, GrammarReg, LanguageFields, PendingLanguageReg, SteelCtx};

use super::SteelResult;
use super::args::{
    hash_entry, list_to_strings, optional_list_items, optional_list_to_strings, optional_path_arg,
    optional_string_arg, pair_fields, path_arg, string_arg,
};
use super::errors::generic_err;

/// `(%define-language! name extensions globs shebangs)`: init-only.
///
/// `extensions`, `globs` and `shebangs` must be lists of strings. Pushes an
/// `Effect::LanguageReg(PendingLanguageReg::Identity)`;
/// `Editor::apply_pending_language_regs` applies it as part of
/// `Editor::apply_script_effects`.
pub(crate) fn define_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    exts_val: SteelVal,
    globs_val: SteelVal,
    shebangs_val: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "%define-language! name")?;
    let extensions = list_to_strings(exts_val, "%define-language! extensions")?;
    let globs = list_to_strings(globs_val, "%define-language! globs")?;
    let shebangs = list_to_strings(shebangs_val, "%define-language! shebangs")?;
    ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Identity {
        name,
        extensions,
        globs,
        shebangs,
    }));
    Ok(SteelVal::Void)
}

/// Decodes a symbol-keyed hash of per-language values: `'roots` (strings),
/// `'language-id` (a string), `'comment-tokens` (strings) and
/// `'block-comment-tokens` (`(start . end)` string pairs). A missing key or
/// `#f` is `None`; any other key is an error. The one decoder for both
/// `%configure-language!` and a bundled data file.
pub(crate) fn language_fields(val: SteelVal, ctx_name: &str) -> Result<LanguageFields, SteelErr> {
    let entry = hash_entry(
        val,
        ctx_name,
        &[
            "roots",
            "language-id",
            "comment-tokens",
            "block-comment-tokens",
        ],
    )?;
    let field = |key| entry.optional(key).unwrap_or(SteelVal::BoolV(false));
    let token_err = |key: &str, e| generic_err(format!("{}: {e}", entry.label(key)));

    let block_label = entry.label("block-comment-tokens");
    let block_pairs = optional_list_items(field("block-comment-tokens"), &block_label)?
        .map(|items| {
            items
                .into_iter()
                .map(|item| {
                    let (start, end) = pair_fields(item, &block_label, "a (start . end) pair")?;
                    Ok((
                        string_arg(start, &block_label)?,
                        string_arg(end, &block_label)?,
                    ))
                })
                .collect::<Result<Vec<_>, SteelErr>>()
        })
        .transpose()?;

    Ok(LanguageFields {
        roots: optional_list_to_strings(field("roots"), &entry.label("roots"))?,
        lsp_language_id: optional_string_arg(field("language-id"), &entry.label("language-id"))?,
        comment_tokens: optional_list_to_strings(
            field("comment-tokens"),
            &entry.label("comment-tokens"),
        )?
        .map(LineCommentTokens::new)
        .transpose()
        .map_err(|e| token_err("comment-tokens", e))?,
        block_comment_tokens: block_pairs
            .map(BlockCommentTokens::new)
            .transpose()
            .map_err(|e| token_err("block-comment-tokens", e))?,
    })
}

/// `(%configure-language! name fields)`: init-only, like `%define-language!`.
/// `fields` is the hash [`language_fields`] decodes; a field it leaves `None`
/// stays as it is. Pushes an `Effect::LanguageReg(PendingLanguageReg::Configure)`,
/// applied in queue order after any `%define-language!` queued before it. A
/// call inside a plugin body is a plugin's, any other is the user's.
pub(crate) fn configure_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    fields_val: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "%configure-language! name")?;
    let fields = language_fields(fields_val, "%configure-language!")?;
    let origin = if ctx.mode() == EvalMode::Init {
        ConfigOrigin::User
    } else {
        ConfigOrigin::Plugin
    };
    ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Configure {
        name,
        fields,
        origin,
    }));
    Ok(SteelVal::Void)
}

/// `(%register-grammar! name grammar-path symbol highlights-path injections-path
/// textobjects-path)`: init or command. `injections-path` and
/// `textobjects-path` are each a string or `#f`; the Scheme-side
/// `register-grammar!` wrapper (`prelude.scm`) supplies `#f` for either
/// keyword the caller omits.
///
/// - **Init mode**: pushes an `Effect::LanguageReg(PendingLanguageReg::Grammar)`.
/// - **Command mode**: attaches immediately via the editor host and pushes
///   an `Effect::GrammarSweep` to sweep already-open buffers afterward.
pub(crate) fn register_grammar(
    ctx: &mut SteelCtx,
    name: SteelVal,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
    injections_path: SteelVal,
    textobjects_path: SteelVal,
) -> SteelResult {
    let cwd = ctx.host.buffers().cwd();
    let under_cwd = |p: std::path::PathBuf| hume_platform::path::absolute_unresolved(&p, &cwd);
    let reg = GrammarReg {
        name: string_arg(name, "register-grammar! name")?,
        grammar_path: under_cwd(path_arg(grammar_path, "register-grammar! grammar-path")?),
        symbol: string_arg(symbol, "register-grammar! symbol")?,
        highlights_path: under_cwd(path_arg(
            highlights_path,
            "register-grammar! highlights-path",
        )?),
        injections_path: optional_path_arg(injections_path, "register-grammar! injections-path")?
            .map(under_cwd),
        textobjects_path: optional_path_arg(
            textobjects_path,
            "register-grammar! textobjects-path",
        )?
        .map(under_cwd),
    };

    if ctx.session == crate::context::EvalSession::Init {
        ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Grammar(reg)));
        return Ok(SteelVal::Void);
    }

    // Command mode: attach immediately via host and trigger a buffer sweep.
    // The host (`EditorHostImpl::attach_grammar`) owns the `register-grammar! '<name>':`
    // prefix; just lift its String error into a SteelErr without re-prefixing.
    ctx.host
        .language()
        .attach_grammar(&reg)
        .map_err(generic_err)?;
    ctx.push_effect(Effect::GrammarSweep(reg.name));
    Ok(SteelVal::Void)
}

/// `(language-has-grammar? name)` returns `#t` if `name` has an attached grammar.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "language-has-grammar?")?;
    Ok(SteelVal::BoolV(ctx.host.language().has_grammar(&name)))
}

#[cfg(test)]
mod tests;
