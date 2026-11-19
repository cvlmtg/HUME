//! Language-identity and grammar Steel builtins.

use steel::rvals::SteelVal;

use hume_core::{
    BlockCommentTokens, BufferId, FileExtension, FileGlob, GrammarSource, LanguageField,
    LanguageFields, LanguageIdentity, LineCommentTokens, OptionalQuery, QueryFiles,
    ShebangInterpreter,
};
use steel::rerrs::SteelErr;

use crate::host::Captures;
use crate::{Effect, GrammarReg, PendingLanguageReg, SteelCtx};

use super::SteelResult;
use super::args::{
    HashEntry, cons_pair, hash_entry, list_of, list_to_strings, optional_hash_entry,
    optional_list_to_string_pairs, optional_list_to_strings, optional_path_arg,
    optional_string_arg, path_arg, string_arg, under_cwd,
};
use super::buffers::{line_ranges_arg, live_text};
use super::errors::generic_err;
use super::ids::{ParsedText, downcast};

/// `(%define-language! name extensions globs shebangs)`: init-only.
///
/// `extensions`, `globs` and `shebangs` must be lists of strings, each
/// extension a valid [`FileExtension`], each glob a valid [`FileGlob`] and
/// each shebang a valid [`ShebangInterpreter`], or the call raises. Pushes an
/// `Effect::LanguageReg(PendingLanguageReg::Identity)`, ranked by the origin
/// of the code that made it; `Editor::apply_pending_language_regs` applies
/// it as part of `Editor::apply_script_effects`.
pub(crate) fn define_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    exts_val: SteelVal,
    globs_val: SteelVal,
    shebangs_val: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "name")?;
    let extensions = list_to_strings(exts_val, "extensions")?
        .into_iter()
        .map(FileExtension::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| generic_err(format!("{name:?}: {e}")))?;
    let globs = list_to_strings(globs_val, "globs")?
        .iter()
        .map(|glob| {
            FileGlob::new(glob)
                .map_err(|e| generic_err(format!("{name:?}: invalid glob {glob:?}: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let shebangs = list_to_strings(shebangs_val, "shebangs")?
        .into_iter()
        .map(ShebangInterpreter::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| generic_err(format!("{name:?}: {e}")))?;
    let origin = ctx.registration_origin();
    ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Identity {
        name,
        identity: LanguageIdentity {
            extensions,
            globs,
            shebangs,
        },
        origin,
    }))?;
    Ok(SteelVal::Void)
}

/// Decodes a symbol-keyed hash of per-language values: `'roots` (strings),
/// `'language-id` (a string), `'comment-tokens` (strings) and
/// `'block-comment-tokens` (`(start . end)` string pairs). A missing key or
/// `#f` is `None`; any other key is an error. The one decoder for both
/// `%configure-language!` and a bundled data file.
pub(crate) fn language_fields(val: SteelVal, ctx_name: &str) -> Result<LanguageFields, SteelErr> {
    let keys = LanguageField::NAMES;
    fields_of(&hash_entry(val, ctx_name, &keys)?)
}

/// [`language_fields`] for a value that may be `#f` (no data).
pub(crate) fn optional_language_fields(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<LanguageFields>, SteelErr> {
    let keys = LanguageField::NAMES;
    optional_hash_entry(val, ctx_name, &keys)?
        .map(|entry| fields_of(&entry))
        .transpose()
}

fn fields_of(entry: &HashEntry) -> Result<LanguageFields, SteelErr> {
    let field = |field: LanguageField| {
        let key = field.name();
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
/// applied in queue order after any `%define-language!` queued before it,
/// ranked by the origin of the code that made it.
pub(crate) fn configure_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    fields_val: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "name")?;
    let fields = language_fields(fields_val, "fields")?;
    let origin = ctx.registration_origin();
    ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Configure {
        name,
        fields,
        origin,
    }))?;
    Ok(SteelVal::Void)
}

/// `(%register-grammar! name grammar-path symbol highlights-path optional-queries)`:
/// init or command. `optional-queries` is `#f` or a symbol-keyed hash from
/// `OptionalQuery::name` to a path string or `#f`; any other key is an error.
/// The Scheme-side `register-grammar!` wrapper (`prelude.scm`) builds the hash
/// from its keyword arguments.
///
/// - **Init mode**: pushes an `Effect::LanguageReg(PendingLanguageReg::Grammar)`,
///   which registers the grammar without loading it: it loads the first
///   time a buffer or an injection needs it.
/// - **Command mode**: registers and loads it immediately via the editor
///   host, so a grammar that cannot load fails the call, and pushes an
///   `Effect::GrammarSweep` to sweep already-open buffers afterward.
///
/// Either way it ranks as `SteelCtx::registration_origin`: a grammar a
/// higher origin registered stays.
pub(crate) fn register_grammar(
    ctx: &mut SteelCtx,
    name: SteelVal,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
    optional_queries: SteelVal,
) -> SteelResult {
    let reg = GrammarReg {
        name: string_arg(name, "name")?,
        source: grammar_source(ctx, grammar_path, symbol, highlights_path, optional_queries)?,
        origin: ctx.registration_origin(),
    };

    if ctx.session == crate::context::EvalSession::Init {
        ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Grammar(reg)))?;
        return Ok(SteelVal::Void);
    }

    // Command mode: attach immediately via host and trigger a buffer sweep.
    // The host's error names the grammar.
    let attached = ctx
        .host
        .language()
        .attach_grammar(&reg)
        .map_err(generic_err)?;
    if attached {
        ctx.push_effect(Effect::GrammarSweep(reg.name))?;
    }
    Ok(SteelVal::Void)
}

