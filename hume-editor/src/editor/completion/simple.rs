use super::{
    Completer, Completion, CompletionCtx, CompletionResult, arg_prefix, theme_name_candidates,
};

// ── CommandCompleter ──────────────────────────────────────────────────────────

/// Completes command names from the registry.
///
/// The completed token is the command name prefix `input[0..cursor]`.
/// Only canonical names are offered — abbreviated aliases (e.g. `w` for
/// `write`) still dispatch when typed directly, but are omitted from the
/// popup so it doesn't get cluttered with shorthand.
pub(crate) struct CommandCompleter;

impl Completer for CommandCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let prefix = &input[..cursor.min(input.len())];
        let mut candidates: Vec<Completion> = ctx
            .registry
            .names()
            .filter(|name| {
                // `str::get` returns None if `prefix.len()` is off a char boundary or
                // out of range — safe for non-ASCII command/alias names from plugins.
                name.get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
                    && !name.eq_ignore_ascii_case(prefix)
            })
            .map(|name| Completion {
                replacement: name.to_owned(),
                display: name.to_owned(),
            })
            .collect();
        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        candidates.dedup_by(|a, b| a.replacement == b.replacement);
        CompletionResult {
            span_start: 0,
            candidates,
        }
    }
}

// ── BufferNameCompleter ───────────────────────────────────────────────────────

/// Completes open buffer names for `:b`.
///
/// Matches on the file basename (or `*scratch*` for unnamed buffers).
/// The `replacement` is the full canonical path so the command receives an
/// unambiguous target.
///
/// When two open buffers share the same basename, a shortened parent-directory
/// suffix is appended to `display` (e.g. `foo.rs  (~/a/)`) so the user can
/// distinguish them in the popup without accepting the wrong one.
pub(crate) struct BufferNameCompleter;

impl Completer for BufferNameCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let (arg_start, prefix) = arg_prefix(input, cursor);

        // (display-basename, full-path replacement for the command).
        let entry_for = |buf: &crate::editor::buffer::Buffer| -> (String, String) {
            let base = buf.display_name();
            let replacement = buf
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| base.clone());
            (base, replacement)
        };

        // Count how many open buffers share each basename (for disambiguation).
        let mut name_count: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
        for (_, buf) in ctx.buffers.iter() {
            let (base, _) = entry_for(buf);
            *name_count.entry(base).or_insert(0) += 1;
        }

        let mut candidates: Vec<Completion> = ctx
            .buffers
            .iter()
            .filter_map(|(_, buf)| {
                let (base, replacement) = entry_for(buf);
                if !base.starts_with(prefix) {
                    return None;
                }
                let display = if *name_count.get(&base).expect("base was counted above") >= 2 {
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
                Some(Completion {
                    display,
                    replacement,
                })
            })
            .collect();

        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        CompletionResult {
            span_start: arg_start,
            candidates,
        }
    }
}

// ── ThemeCompleter ────────────────────────────────────────────────────────────

/// Completes theme names for `:theme`.
///
/// Scans `<config_dir>/themes/*.toml` and `<runtime_dir>/themes/*.toml`,
/// strips the `.toml` extension, deduplicates (user theme wins over bundled),
/// and filters by the current prefix.
pub(crate) struct ThemeCompleter;

impl Completer for ThemeCompleter {
    fn complete(&self, input: &str, cursor: usize, _ctx: &CompletionCtx<'_>) -> CompletionResult {
        let (arg_start, prefix) = arg_prefix(input, cursor);
        let mut candidates = theme_name_candidates(prefix);
        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        CompletionResult {
            span_start: arg_start,
            candidates,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
