//! Minibuffer tab-completion — completers, types, and dispatch helpers.
//!
//! Design contract:
//! - `Completer::complete` is a pure function: given `(input, cursor, ctx)` it
//!   returns a sorted `Vec<Completion>` and the byte offset in `input` at which
//!   the completed token starts (`span_start`).  No &mut access, no I/O side
//!   effects visible to the caller.
//! - `CompletionState` on `Editor` is the SSOT.  It is cleared whenever the
//!   minibuffer closes or the user edits the input by any key other than Tab /
//!   Shift-Tab.

use std::path::{Path, PathBuf};

use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceRender, WrapMode};

use crate::editor::buffer_store::BufferStore;
use crate::editor::registry::CommandRegistry;
use crate::editor::syntax::LanguageRegistry;
use crate::settings::{TabStyle, all_setting_keys, setting_scopes};

// ── Public types ──────────────────────────────────────────────────────────────

/// A single completion candidate.
///
/// `display` is shown in the popup (may include decorators like trailing `/`
/// for directories). `replacement` is the text written into the minibuffer.
/// The two fields are often identical; they differ for e.g. buffer names where
/// the display is the basename but the replacement is the full path.
#[derive(Debug, Clone)]
pub(crate) struct Completion {
    /// Text to insert at the span location in the minibuffer input.
    pub replacement: String,
    /// Text shown in the completion popup row.
    pub display: String,
}

/// Completion session state, stored on `Editor` while a popup is open.
///
/// Invariant: `selected < candidates.len()`. Created only when there are ≥2
/// candidates (single-candidate completion is applied silently without state).
pub(crate) struct CompletionState {
    pub candidates: Vec<Completion>,
    /// Index of the currently-displayed candidate.
    pub selected: usize,
    /// Byte offset in the minibuffer input where the completed token starts.
    /// Constant across the session (the span start never shifts while cycling).
    pub span_start: usize,
}

impl CompletionState {
    /// The byte range that the current replacement occupies in the input.
    pub(crate) fn current_span(&self) -> std::ops::Range<usize> {
        debug_assert!(
            self.selected < self.candidates.len(),
            "CompletionState invariant violated: selected {} >= len {}",
            self.selected,
            self.candidates.len(),
        );
        let end = self.span_start + self.candidates[self.selected].replacement.len();
        self.span_start..end
    }
}

/// Context supplied to every `Completer::complete` call.
///
/// Bundles read-only references to the editor state that completers need
/// (command registry, buffer list, working directory) without exposing a full
/// `&Editor`.  This makes unit-testing completers straightforward — no Editor
/// construction required.
pub(crate) struct CompletionCtx<'a> {
    pub registry: &'a CommandRegistry,
    pub buffers: &'a BufferStore,
    pub cwd: &'a Path,
    pub languages: &'a LanguageRegistry,
}

/// Result of a single `Completer::complete` call.
///
/// `span_start` is the byte offset in `input` where the completed token
/// begins.  All candidates are replacements for `input[span_start..cursor]`.
pub(crate) struct CompletionResult {
    pub span_start: usize,
    pub candidates: Vec<Completion>,
}

/// A completion source for a specific context (command name, path, buffer name).
pub(crate) trait Completer {
    /// Return sorted candidates for the token at `cursor` in `input`.
    ///
    /// Returns `span_start` (the byte offset where the completed token begins)
    /// alongside the candidates.  Returns an empty `Vec` when there are no
    /// matches.
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult;
}

// ── CommandCompleter ──────────────────────────────────────────────────────────

/// Completes command names + aliases from the registry.
///
/// The completed token is the command name prefix `input[0..cursor]`.
/// Both canonical names and aliases are offered as candidates so the user
/// can discover either form.
pub(crate) struct CommandCompleter;