/// `(%check-grammar! name grammar-path symbol highlights-path optional-queries)`:
/// the arguments of `%register-grammar!`. Loads the grammar they name as
/// `name`'s would load, registering nothing, and raises when it cannot
/// load: its library does not open, its ABI is out of range, or its
/// `highlights.scm` cannot be read or does not compile. The same in every
/// session, and whatever origin holds `name`'s grammar.
pub(crate) fn check_grammar(
    ctx: &mut SteelCtx,
    name: SteelVal,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
    optional_queries: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "name")?;
    let source = grammar_source(ctx, grammar_path, symbol, highlights_path, optional_queries)?;
    ctx.host
        .language()
        .check_grammar(&name, &source)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// The grammar `%register-grammar!`'s and `%check-grammar!`'s path, symbol
/// and query arguments name, each path joined onto `(cwd)`.
fn grammar_source(
    ctx: &mut SteelCtx,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
    optional_queries: SteelVal,
) -> Result<GrammarSource, SteelErr> {
    let cwd = ctx.host.buffers().cwd();
    let resolve = |val: SteelVal, ctx_name: &str| {
        under_cwd(&path_arg(val, ctx_name)?, &cwd, ctx_name).map(|path| path.to_path_buf())
    };
    let mut queries = QueryFiles::new(resolve(highlights_path, "highlights-path")?);
    let keys = OptionalQuery::NAMES;
    if let Some(entry) = optional_hash_entry(optional_queries, "optional-queries", &keys)? {
        for kind in OptionalQuery::ALL {
            let key = kind.name();
            let path = entry
                .optional(key)
                .map(|val| optional_path_arg(val, &entry.label(key)))
                .transpose()?
                .flatten();
            if let Some(path) = path {
                queries = queries.with(
                    kind,
                    under_cwd(&path, &cwd, &entry.label(key))?.to_path_buf(),
                );
            }
        }
    }
    Ok(GrammarSource {
        path: resolve(grammar_path, "grammar-path")?,
        symbol: string_arg(symbol, "symbol")?,
        queries,
    })
}

