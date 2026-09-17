use super::{CompletionCtx, CompletionItem};

// ── Command name ──────────────────────────────────────────────────────────────

/// The name this source registers under — referenced by the built-in
/// commands that declare it as their argument completer
/// (`registry/defaults/typed.rs`), so a typo there fails to compile at this
/// constant rather than silently naming a source that doesn't exist.
pub(in crate::editor) const COMMAND_SOURCE: &str = "command";

/// Every registered typed command's canonical name — the completed token is
/// the command name prefix, filtered generically by the session's own
/// `MatchKind::String { case_sensitive: false }` (aliases are excluded —
/// `:` can't dispatch editor commands anyway; see `registry/mod.rs`'s
/// module doc — and only canonical names are offered, so the popup doesn't
/// get cluttered with shorthand like `w` alongside `write`).
pub(in crate::editor) fn complete_command(ctx: &CompletionCtx<'_>) -> Vec<CompletionItem> {
    let mut names: Vec<&str> = ctx.registry.typed_names().collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| CompletionItem::plain(name.to_owned(), name.to_owned(), name.to_owned()))
        .collect()
}

// ── Buffer name ───────────────────────────────────────────────────────────────

pub(in crate::editor) const BUFFER_NAME_SOURCE: &str = "buffer-name";

/// Every open buffer's display name for `:b`.
///
/// Matches on the file basename (or `*scratch*` for unnamed buffers).
/// The `insert_text` is the full canonical `path`, not `display_path` — it
/// feeds straight back into path resolution, which doesn't `~`-expand, so an
/// unambiguous target requires the canonical form.
///
/// When two open buffers share the same basename, a shortened parent-directory
/// suffix is appended to the label (e.g. `foo.rs  (~/a/)`) so the user can
/// distinguish them in the popup without accepting the wrong one.
pub(in crate::editor) fn complete_buffer_name(ctx: &CompletionCtx<'_>) -> Vec<CompletionItem> {
    // (display-basename, full-path insert text).
    let entry_for = |buf: &crate::editor::buffer::Buffer| -> (String, String) {
        let base = buf.display_name();
        let insert_text = buf
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| base.clone());
        (base, insert_text)
    };

    // Count how many open buffers share each basename (for disambiguation).
    let mut name_count: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
    for (_, buf) in ctx.buffers.iter() {
        let (base, _) = entry_for(buf);
        *name_count.entry(base).or_insert(0) += 1;
    }

    ctx.buffers
        .iter()
        .map(|(_, buf)| {
            let (base, insert_text) = entry_for(buf);
            let label = if *name_count.get(&base).expect("base was counted above") >= 2 {
                // Two or more buffers share this basename — show parent dir,
                // taken from the display-ready path (already `~`-collapsed).
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
            CompletionItem::plain(label.clone(), insert_text, label)
        })
        .collect()
}

// ── Theme name ────────────────────────────────────────────────────────────────

pub(in crate::editor) const THEME_SOURCE: &str = "theme";

/// Every installed theme name for `:theme` — see [`super::theme_name_candidates`]
/// (called here with an empty prefix — an unfiltered universe, narrowed
/// generically by the session's own `MatchKind::String`).
pub(in crate::editor) fn complete_theme(_ctx: &CompletionCtx<'_>) -> Vec<CompletionItem> {
    super::theme_name_candidates("")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