impl Completer for CommandCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let prefix = &input[..cursor.min(input.len())];
        let mut candidates: Vec<Completion> = ctx
            .registry
            .iter_names_and_aliases()
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
        let mut name_count: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
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
                    // Two or more buffers share this basename — show parent dir.
                    if let Some(parent) = buf.path().and_then(|p| p.parent()) {
                        format!("{base}  ({}/)", hume_platform::path::shorten_home(parent))
                    } else {
                        base // scratch can't collide
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

// ── PathCompleter ─────────────────────────────────────────────────────────────

/// Completes filesystem paths for `:e` / `:w` / `:cd`.
///
/// Splits the arg into a directory prefix and a filename prefix.  Reads the
/// directory and filters by the filename prefix.  Directory entries get a
/// trailing `/` in both `display` and `replacement`.  Hidden files (leading
/// `.`) are excluded unless the filename prefix itself starts with `.`.
///
/// When `dirs_only` is `true` (used by `:cd`), non-directory entries are
/// filtered out.
pub(crate) struct PathCompleter {
    pub(crate) dirs_only: bool,
}

impl PathCompleter {
    /// Testable core of [`Completer::complete`].
    ///
    /// `expand_fn` mirrors `hume_platform::path::expand`: given a raw path string it
    /// returns the tilde / env-var expanded form.  Tests pass a stub closure;
    /// production calls this with the real `expand`.
    fn complete_with_expand<F>(
        &self,
        input: &str,
        cursor: usize,
        ctx: &CompletionCtx<'_>,
        expand_fn: F,
    ) -> CompletionResult
    where
        F: for<'a> Fn(&'a str) -> std::borrow::Cow<'a, str>,
    {
        let (arg_start, prefix) = arg_prefix(input, cursor);

        // Split prefix into (dir_str, file_prefix).
        let (dir_str, file_prefix) = hume_platform::path::split_path_at_sep(prefix);

        // Expand `~` and env vars for the directory lookup only; the literal
        // `dir_str` is still used in `replacement` below so `~/` is preserved
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

        // `hume_platform::fs::read_dir` wraps std::fs::read_dir.  On error (dir
        // doesn't exist or no permission), return no candidates — not a hard error.
        let rd = match hume_platform::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                return CompletionResult {
                    span_start: arg_start,
                    candidates: vec![],
                };
            }
        };

        let mut candidates: Vec<Completion> = rd
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
                if self.dirs_only && !is_dir {
                    return None;
                }
                let suffix = if is_dir { "/" } else { "" };
                let display = format!("{name}{suffix}");
                // Build the full replacement: dir_str + name + suffix.
                let replacement = format!("{dir_str}{name}{suffix}");
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

impl Completer for PathCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        self.complete_with_expand(input, cursor, ctx, hume_platform::path::expand)
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

/// Scan `themes/*.toml` in every search path and return the stems that start
/// with `prefix` (excluding an exact match, so Tab on a fully-typed theme name
/// is a no-op rather than re-offering it). User themes (earlier in the search
/// path list) shadow bundled themes with the same stem.
///
/// Shared by `:theme` (via [`ThemeCompleter`]) and `:set global theme=` (via
/// [`SetCompleter`]) so the candidate set stays in sync.
fn theme_name_candidates(prefix: &str) -> Vec<Completion> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut candidates: Vec<Completion> = Vec::new();

    for dir in &super::theme_search_paths() {
        let entries = match hume_platform::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // User themes (earlier in search_dirs) shadow bundled themes.
            if !seen.insert(stem.to_owned()) {
                continue;
            }
            if stem.starts_with(prefix) && stem != prefix {
                candidates.push(Completion {
                    replacement: stem.to_owned(),
                    display: stem.to_owned(),
                });
            }
        }
    }

    candidates
}

// ── SetCompleter ──────────────────────────────────────────────────────────────

/// Completes `:set <scope> <key>=<value>` arguments.
///
/// Three phases, selected by cursor position within the argument:
/// - **scope** (no space yet) — offers `global`/`buffer`/`pane`.
/// - **key** (space present, no `=` yet) — offers every setting key whose
///   declared scopes include the chosen scope, plus `language` for `buffer`.
/// - **value** (`=` present) — offers the valid value set for enum/bool keys,
///   registered language names for `language`, installed theme names for
///   `theme`. Numeric/free-form keys (e.g. `scrolloff`, `statusline`) get no
///   candidates — the user types them and `apply_setting` validates.
///
/// Value lists are completion *hints* mirrored from each setting's parser;
/// `apply_setting` remains the validation SSOT, so the two can drift only in
/// what's offered, never in what's accepted.
pub(crate) struct SetCompleter;

/// The three `:set` scopes. `pane` exists only because `wrap-mode` declares it.
const SET_SCOPES: &[&str] = &["global", "buffer", "pane"];