/// `(%grammar-optional-queries)` returns `((key . file-name) ...)` for every
/// `OptionalQuery`: the keys `%register-grammar!` accepts and the file each
/// one is read from.
pub(crate) fn grammar_optional_queries() -> SteelResult {
    let pairs = OptionalQuery::ALL
        .into_iter()
        .map(|kind| {
            cons_pair(
                SteelVal::SymbolV(kind.name().into()),
                SteelVal::StringV(kind.file_name().into()),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(list_of(pairs))
}

/// `(language-has-grammar? name)` returns `#t` if `name` has a grammar, loaded
/// or yet to load, and `#f` once its load has failed. It never loads one.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "name")?;
    Ok(SteelVal::BoolV(ctx.host.language().has_grammar(&name)))
}

/// `(%syntax-captures pane language query-source ranges)`: Rust half of the
/// bootstrap-wrapped `(syntax-captures pane language query-source #:ranges ..)`.
/// `ranges` is a list of line range values (see
/// [`super::args::line_range_value`], the shape `viewport-range` returns) or
/// `#f` for the whole buffer.
///
/// Returns a list of `(list name start-char end-char)`, one per node
/// `query-source` captures in `pane`'s buffer, over every layer of `language`
/// (the root, or a language it injects), as
/// [`crate::host::SyntaxHost::syntax_captures`] gives them: `name` the capture
/// name without its `@`, one string shared by every entry of that name. `'()`
/// when the buffer has no tree for its current text.
///
/// With `ranges`, a match is returned when the node its pattern is rooted at
/// intersects the lines of one of them, with every capture of the match.
///
/// Raises when `language` has no grammar, `query-source` does not compile, or
/// `ranges` is not a list or holds a range that is malformed or past the
/// buffer's line count.
pub(crate) fn syntax_captures(
    ctx: &mut SteelCtx,
    bid: BufferId,
    language: String,
    query: String,
    ranges: SteelVal,
) -> SteelResult {
    // Checked first, so a host with no syntax support says so before the
    // buffer or the ranges are read.
    ctx.host.require(|h| h.syntax())?;
    let line_count = live_text(ctx, bid)?.content_line_count();
    let ranges = line_ranges_arg(ranges, line_count)?;
    let captures = ctx
        .host
        .require(|h| h.syntax())?
        .syntax_captures(bid, &language, &query, &ranges)
        .map_err(generic_err)?;
    Ok(captures_list(&captures))
}

/// `(parsed-text-captures parsed language query-source)`: the
/// [`syntax_captures`] result for the text `parsed` names, a
/// `format-text/<lang>` command's second argument, over every one of its
/// lines, as [`crate::host::LanguageHost::parsed_text_captures`] gives them.
/// It reads the tree the editor parsed the text into and never parses.
///
/// Raises when `parsed` is not a parsed text or its call has returned,
/// `language` has no grammar, or `query-source` does not compile.
pub(crate) fn parsed_text_captures(
    ctx: &mut SteelCtx,
    parsed: SteelVal,
    language: String,
    query: String,
) -> SteelResult {
    let ParsedText(token) = downcast(&parsed)
        .ok_or_else(|| generic_err("parsed: expected a format-text command's parsed text"))?;
    let captures = ctx
        .host
        .language()
        .parsed_text_captures(token, &language, &query)
        .map_err(generic_err)?;
    Ok(captures_list(&captures))
}

/// `captures` as a list of `(list name start-char end-char)`, `name` one
/// string shared by every entry of that name.
fn captures_list(captures: &Captures) -> SteelVal {
    let names: Vec<SteelVal> = captures
        .names
        .iter()
        .map(|name| SteelVal::StringV(name.as_ref().into()))
        .collect();
    list_of(captures.entries.iter().map(|entry| {
        list_of([
            names[entry.name as usize].clone(),
            SteelVal::IntV(entry.start_char as isize),
            SteelVal::IntV(entry.end_char as isize),
        ])
    }))
}

#[cfg(test)]
mod tests;
