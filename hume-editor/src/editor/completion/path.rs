use std::ops::Range;
use std::path::{Path, PathBuf};

use super::{CompletionCtx, CompletionItem, arg_prefix, token_end_at};

// ── Filesystem path ───────────────────────────────────────────────────────────

/// The name this source registers under for file/directory completion
/// (`:e`/`:w`); `PATH_DIRS_ONLY_SOURCE` is the directory-only variant
/// (`:cd`) — two names, not one name plus a hidden config, since the
/// registry's native-fn entries carry no parameters of their own.
pub(in crate::editor) const PATH_SOURCE: &str = "path";
pub(in crate::editor) const PATH_DIRS_ONLY_SOURCE: &str = "path-dirs-only";

/// Completes filesystem paths for `:e`/`:w` — files and directories alike.
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

/// `:cd`'s variant — non-directory entries are filtered out.
pub(super) fn complete_path_dirs_only(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    complete_path_with_expand(input, cursor, ctx, true, hume_platform::path::expand)
}

/// Testable core of [`complete_path`]/[`complete_path_dirs_only`].
///
/// Splits the arg into a directory prefix and a filename prefix.  Reads the
/// directory and filters by the filename prefix.  Directory entries get a
/// trailing `/` in both the label and the insert text.  Hidden files
/// (leading `.`) are excluded unless the filename prefix itself starts with
/// `.`.
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

    // Split prefix into (dir_str, file_prefix).
    let (dir_str, file_prefix) = hume_platform::path::split_path_at_sep(prefix);

    // Expand `~` and env vars for the directory lookup only; the literal
    // `dir_str` is still used in the insert text below so `~/` is preserved
    // in the minibuffer exactly as the user typed it.
    let expanded_dir = expand_fn(dir_str);

    // Resolve the directory: absolute if it starts with '/', else relative to cwd.
    let dir: PathBuf = if expanded_dir.is_empty() {
        ctx.cwd.to_owned()
    } else if Path::new(expanded_dir.as_ref()).is_absolute() {
        PathBuf::from(expanded_dir.as_ref())
    } else {
        ctx.cwd.join(expanded_dir.as_ref())
    };

    let include_hidden = file_prefix.starts_with('.');

    // On error (dir doesn't exist or no permission), return no
    // candidates — not a hard error.
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return (arg_start..arg_end, Vec::new()),
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
            // Build the full insert text: dir_str + name + suffix.
            let insert_text = format!("{dir_str}{name}{suffix}");
            // Empty sort_text: a `Delegated` source's own order is what the
            // session's rank key preserves (see `MatchKind::Delegated`'s
            // doc) — sorted right here, not left to that tiebreak.
            Some(CompletionItem::plain(label, insert_text, String::new()))
        })
        .collect();
    candidates.sort_unstable_by(|a, b| a.label.cmp(&b.label));

    (arg_start..arg_end, candidates)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