/// Prefix-filter `items`, dropping an exact match (Tab on a fully-typed value
/// is a no-op), and wrap each into a `Completion`. Caller sorts.
fn prefix_completions<'a>(items: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<Completion> {
    items
        .filter(|s| s.starts_with(prefix) && *s != prefix)
        .map(|s| Completion {
            replacement: s.to_owned(),
            display: s.to_owned(),
        })
        .collect()
}

/// Static value candidates for enum/bool keys. Returns `None` for keys whose
/// values are dynamic (`language`, `theme`) or free-form (numbers,
/// `statusline`) — those are handled in [`SetCompleter::complete`].
fn static_value_candidates(key: &str) -> Option<&'static [&'static str]> {
    // Bool keys are derived from `define_settings!`'s `parser: bool` — not
    // hand-listed — so a new bool setting gets value completion for free.
    if crate::settings::is_bool_setting(key) {
        return Some(&["true", "false"]);
    }
    Some(match key {
        "tab-style" => TabStyle::VALUES,
        "line-number-style" => LineNumberStyle::VALUES,
        "wrap-mode" => WrapMode::VALUES,
        "whitespace-space" | "whitespace-tab" | "whitespace-newline" => WhitespaceRender::VALUES,
        _ => return None,
    })
}

/// Phase 1: completing the scope token (`global`/`buffer`/`pane`).
fn complete_set_scope(prefix: &str, span_start: usize) -> CompletionResult {
    let mut candidates = prefix_completions(SET_SCOPES.iter().copied(), prefix);
    candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
    CompletionResult {
        span_start,
        candidates,
    }
}

/// Phase 2: completing the key. Surface every declared key whose scopes
/// include `scope`; `language` is the one key with no macro entry — valid
/// only for buffer, so it's chained in when the scope matches.
fn complete_set_key(scope: &str, rest: &str, span_start: usize) -> CompletionResult {
    let scope_keys = all_setting_keys()
        .iter()
        .copied()
        .filter(|k| setting_scopes(k).contains(&scope));
    let language = (scope == "buffer").then_some("language");
    let mut candidates = prefix_completions(scope_keys.chain(language), rest);
    candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
    CompletionResult {
        span_start,
        candidates,
    }
}

/// Phase 3: completing the value. Static enum/bool lists come from
/// [`static_value_candidates`]; `language` and `theme` are dynamic.
///
/// Every key checks its scope before offering values — the same gate
/// `typed_set` enforces at execution time — so e.g. `:set pane tab-style=`
/// (tab-style isn't pane-scoped) never dangles a completion that would error
/// on Enter.
fn complete_set_value(
    scope: &str,
    key: &str,
    value_prefix: &str,
    span_start: usize,
    ctx: &CompletionCtx<'_>,
) -> CompletionResult {
    // `language` has no `setting_scopes` entry by design (see settings.rs) —
    // valid only for buffer scope, checked directly instead of through the
    // generic gate below.
    let mut candidates = if key == "language" {
        if scope == "buffer" {
            prefix_completions(ctx.languages.iter_names(), value_prefix)
        } else {
            Vec::new()
        }
    } else if !setting_scopes(key).contains(&scope) {
        Vec::new()
    } else if let Some(values) = static_value_candidates(key) {
        prefix_completions(values.iter().copied(), value_prefix)
    } else if key == "theme" {
        theme_name_candidates(value_prefix)
    } else {
        Vec::new()
    };
    candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
    CompletionResult {
        span_start,
        candidates,
    }
}

