use std::ops::Range;
use std::path::Path;

use hume_platform::path::AbsolutePath;

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
    // Resolved the way `:e` resolves the path it completes; a Windows
    // drive-relative directory cannot be, and lists nothing.
    let Ok(dir) = AbsolutePath::from_typed(Path::new(&*expand_fn(dir_str)), ctx.cwd) else {
        return (arg_start..arg_end, Vec::new());
    };
    let items = hume_platform::path::completion_entries(&dir, file_prefix, dirs_only)
        .into_iter()
        .map(|label| {
            let insert_text = format!("{dir_str}{label}");
            let is_dir = label.ends_with('/');
            let mut item = CompletionItem::plain(label, insert_text);
            // The minibuffer's Enter handler reads this to decide whether
            // the selected candidate is a directory to descend into, rather
            // than naming this source explicitly; any source's item can
            // opt in the same way.
            if is_dir {
                item.kind = Some(lsp_types::CompletionItemKind::FOLDER);
            }
            item
        })
        .collect();
    (arg_start..arg_end, items)
}

#[cfg(test)]
mod tests;
