//! Language-identity and grammar Steel builtins.

use steel::rvals::SteelVal;

use hume_editing::comment::{BlockCommentTokens, LineCommentTokens};
use steel::rerrs::SteelErr;

use crate::context::EvalMode;
use crate::{ConfigOrigin, Effect, GrammarReg, PendingLanguageReg, SteelCtx};
use hume_treesitter::registry::{
    FileExtension, FileGlob, GrammarSource, LanguageField, LanguageFields, OptionalQuery,
    QueryFiles,
};

use super::SteelResult;
use super::args::{
    HashEntry, cons_pair, hash_entry, list_of, list_to_strings, optional_hash_entry,
    optional_list_to_string_pairs, optional_list_to_strings, optional_path_arg,
    optional_string_arg, path_arg, string_arg,
};
use super::errors::generic_err;

/// `(%define-language! name extensions globs shebangs)`: init-only.
///
/// `extensions`, `globs` and `shebangs` must be lists of strings, each
/// extension a valid [`FileExtension`] and each glob a valid [`FileGlob`],
/// or the call raises. Pushes an
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
    let extensions = list_to_strings(exts_val, "%define-language! extensions")?
        .iter()
        .map(|ext| FileExtension::new(ext))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| generic_err(format!("define-language! {name:?}: {e}")))?;
    let globs = list_to_strings(globs_val, "%define-language! globs")?
        .iter()
        .map(|glob| {
            FileGlob::new(glob).map_err(|e| {
                generic_err(format!(
                    "define-language! {name:?}: invalid glob {glob:?}: {e}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    let keys = LanguageField::ALL.map(LanguageField::key);
    fields_of(&hash_entry(val, ctx_name, &keys)?)
}

/// [`language_fields`] for a value that may be `#f` (no data).
pub(crate) fn optional_language_fields(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<LanguageFields>, SteelErr> {
    let keys = LanguageField::ALL.map(LanguageField::key);
    optional_hash_entry(val, ctx_name, &keys)?
        .map(|entry| fields_of(&entry))
        .transpose()
}

fn fields_of(entry: &HashEntry) -> Result<LanguageFields, SteelErr> {
    let field = |field: LanguageField| {
        let key = field.key();
        let val = entry.optional(key).unwrap_or(SteelVal::BoolV(false));
        (val, entry.label(key))
    };
    let tokens_err = |label: &str, e| generic_err(format!("{label}: {e}"));

    let (roots, roots_label) = field(LanguageField::Roots);
    let (id, id_label) = field(LanguageField::LanguageId);
    let (line, line_label) = field(LanguageField::CommentTokens);
    let (block, block_label) = field(LanguageField::BlockCommentTokens);
    Ok(LanguageFields {
        roots: optional_list_to_strings(roots, &roots_label)?,
        lsp_language_id: optional_string_arg(id, &id_label)?,
        comment_tokens: optional_list_to_strings(line, &line_label)?
            .map(LineCommentTokens::new)
            .transpose()
            .map_err(|e| tokens_err(&line_label, e))?,
        block_comment_tokens: optional_list_to_string_pairs(
            block,
            &block_label,
            "a (start . end) pair",
        )?
        .map(BlockCommentTokens::new)
        .transpose()
        .map_err(|e| tokens_err(&block_label, e))?,
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

/// `(%register-grammar! name grammar-path symbol highlights-path optional-queries)`:
/// init or command. `optional-queries` is `#f` or a symbol-keyed hash from
/// `OptionalQuery::key` to a path string or `#f`; any other key is an error.
/// The Scheme-side `register-grammar!` wrapper (`prelude.scm`) builds the hash
/// from its keyword arguments.
///
/// - **Init mode**: pushes an `Effect::LanguageReg(PendingLanguageReg::Grammar)`,
///   which registers the grammar without loading it: it loads the first
///   time a buffer or an injection needs it.
/// - **Command mode**: registers and loads it immediately via the editor
///   host, so a grammar that cannot load fails the call, and pushes an
///   `Effect::GrammarSweep` to sweep already-open buffers afterward.
pub(crate) fn register_grammar(
    ctx: &mut SteelCtx,
    name: SteelVal,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
    optional_queries: SteelVal,
) -> SteelResult {
    let cwd = ctx.host.buffers().cwd();
    let under_cwd = |p: std::path::PathBuf| hume_platform::path::absolute_unresolved(&p, &cwd);
    let mut queries = QueryFiles::new(under_cwd(path_arg(
        highlights_path,
        "register-grammar! highlights-path",
    )?));
    let keys = OptionalQuery::ALL.map(OptionalQuery::key);
    if let Some(entry) = optional_hash_entry(
        optional_queries,
        "register-grammar! optional-queries",
        &keys,
    )? {
        for kind in OptionalQuery::ALL {
            let key = kind.key();
            let path = entry
                .optional(key)
                .map(|val| optional_path_arg(val, &entry.label(key)))
                .transpose()?
                .flatten();
            if let Some(path) = path {
                queries = queries.with(kind, under_cwd(path));
            }
        }
    }
    let reg = GrammarReg {
        name: string_arg(name, "register-grammar! name")?,
        source: GrammarSource {
            path: under_cwd(path_arg(grammar_path, "register-grammar! grammar-path")?),
            symbol: string_arg(symbol, "register-grammar! symbol")?,
            queries,
        },
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

/// `(grammar-optional-queries)` returns `((key . file-name) ...)` for every
/// `OptionalQuery`: the keys `%register-grammar!` accepts and the file each
/// one is read from.
pub(crate) fn grammar_optional_queries() -> SteelResult {
    let pairs = OptionalQuery::ALL
        .into_iter()
        .map(|kind| {
            cons_pair(
                SteelVal::SymbolV(kind.key().into()),
                SteelVal::StringV(kind.file_name().into()),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(list_of(pairs))
}

/// `(language-has-grammar? name)` returns `#t` if `name` has a grammar, loaded
/// or yet to load, and `#f` once its load has failed. It never loads one.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "language-has-grammar?")?;
    Ok(SteelVal::BoolV(ctx.host.language().has_grammar(&name)))
}

#[cfg(test)]
mod tests;