impl Completer for SetCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let up_to = &input[..cursor.min(input.len())];
        // Argument region begins after the command word ("set ").
        let Some(arg_start) = up_to.find(' ').map(|i| i + 1) else {
            return CompletionResult {
                span_start: up_to.len(),
                candidates: Vec::new(),
            };
        };
        let arg = up_to[arg_start..].trim_start();

        match arg.split_once(' ') {
            None => {
                // Scope token: bounded by whitespace only — no '=' can occur
                // yet, so the last space before the cursor is always correct,
                // robust to stray extra whitespace.
                let span_start = up_to.rfind(' ').map_or(0, |i| i + 1);
                complete_set_scope(arg, span_start)
            }
            Some((scope, rest)) => {
                let rest = rest.trim_start();
                match rest.split_once('=') {
                    None => {
                        // Key token: same reasoning as the scope case.
                        let span_start = up_to.rfind(' ').map_or(0, |i| i + 1);
                        complete_set_key(scope, rest, span_start)
                    }
                    Some((key, value)) => {
                        // Value token: bounded by '=' only, never by internal
                        // whitespace — a value can legitimately contain spaces
                        // (e.g. a theme filename stem like "my theme"), and
                        // replacing from the last *space* would drop
                        // everything before it instead of the whole value.
                        let span_start = up_to.rfind('=').map_or(0, |i| i + 1);
                        complete_set_value(scope, key, value, span_start, ctx)
                    }
                }
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the argument prefix for commands that take a single argument.
///
/// Splits `input[..cursor]` on the first space.  Returns `(arg_start, prefix)`
/// where `arg_start` is the byte offset of the argument in `input` and
/// `prefix` is the unfinished argument text up to the cursor.
///
/// If there is no space (command-only input), returns `(0, input[..cursor])`.
fn arg_prefix(input: &str, cursor: usize) -> (usize, &str) {
    let up_to_cursor = &input[..cursor.min(input.len())];
    match up_to_cursor.find(' ') {
        Some(space_idx) => (space_idx + 1, &up_to_cursor[space_idx + 1..]),
        None => (0, up_to_cursor),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::editor::buffer::Buffer;
    use crate::editor::buffer_store::BufferStore;
    use crate::editor::registry::CommandRegistry;
    use hume_editing::selection::SelectionSet;
    use hume_editing::text::Text;
    use hume_engine::pipeline::{BufferId, EngineView, SharedBuffer};
    use hume_engine::theme::Theme;

    use super::*;

    fn make_ctx_parts() -> (CommandRegistry, BufferStore, TempDir) {
        let reg = CommandRegistry::with_defaults();
        let store = BufferStore::new();
        let dir = tempfile::tempdir().unwrap();
        (reg, store, dir)
    }

    fn ctx<'a>(
        registry: &'a CommandRegistry,
        buffers: &'a BufferStore,
        cwd: &'a Path,
    ) -> CompletionCtx<'a> {
        ctx_with(registry, buffers, cwd, empty_langs())
    }

    fn ctx_with<'a>(
        registry: &'a CommandRegistry,
        buffers: &'a BufferStore,
        cwd: &'a Path,
        languages: &'a LanguageRegistry,
    ) -> CompletionCtx<'a> {
        CompletionCtx {
            registry,
            buffers,
            cwd,
            languages,
        }
    }

    /// Shared empty registry for tests that don't register languages — avoids
    /// re-allocating one per `ctx()` call and sidesteps the borrow-lifetime
    /// issue of constructing it inline.
    fn empty_langs() -> &'static LanguageRegistry {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<LanguageRegistry> = OnceLock::new();
        EMPTY.get_or_init(LanguageRegistry::new)
    }

    fn ev() -> EngineView {
        EngineView::new(Theme::default())
    }

    fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(SharedBuffer::new())
    }

    fn make_buf() -> Buffer {
        Buffer::new(Text::from("a\n"), SelectionSet::default())
    }

    fn buf_with_path(path: &str) -> Buffer {
        let mut b = make_buf();
        b.set_path(Some(PathBuf::from(path)));
        b
    }

    // ── CommandCompleter ──────────────────────────────────────────────────────

    #[test]
    fn command_completer_empty_prefix_returns_all() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("", 0, &ctx);
        // All registered names (canonicals + aliases) minus empty prefix match all.
        assert!(!result.candidates.is_empty());
        assert_eq!(result.span_start, 0);
    }

    #[test]
    fn command_completer_prefix_filters() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("q", 1, &ctx);
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.replacement.starts_with('q'))
        );
        assert!(result.candidates.iter().any(|c| c.replacement == "quit"));
    }

    #[test]
    fn command_completer_no_match_returns_empty() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("zzz", 3, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn command_completer_sorted_ascending() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("w", 1, &ctx);
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn command_completer_alias_and_canonical_both_appear() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        // Typing "wr" matches "write" (canonical) and "write-quit" (canonical);
        // "w" (alias) is excluded because it doesn't start with "wr".
        // This verifies that both alias forms and canonical forms of other commands
        // starting with the same prefix are surfaced.
        let result = CommandCompleter.complete("wr", 2, &ctx);
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect();
        assert!(names.contains(&"write"), "canonical 'write' should appear");
        assert!(
            names.contains(&"write-quit"),
            "canonical 'write-quit' should appear"
        );
        // Verify aliases also surface: "wq" is an alias, starts with "w" not "wr".
        let result2 = CommandCompleter.complete("w", 1, &ctx);
        let names2: Vec<&str> = result2
            .candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect();
        assert!(
            names2.contains(&"write"),
            "canonical 'write' should appear with prefix 'w'"
        );
        assert!(
            names2.contains(&"wq"),
            "'wq' alias should appear with prefix 'w'"
        );
    }

    #[test]
    fn command_completer_exact_prefix_not_included() {
        // Typing the exact name should not complete to itself.
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("quit", 4, &ctx);
        assert!(!result.candidates.iter().any(|c| c.replacement == "quit"));
    }

    #[test]
    fn command_completer_non_ascii_name_does_not_panic() {
        // Regression: the old `name[..prefix.len()]` byte-slice would panic when
        // `prefix.len()` lands mid-codepoint in a non-ASCII command/alias name.
        // Here "naïve-cmd" has 'ï' at bytes 2-3; a 1-byte prefix "n" must not panic.
        use std::borrow::Cow;
        let mut reg = CommandRegistry::with_defaults();
        fn noop(
            _ed: &mut crate::editor::Editor,
            _arg: Option<&str>,
            _force: bool,
        ) -> Result<(), crate::editor::error::CommandError> {
            Ok(())
        }
        reg.register_typed(crate::editor::registry::TypedCommand {
            name: Cow::Borrowed("naïve-cmd"),
            doc: Cow::Borrowed(""),
            aliases: &[],
            fun: noop,
        });
        let store = crate::editor::buffer_store::BufferStore::new();
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&reg, &store, dir.path());
        // "n" has byte-length 1; "ï" at bytes 2-3 means name[..1] would panic.
        // Must not panic and must return the non-ASCII command as a candidate.
        let result = CommandCompleter.complete("n", 1, &ctx);
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "naïve-cmd"),
            "non-ASCII command must appear in completions"
        );
    }

    // ── BufferNameCompleter ───────────────────────────────────────────────────

    #[test]
    fn buffer_name_completer_matches_basename() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id = make_id(&mut ev);
        store.open(id, buf_with_path("/tmp/foo.txt"));
        let ctx = ctx(&reg, &store, dir.path());
        let result = BufferNameCompleter.complete("bd f", 4, &ctx);
        assert_eq!(result.span_start, 3);
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/tmp/foo.txt")
        );
    }

    #[test]
    fn buffer_name_completer_scratch_buffer() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id = make_id(&mut ev);
        store.open(id, make_buf()); // no path → scratch
        let ctx = ctx(&reg, &store, dir.path());
        let result = BufferNameCompleter.complete("bd *", 4, &ctx);
        assert_eq!(result.span_start, 3);
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "*scratch*")
        );
    }

    #[test]
    fn buffer_name_completer_no_match() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id = make_id(&mut ev);
        store.open(id, buf_with_path("/tmp/foo.txt"));
        let ctx = ctx(&reg, &store, dir.path());
        let result = BufferNameCompleter.complete("bd z", 4, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn buffer_name_completer_duplicate_basename_adds_parent_suffix() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id1 = make_id(&mut ev);
        let id2 = make_id(&mut ev);
        let id3 = make_id(&mut ev);
        store.open(id1, buf_with_path("/a/foo.txt"));
        store.open(id2, buf_with_path("/b/foo.txt"));
        store.open(id3, buf_with_path("/tmp/bar.txt"));
        let ctx = ctx(&reg, &store, dir.path());

        let result = BufferNameCompleter.complete("b ", 2, &ctx);
        // All three buffers should appear (prefix "" matches all).
        assert_eq!(result.candidates.len(), 3);

        // The two foo.txt entries must have parent-dir suffixes in their display.
        let foo_entries: Vec<&str> = result
            .candidates
            .iter()
            .filter(|c| c.display.contains("foo.txt"))
            .map(|c| c.display.as_str())
            .collect();
        assert_eq!(foo_entries.len(), 2, "both foo.txt entries must appear");
        assert!(
            foo_entries.iter().all(|d| d.contains('(')),
            "duplicate basenames must include a parent-dir suffix: {foo_entries:?}"
        );

        // The unique bar.txt entry must NOT have a suffix.
        let bar_entry = result
            .candidates
            .iter()
            .find(|c| c.display.contains("bar.txt"))
            .expect("bar.txt must appear");
        assert!(
            !bar_entry.display.contains('('),
            "unique basename must not have a suffix: {}",
            bar_entry.display
        );

        // Replacements are always the full paths.
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/a/foo.txt")
        );
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/b/foo.txt")
        );
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/tmp/bar.txt")
        );
    }

    // ── PathCompleter ─────────────────────────────────────────────────────────

    #[test]
    fn path_completer_lists_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), b"").unwrap();
        std::fs::write(dir.path().join("beta.txt"), b"").unwrap();
        std::fs::create_dir(dir.path().join("gamma")).unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let input = "e ";
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);

        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert!(names.contains(&"alpha.txt"), "alpha.txt should appear");
        assert!(names.contains(&"beta.txt"), "beta.txt should appear");
        assert!(names.contains(&"gamma/"), "directory gets trailing /");
        assert_eq!(result.span_start, 2);
    }

    #[test]
    fn path_completer_filters_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.txt"), b"").unwrap();
        std::fs::write(dir.path().join("bar.txt"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let input = "e foo";
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);

        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].replacement, "foo.txt");
    }

    #[test]
    fn path_completer_excludes_hidden_unless_dot_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".hidden"), b"").unwrap();
        std::fs::write(dir.path().join("visible"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());

        // Without dot prefix: hidden excluded.
        let result = PathCompleter { dirs_only: false }.complete("e ", 2, &ctx);
        assert!(!result.candidates.iter().any(|c| c.display.starts_with('.')));
        assert!(result.candidates.iter().any(|c| c.display == "visible"));

        // With dot prefix: hidden included.
        let input = "e .";
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);
        assert!(result.candidates.iter().any(|c| c.display == ".hidden"));
    }

    #[test]
    fn path_completer_multi_segment() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.rs"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());

        // Completing "sub/f" — should find "sub/file.rs".
        let input = "e sub/f";
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].replacement, "sub/file.rs");
    }

    #[test]
    fn path_completer_missing_dir_returns_empty() {
        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/nonexistent/path/that/does/not/exist");
        let langs = LanguageRegistry::new();
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
            languages: &langs,
        };
        let result = PathCompleter { dirs_only: false }.complete("e foo", 5, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn path_completer_sorted_ascending() {
        let dir = tempfile::tempdir().unwrap();
        for name in &["zz.txt", "aa.txt", "mm.txt"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let result = PathCompleter { dirs_only: false }.complete("e ", 2, &ctx);
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "results must be sorted alphabetically");
    }

    #[test]
    #[cfg(not(windows))]
    fn path_completer_tilde_expands_for_lookup_keeps_literal_replacement() {
        use std::borrow::Cow;

        let home_dir = tempfile::tempdir().unwrap();
        std::fs::write(home_dir.path().join("notes.md"), b"").unwrap();
        std::fs::create_dir(home_dir.path().join("code")).unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/tmp");
        let langs = LanguageRegistry::new();
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
            languages: &langs,
        };

        let home = home_dir.path().to_path_buf();
        let input = "e ~/";
        let result = PathCompleter { dirs_only: false }.complete_with_expand(
            input,
            input.len(),
            &ctx,
            |s: &str| {
                if let Some(tail) = s.strip_prefix('~')
                    && (tail.is_empty() || tail.starts_with('/'))
                {
                    return Cow::Owned(format!("{}{tail}", home.display()));
                }
                Cow::Borrowed(s)
            },
        );

        // Candidates must be present (the temp home has files).
        assert!(
            !result.candidates.is_empty(),
            "tilde should resolve to home and list entries"
        );
        // Replacements must keep the literal `~/` prefix, not expand to the absolute path.
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.replacement.starts_with("~/")),
            "replacements must preserve the `~/` prefix"
        );
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert!(names.contains(&"notes.md"), "notes.md should appear");
        assert!(
            names.contains(&"code/"),
            "code/ directory should appear with trailing /"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn path_completer_dollar_var_expands_for_lookup() {
        use std::borrow::Cow;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/tmp");
        let langs = LanguageRegistry::new();
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
            languages: &langs,
        };

        let expanded = dir.path().to_string_lossy().into_owned();
        let input = "e $MYDIR/";
        let result = PathCompleter { dirs_only: false }.complete_with_expand(
            input,
            input.len(),
            &ctx,
            |s: &str| {
                if let Some(rest) = s.strip_prefix("$MYDIR") {
                    Cow::Owned(format!("{expanded}{rest}"))
                } else {
                    Cow::Borrowed(s)
                }
            },
        );

        assert!(
            !result.candidates.is_empty(),
            "$MYDIR should expand and list entries"
        );
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.replacement.starts_with("$MYDIR/"))
        );
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert!(names.contains(&"main.rs"));
    }

    // ── SetCompleter: scope phase ─────────────────────────────────────────────

    fn set_result(input: &str) -> CompletionResult {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        SetCompleter.complete(input, input.len(), &ctx)
    }

    fn names_of(result: &CompletionResult) -> Vec<&str> {
        result
            .candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect()
    }

    #[test]
    fn set_completer_scope_empty_prefix_lists_all_scopes() {
        let result = set_result("set ");
        assert_eq!(result.span_start, 4);
        assert_eq!(names_of(&result), vec!["buffer", "global", "pane"]);
    }

    #[test]
    fn set_completer_scope_prefix_filters() {
        let result = set_result("set g");
        assert_eq!(result.span_start, 4);
        assert_eq!(names_of(&result), vec!["global"]);
    }

    #[test]
    fn set_completer_scope_exact_match_excluded() {
        let result = set_result("set global");
        assert!(result.candidates.is_empty());
    }

    // ── SetCompleter: key phase ───────────────────────────────────────────────

    #[test]
    fn set_completer_keys_for_global_scope() {
        let result = set_result("set global ");
        assert_eq!(result.span_start, 11);
        let names = names_of(&result);
        assert!(!names.is_empty());
        assert!(
            names.contains(&"scrolloff"),
            "global-only key should appear"
        );
        assert!(
            names.contains(&"tab-width"),
            "global+buffer key should appear"
        );
        assert!(
            names.contains(&"wrap-mode"),
            "global+pane key should appear"
        );
        assert!(names.contains(&"statusline"), "hand-listed global key");
        assert!(!names.contains(&"language"), "language has no global scope");
    }

    #[test]
    fn set_completer_keys_for_buffer_scope_includes_language() {
        let result = set_result("set buffer ");
        assert_eq!(result.span_start, 11);
        let names = names_of(&result);
        assert!(names.contains(&"language"), "language is buffer-only");
        assert!(names.contains(&"tab-width"), "buffer-overridable key");
        assert!(
            !names.contains(&"scrolloff"),
            "global-only key must not appear under buffer scope"
        );
    }

    #[test]
    fn set_completer_keys_for_pane_scope_only_wrap_mode() {
        let result = set_result("set pane ");
        assert_eq!(result.span_start, 9);
        assert_eq!(names_of(&result), vec!["wrap-mode"]);
    }

    #[test]
    fn set_completer_key_prefix_filters() {
        let result = set_result("set global tab");
        assert_eq!(result.span_start, 11);
        let names = names_of(&result);
        assert!(names.contains(&"tab-width"));
        assert!(names.contains(&"tab-style"));
        assert!(names.iter().all(|n| n.starts_with("tab")));
    }

    #[test]
    fn set_completer_key_exact_match_excluded() {
        let result = set_result("set global tab-width");
        assert!(!names_of(&result).contains(&"tab-width"));
    }

    // ── SetCompleter: value phase (static enums / bools) ──────────────────────

    #[test]
    fn set_completer_value_bool_offers_true_false() {
        let result = set_result("set global mouse-enabled=");
        assert_eq!(result.span_start, "set global mouse-enabled=".len());
        assert_eq!(names_of(&result), vec!["false", "true"]);
    }

    #[test]
    fn set_completer_value_tab_style() {
        let result = set_result("set buffer tab-style=");
        assert_eq!(names_of(&result), vec!["hard", "soft"]);
    }

    #[test]
    fn set_completer_value_wrap_mode() {
        let result = set_result("set global wrap-mode=");
        assert_eq!(names_of(&result), vec!["indent", "none", "soft", "word"]);
    }

    #[test]
    fn set_completer_value_whitespace_render() {
        let result = set_result("set buffer whitespace-space=");
        assert_eq!(names_of(&result), vec!["all", "none", "trailing"]);
    }

    #[test]
    fn set_completer_value_prefix_filters() {
        let result = set_result("set buffer tab-style=s");
        assert_eq!(result.span_start, "set buffer tab-style=".len());
        assert_eq!(names_of(&result), vec!["soft"]);
    }

    #[test]
    fn set_completer_value_exact_match_excluded() {
        let result = set_result("set buffer tab-style=hard");
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn set_completer_value_numeric_no_candidates() {
        let result = set_result("set global scrolloff=");
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn set_completer_value_static_enum_rejects_ineligible_scope() {
        // tab-style is global/buffer-scoped, not pane-scoped — completion
        // must not offer values for a scope the key doesn't accept, matching
        // the error `typed_set` would give on Enter.
        let result = set_result("set pane tab-style=");
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn set_completer_value_static_bool_rejects_unknown_scope() {
        let result = set_result("set bogus mouse-enabled=");
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn set_completer_value_span_start_stops_at_equals_not_internal_space() {
        // A value can legitimately contain spaces (e.g. a theme filename stem
        // like "my theme"). The replacement span must start right after '=',
        // not after the last internal space — otherwise completion would
        // replace only the tail after the space and duplicate the rest
        // (e.g. "set global theme=my my theme").
        let result = set_result("set global theme=my theme");
        assert_eq!(result.span_start, "set global theme=".len());
    }

    // ── SetCompleter: stray whitespace robustness ─────────────────────────────
    //
    // A naive first-space split collapses the parsed scope to "" when extra
    // whitespace appears anywhere before the key token (e.g. a double
    // space-bar tap), silently emptying the popup. These pin the fix.

    #[test]
    fn set_completer_double_space_after_set_still_lists_buffer_keys() {
        let result = set_result("set  buffer ");
        let names = names_of(&result);
        assert!(
            names.contains(&"language"),
            "buffer scope should resolve despite double space"
        );
        assert!(names.contains(&"tab-width"));
    }

    #[test]
    fn set_completer_double_space_before_key_still_filters() {
        let result = set_result("set global  tab");
        let names = names_of(&result);
        assert!(
            !names.is_empty(),
            "scope should resolve despite double space"
        );
        assert!(names.iter().all(|n| n.starts_with("tab")));
    }

    #[test]
    fn set_completer_double_space_before_value_still_offers_bools() {
        let result = set_result("set global  mouse-enabled=");
        assert_eq!(names_of(&result), vec!["false", "true"]);
    }

    // ── SetCompleter: value phase (language from registry) ────────────────────

    #[test]
    fn set_completer_value_language_from_registry() {
        let (reg, store, dir) = make_ctx_parts();
        let mut langs = LanguageRegistry::new();
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        langs.register_identity("ruby", &["rb"], &[], &[]).unwrap();
        let ctx = ctx_with(&reg, &store, dir.path(), &langs);
        let result = SetCompleter.complete("set buffer language=", 21, &ctx);
        let names = names_of(&result);
        assert!(names.contains(&"rust"));
        assert!(names.contains(&"ruby"));
        assert_eq!(result.span_start, "set buffer language=".len());
    }

    #[test]
    fn set_completer_value_language_only_buffer_scope() {
        // `:set global language=` is invalid; the completer must not offer
        // language names under a non-buffer scope.
        let (reg, store, dir) = make_ctx_parts();
        let mut langs = LanguageRegistry::new();
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        let ctx = ctx_with(&reg, &store, dir.path(), &langs);
        let result = SetCompleter.complete("set global language=", 21, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn set_completer_value_language_prefix_filters() {
        let (reg, store, dir) = make_ctx_parts();
        let mut langs = LanguageRegistry::new();
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        langs.register_identity("ruby", &["rb"], &[], &[]).unwrap();
        let ctx = ctx_with(&reg, &store, dir.path(), &langs);
        let result = SetCompleter.complete("set buffer language=ru", 22, &ctx);
        let names = names_of(&result);
        assert!(names.contains(&"rust"));
        assert!(names.contains(&"ruby"));
    }

    #[test]
    fn set_completer_value_language_excludes_exact_match() {
        // The completer must drop a language whose name equals the typed
        // prefix — Tab on a fully-typed value is a no-op.
        let (reg, store, dir) = make_ctx_parts();
        let mut langs = LanguageRegistry::new();
        langs.register_identity("ru", &[], &[], &[]).unwrap();
        langs.register_identity("rust", &["rs"], &[], &[]).unwrap();
        let ctx = ctx_with(&reg, &store, dir.path(), &langs);
        let result = SetCompleter.complete("set buffer language=ru", 22, &ctx);
        let names = names_of(&result);
        assert!(names.contains(&"rust"));
        assert!(!names.contains(&"ru"));
    }
}
