use std::ops::Range;

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
    complete_path_with(input, cursor, ctx, false, AbsolutePath::from_typed)
}

/// `:cd`'s variant: non-directory entries are filtered out.
pub(super) fn complete_path_dirs_only(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    complete_path_with(input, cursor, ctx, true, AbsolutePath::from_typed)
}

/// Testable core of [`complete_path`]/[`complete_path_dirs_only`]: the
/// argument is everything after the command name, split into a directory
/// prefix and a filename prefix, inserted as typed.
///
/// `resolve_dir` makes the typed directory prefix absolute against the cwd:
/// production passes [`AbsolutePath::from_typed`], tests a stub that expands
/// a fake `~` or variable.
fn complete_path_with<F>(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
    dirs_only: bool,
    resolve_dir: F,
) -> (Range<usize>, Vec<CompletionItem>)
where
    F: Fn(&str, &AbsolutePath) -> std::io::Result<AbsolutePath>,
{
    let (arg_start, prefix) = arg_prefix(input, cursor);
    let arg_end = token_end_at(input, cursor, &[' ']);
    let (dir_str, file_prefix) = hume_platform::path::split_path_at_sep(prefix);
    // Resolved the way `:e` resolves the path it completes; a Windows
    // drive-relative directory cannot be, and lists nothing.
    let Ok(dir) = resolve_dir(dir_str, ctx.cwd) else {
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
