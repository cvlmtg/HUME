use std::ops::Range;
use std::path::{Path, PathBuf};

use super::{CompletionCtx, CompletionItem, arg_prefix, token_end_at};

// ── Filesystem path ───────────────────────────────────────────────────────────

/// The name this source registers under for file/directory completion
/// (`:e`/`:w`); `PATH_DIRS_ONLY_SOURCE` is the directory-only variant
/// (`:cd`): two names, not one name plus a hidden config, since the
/// registry's native-fn entries carry no parameters of their own.
pub(in crate::editor) const PATH_SOURCE: &str = "path";
pub(in crate::editor) const PATH_DIRS_ONLY_SOURCE: &str = "path-dirs-only";

/// Completes filesystem paths for `:e`/`:w`: files and directories alike.
/// `Delegated`: the candidate universe (a directory's own listing) depends
/// entirely on the live input, so this takes it directly rather than
/// enumerating a stable universe for the session to filter.
pub(super) fn complete_path(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    complete_path_with_expand(input, cursor, ctx, false, hume_platform::path::expand)
}

/// `:cd`'s variant: non-directory entries are filtered out.
pub(super) fn complete_path_dirs_only(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    complete_path_with_expand(input, cursor, ctx, true, hume_platform::path::expand)
}

/// Testable core of [`complete_path`]/[`complete_path_dirs_only`]: the
/// argument is everything after the command name, split into a directory
/// prefix and a filename prefix, inserted as typed.
///
/// `expand_fn` mirrors `hume_platform::path::expand`: given a raw path
/// string it returns the tilde/env-var expanded form.  Tests pass a stub
/// closure; production calls this with the real `expand`.
fn complete_path_with_expand<F>(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
    dirs_only: bool,
    expand_fn: F,
) -> (Range<usize>, Vec<CompletionItem>)
where
    F: for<'a> Fn(&'a str) -> std::borrow::Cow<'a, str>,
{
    let (arg_start, prefix) = arg_prefix(input, cursor);
    let arg_end = token_end_at(input, cursor, &[' ']);
    let (dir_str, file_prefix) = hume_platform::path::split_path_at_sep(prefix);
    let items = path_candidates(
        dir_str,
        file_prefix,
        ctx.cwd,
        dirs_only,
        expand_fn,
        |name| format!("{dir_str}{name}"),
    );
    (arg_start..arg_end, items)
}

/// The entries of directory `dir` (relative to `cwd` unless absolute,
/// `~` and env vars expanded by `expand_fn`) whose names start with
/// `file_prefix`, sorted by label. Directories get a trailing `/` in both
/// label and insert text. Hidden entries (leading `.`) are left out unless
/// `file_prefix` itself starts with `.`. `insert` turns a name (with its
/// `/`) into the text that replaces the completed token; the label is the
/// name alone. An unreadable directory lists nothing.
pub(super) fn path_candidates<F>(
    dir: &str,
    file_prefix: &str,
    cwd: &Path,
    dirs_only: bool,
    expand_fn: F,
    insert: impl Fn(&str) -> String,
) -> Vec<CompletionItem>
where
    F: for<'a> Fn(&'a str) -> std::borrow::Cow<'a, str>,
{
    let expanded_dir = expand_fn(dir);
    let dir: PathBuf = if expanded_dir.is_empty() {
        cwd.to_owned()
    } else if Path::new(expanded_dir.as_ref()).is_absolute() {
        PathBuf::from(expanded_dir.as_ref())
    } else {
        cwd.join(expanded_dir.as_ref())
    };

    let include_hidden = file_prefix.starts_with('.');

    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut candidates: Vec<CompletionItem> = rd
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(file_prefix) {
                return None;
            }
            if !include_hidden && name.starts_with('.') {
                return None;
            }
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if dirs_only && !is_dir {
                return None;
            }
            let suffix = if is_dir { "/" } else { "" };
            let label = format!("{name}{suffix}");
            let insert_text = insert(&label);
            let mut item = CompletionItem::plain(label, insert_text);
            // The minibuffer's Enter handler reads this to decide whether
            // the selected candidate is a directory to descend into, rather
            // than naming this source explicitly; any source's item can
            // opt in the same way.
            if is_dir {
                item.kind = Some(lsp_types::CompletionItemKind::FOLDER);
            }
            Some(item)
        })
        .collect();
    candidates.sort_unstable_by(|a, b| a.label.cmp(&b.label));
    candidates
}

#[cfg(test)]
mod tests;
