use super::{CompletionCtx, CompletionItem};

// ── Command name ──────────────────────────────────────────────────────────────

/// The source the `:` prompt completes a command name from.
pub(in crate::editor) const COMMAND_SOURCE: &str = "command";

/// Every typed command's canonical name, filtered case-insensitively by the
/// session. Aliases are left out so `w` does not clutter the list beside
/// `write`.
pub(super) fn complete_command(ctx: &CompletionCtx<'_>) -> Vec<CompletionItem> {
    let mut names: Vec<&str> = ctx.registry.typed_names().collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| CompletionItem::plain(name.to_owned(), name.to_owned()))
        .collect()
}

// ── Buffer name ───────────────────────────────────────────────────────────────

pub(in crate::editor) const BUFFER_NAME_SOURCE: &str = "buffer-name";

/// Every open buffer's display name for `:b`.
///
/// The inserted text is the full `path`, not `display_path`: path
/// resolution does not `~`-expand. Buffers sharing a basename get their
/// parent directory appended to the label (`foo.rs  (~/a/)`).
pub(super) fn complete_buffer_name(ctx: &CompletionCtx<'_>) -> Vec<CompletionItem> {
    let mut name_count: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
    for (_, buf) in ctx.buffers.iter() {
        *name_count.entry(buf.display_name()).or_insert(0) += 1;
    }

    ctx.buffers
        .iter()
        .map(|(_, buf)| {
            let base = buf.display_name();
            let insert_text = buf
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| base.clone());
            let label = if *name_count.get(&base).expect("base was counted above") >= 2 {
                let dir = buf
                    .display_path()
                    .map(|p| hume_platform::path::split_path_at_sep(p).0);
                match dir {
                    Some(dir) if !dir.is_empty() => format!("{base}  ({dir})"),
                    _ => base, // scratch can't collide
                }
            } else {
                base
            };
            CompletionItem::plain(label, insert_text)
        })
        .collect()
}

// ── Theme name ────────────────────────────────────────────────────────────────

pub(in crate::editor) const THEME_SOURCE: &str = "theme";

/// Every installed theme name for `:theme`, filtered by the session.
pub(super) fn complete_theme(ctx: &CompletionCtx<'_>) -> Vec<CompletionItem> {
    super::theme_name_candidates(ctx.dirs)
        .into_iter()
        .map(|stem| CompletionItem::plain(stem.clone(), stem))
        .collect()
}

#[cfg(test)]
mod tests;
