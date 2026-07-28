//! Compile-time lints enforced as `cargo test` unit tests.
//!
//! Each lint scans a curated list of source files for patterns that violate an
//! architectural rule. Tests fail with a human-readable violation list so the
//! offending line is easy to locate and fix.
//!
//! # Grapheme-cluster discipline
//!
//! All position advances in motion and selection code must go through
//! `next_grapheme_boundary` / `prev_grapheme_boundary` — never raw
//! `pos += 1` / `pos -= 1`, which skip over combining codepoints (e.g. `é` =
//! U+0065 + U+0301) instead of advancing a full grapheme cluster.
//!
//! `no_raw_char_stepping_in_motion_code` recursively scans `src/ops/`,
//! `hume-editing/src/lines.rs` + `hume-editing/src/word.rs` for the
//! forbidden patterns.
//!
//! **Opt-out**: annotate a line with `// grapheme-safe: <reason>` (e.g.
//! ASCII-only delimiter scanning, grapheme-boundary-aligned bound conversion).
//!
//! # Single native-dispatch funnel
//!
//! All native command variants (`Motion`, `Selection`, `Edit`, `EditorCmd`)
//! must execute exclusively through `commands::run_native_body`
//! (`src/editor/commands/mod.rs`) — the one place that destructures a native
//! variant to call its `fun` pointer, with `run_dispatch_pipeline` doing all
//! post-dispatch bookkeeping (paste-session commit, jump-list update,
//! dot-repeat recording, `last_command` stamping) around it.
//!
//! `single_native_dispatch_funnel` scans the editor crate for any line
//! binding a native `MappableCommand`'s `fun` field for execution outside
//! `commands/mod.rs`.
//!
//! **Opt-out**: annotate the violation line (or the line above it, so
//! `cargo fmt` doesn't hoist a trailing comment) with
//! `// single-funnel-exempt: <reason>`. Use only for a deliberate second
//! dispatch path with its own equivalent bookkeeping — rare.
//!
//! # Plugin manifest command-list drift
//!
//! A plugin's `manifest.scm` (`#:commands '(...)`) is the zero-argument
//! `(declare-plugin "core:foo")` activation list — hand-maintained, and
//! duplicated nowhere else the compiler checks. A command added to a
//! feature file without a matching manifest entry silently never triggers
//! lazy activation (the plugin loads, but `:that-command` looks unbound
//! until something else activates it); a stale manifest entry for a
//! deleted command is dead weight.
//!
//! `plugin_manifest_commands_match_defined_commands` scans every
//! `runtime/plugins/core/*/manifest.scm` present, and asserts its
//! `#:commands` list is the exact same set (both directions) as every
//! `(define-command! "name" ...)` found across that plugin's own `*.scm`
//! files. A plugin directory with no `manifest.scm` (no zero-arg
//! `declare-plugin` activation defined) is skipped, not a violation.
//!
//! # `write_global`/`write_buffer` single-chokepoint discipline
//!
//! `settings::write_global`/`settings::write_buffer` write a setting's raw
//! value with no derived-state resync (the undo-tree cap on every buffer,
//! the prompt history's ring capacity, every open pane's jump-list capacity,
//! the loaded theme). Production code must go through
//! `editor::settings_ops::apply_global`/`apply_buffer` instead, which wrap
//! them and run those effects — calling `write_global`/`write_buffer`
//! directly silently skips them, the exact bug this lint exists to prevent
//! from recurring.
//!
//! Neither can be `pub(crate)`: `testing/mock_host.rs` (which has no editor
//! state to resync effects against, so it must call the raw writer) is
//! `#[path]`-included into two external integration-test crates
//! (`tests/scripting.rs`, `tests/unix/main.rs`), where `pub(crate)` is
//! invisible. They stay `pub`, and
//! `write_global_and_write_buffer_only_called_from_allowlist` enforces the
//! restriction at the source level instead.
//!
//! **Opt-out**: none — add the new call site's file to `allowed_files`
//! instead, with a comment explaining why it has no editor state to resync
//! effects against.
//!
//! # User-manual option-table drift
//!
//! `user-manual/docs/configuration.md`'s "Global options"/"Buffer options"
//! tables are a hand-maintained mirror of `settings::all_setting_keys()`
//! (plus `"language"`, documented but excluded from that list by design —
//! see `settings.rs`'s module doc). Nothing else keeps the two in sync;
//! `user_manual_option_tables_match_all_setting_keys` scans both tables for
//! every backtick-quoted first-column key and diffs the set against the
//! code's key list in both directions, catching a key added to
//! `define_settings!` without a manual row (or vice versa). It also
//! cross-checks each documented key's own scope, catching a row moved to the
//! wrong table (e.g. a global-only key documented under "Buffer options").
//!
//! # `resync_derived_state` completeness
//!
//! `settings::has_declared_resync` and
//! `editor::settings_ops::resync_derived_state` are two independent sources
//! for the same fact — which settings have a derived-state effect —
//! kept in sync only by a one-directional `debug_assert!` inside
//! `resync_derived_state` itself: a key that declares `resync: true` in
//! `define_settings!` but has no matching match arm panics immediately. The
//! reverse has no check at all: a match arm added for a key that never
//! declared `resync: true` compiles and runs fine on every `:set`, but
//! `reset_globals`'s `has_declared_resync`-gated loop silently skips it on
//! `:reload-config`, so the effect never resyncs after a reload.
//! `resync_derived_state_arms_all_declare_resync_true` extracts every
//! string-literal match-arm pattern from `resync_derived_state`'s source and
//! asserts `has_declared_resync` is true for each, closing the other
//! direction.

#[cfg(test)]
mod tests {
    // ── Grapheme-cluster discipline ───────────────────────────────────────────

    /// Collect all `.rs` files under `dir`, recursively, excluding any
    /// directory named `tests` and any file named `tests.rs`.  Results are
    /// sorted for deterministic test output.
    fn collect_source_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            let n = name.to_string_lossy();
            if path.is_dir() && n != "tests" {
                collect_source_rs(&path, out);
            } else if path.is_file() && n.ends_with(".rs") && n != "tests.rs" {
                out.push(path);
            }
        }
    }

    /// The portion of `line` before any line comment (`//`), skipping `//`
    /// that appears inside a string literal — a naive `line.find("//")`
    /// would truncate a call like `write_global(key, "a//b", ...)`
    /// at the string's embedded `//`, hiding the rest of the line (and any
    /// forbidden pattern in it) from every lint that strips comments this
    /// way. Escaped quotes (`\"`) inside a string keep it open; char
    /// literals (`'x'`, `'\x'`) are skipped so their quote marks don't
    /// falsely open/close string tracking; a bare `'` that isn't a char
    /// literal (a lifetime) is left alone. Raw strings (`r"..."`) are not
    /// handled — none of the scanned patterns appear inside one today.
    fn strip_line_comment(line: &str) -> &str {
        let bytes = line.as_bytes();
        let mut in_string = false;
        let mut escaped = false;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }
            match b {
                b'"' => {
                    in_string = true;
                    i += 1;
                }
                b'\'' if bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\'') => {
                    i += 4; // escaped char literal: '\x'
                }
                b'\'' if bytes.get(i + 2) == Some(&b'\'') => {
                    i += 3; // char literal: 'x'
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') => return &line[..i],
                _ => i += 1,
            }
        }
        line
    }

    /// True if `code` assigns to a `.statusline` field (`x.statusline = …`),
    /// not an equality comparison (`x.statusline == …`) or an unrelated field
    /// (`x.statusline_foo = …`). A naive `contains(".statusline =")` matches
    /// `==` too — a false positive that would fail the build on a legitimate
    /// comparison.
    fn assigns_statusline(code: &str) -> bool {
        let mut rest = code;
        while let Some(idx) = rest.find(".statusline") {
            let after = &rest[idx + ".statusline".len()..];
            let after_trimmed = after.trim_start();
            if let Some(tail) = after_trimmed.strip_prefix('=')
                && !tail.starts_with('=')
            {
                return true; // single `=` → assignment, not `==`
            }
            rest = &rest[idx + ".statusline".len()..];
        }
        false
    }

    /// Scan motion-related source files for raw char-level stepping.
    ///
    /// The grapheme cluster invariant (CLAUDE.md) requires that all position
    /// advances in motion and selection code go through `next_grapheme_boundary`
    /// or `prev_grapheme_boundary` — never raw `pos += 1` / `pos -= 1`.
    ///
    /// The bug that prompted this test: word motions used `pos += 1`, causing
    /// combining codepoints (e.g. U+0301, which classify_char sees as Punctuation)
    /// to be treated as false word boundaries inside a grapheme cluster.
    ///
    /// This test reads the source files at compile time, skips test blocks and
    /// comment lines, and fails if any active code contains a forbidden stepping
    /// pattern on a char-position variable.
    #[test]
    fn no_raw_char_stepping_in_motion_code() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");

        // Collect all non-test source files under src/ops/ plus two standalone files.
        // Using directory traversal so future submodule splits are covered automatically.
        let root = std::path::Path::new(&manifest);
        let workspace_root = root.parent().expect("workspace root");
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_source_rs(&root.join("src/ops"), &mut paths);
        // lines.rs and word.rs live in the editing crate — scan them from there.
        paths.push(workspace_root.join("hume-editing/src/lines.rs"));
        paths.push(workspace_root.join("hume-editing/src/word.rs"));

        // Forbidden patterns — raw +1/-1 steps on char-position variables.
        // Stepping by 1 skips over combining codepoints (e.g. é = U+0065 + U+0301)
        // instead of advancing by a full grapheme cluster.
        //
        // Assignment forms: caught directly.
        // char_at() forms: explicitly forbidden by CLAUDE.md — char_at(pos + 1) and
        //   char_at(pos - 1) were the original motivating footguns.
        let forbidden = [
            // ── Assignment forms ───────────────────────────────────────────────
            "pos += 1",
            "pos -= 1",
            "start += 1",
            "start -= 1",
            "end += 1",
            "end -= 1",
            "head += 1",
            "head -= 1",
            "anchor += 1",
            "anchor -= 1",
            // ── char_at() expression forms ─────────────────────────────────────
            "char_at(pos + 1)",
            "char_at(pos - 1)",
            "char_at(head + 1)",
            "char_at(head - 1)",
            "char_at(anchor + 1)",
            "char_at(anchor - 1)",
        ];

        let mut violations: Vec<String> = Vec::new();

        for path in &paths {
            let file = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            let src = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            // Track whether we are inside a `#[cfg(test)] mod tests { … }` block
            // so we don't flag historical references in test comments.
            let mut in_test_block = false;
            let mut brace_depth: i64 = 0;
            let mut test_entry_depth: i64 = 0;
            let mut saw_cfg_test = false;

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();

                // Detect `#[cfg(test)]` on its own line.
                if trimmed == "#[cfg(test)]" {
                    saw_cfg_test = true;
                }
                // The very next `mod tests` after that attribute opens the block.
                if saw_cfg_test && trimmed.starts_with("mod tests") {
                    in_test_block = true;
                    test_entry_depth = brace_depth;
                    saw_cfg_test = false;
                }

                // Track brace depth so we know when the test block closes.
                let opens = line.chars().filter(|&c| c == '{').count() as i64;
                let closes = line.chars().filter(|&c| c == '}').count() as i64;
                brace_depth += opens - closes;
                if in_test_block && brace_depth <= test_entry_depth {
                    in_test_block = false;
                }

                // Skip everything inside the test module.
                if in_test_block {
                    continue;
                }

                // Skip pure comment lines.
                if trimmed.starts_with("//") {
                    continue;
                }

                // `// grapheme-safe: <reason>` opt-out: lines where raw +1/-1 is
                // intentional and safe (e.g. ASCII-only delimiter arithmetic, or
                // converting a grapheme-boundary-aligned exclusive end to inclusive).
                // The reason after the colon must explain *why* it is safe.
                if line.contains("// grapheme-safe:") {
                    continue;
                }

                // Strip any remaining inline comment before pattern-matching.
                // This prevents explanatory comments like `// was: pos += 1` from
                // triggering false positives.
                let code = strip_line_comment(line);

                for pattern in &forbidden {
                    if code.contains(pattern) {
                        violations.push(format!(
                            "  {file}:{} — `{pattern}` in: {trimmed}",
                            lineno + 1,
                        ));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "\nRaw char-level stepping detected in motion/selection code.\n\
             Use next_grapheme_boundary(buf, pos) or prev_grapheme_boundary(buf, pos) instead.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }

    // ── Single native-dispatch funnel discipline ──────────────────────────────

    /// Forbid any site outside `commands/mod.rs` from binding the `fun` field of
    /// a native `MappableCommand` variant (`Motion { fun`, `Selection { fun`,
    /// `Edit { fun`, `EditorCmd { fun`).
    ///
    /// These patterns mean "I am reaching into a native variant to call its
    /// function pointer."  Only `run_native_body` in `commands/mod.rs` is
    /// allowed to do that — it is the single funnel that the dispatch pipeline
    /// wraps with all post-dispatch bookkeeping.  A second naked match would
    /// silently drop the bookkeeping cluster (jump list, last_command,
    /// dot-repeat, paste session) exactly as happened in the original regression.
    ///
    /// Opt-out: annotate the violation line, or the line immediately above it,
    /// with `// single-funnel-exempt: <reason>`.  The preceding-line form is the
    /// natural one (`cargo fmt` hoists trailing comments above, and a leading
    /// comment reads as "why the next line is exempt").
    ///
    /// Fail oracle: paste
    ///   `MappableCommand::Motion { fun, .. } => fun(t, s, 1, m),`
    /// into `host_impl.rs` — this test must fail naming that line.
    #[test]
    fn single_native_dispatch_funnel() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let root = std::path::Path::new(&manifest);

        // The patterns that indicate "binding fun from a native MappableCommand variant".
        // Matches both `MappableCommand::Motion { fun` and `Self::Motion { fun` etc.
        let forbidden_patterns: &[&str] = &[
            "Motion { fun",
            "Selection { fun",
            "Edit { fun",
            "EditorCmd { fun",
        ];

        // Only this file is the single legal executor of native commands.
        let allowed_file = "src/editor/commands/mod.rs";

        let mut violations: Vec<String> = Vec::new();

        let src_root = root.join("src");
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_source_rs(&src_root, &mut paths);

        for path in &paths {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            // The allowed file may contain these patterns — it IS the funnel.
            if rel == allowed_file {
                continue;
            }

            let src = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            let mut in_test_block = false;
            let mut brace_depth: i64 = 0;
            let mut test_entry_depth: i64 = 0;
            let mut saw_cfg_test = false;
            // Previous non-test source line, kept so an exempt marker on the line
            // *above* a violation suppresses it. `cargo fmt` hoists trailing
            // comments onto their own line, so the marker often sits above the
            // forbidden pattern rather than beside it.
            let mut prev_line: &str = "";

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();

                // Track the previous source line for the preceding-line opt-out.
                // Done first so every `continue` below still keeps prev_line in
                // sync with the real line history.
                let prev_for_exempt = prev_line;
                prev_line = line;

                if trimmed == "#[cfg(test)]" {
                    saw_cfg_test = true;
                }
                if saw_cfg_test && trimmed.starts_with("mod tests") {
                    in_test_block = true;
                    test_entry_depth = brace_depth;
                    saw_cfg_test = false;
                }

                let opens = line.chars().filter(|&c| c == '{').count() as i64;
                let closes = line.chars().filter(|&c| c == '}').count() as i64;
                brace_depth += opens - closes;
                if in_test_block && brace_depth <= test_entry_depth {
                    in_test_block = false;
                }

                if in_test_block {
                    continue;
                }

                if trimmed.starts_with("//") {
                    continue;
                }

                // Same-line opt-out (marker sits beside the forbidden pattern).
                if line.contains("// single-funnel-exempt:") {
                    continue;
                }

                let code = strip_line_comment(line);

                for pattern in forbidden_patterns {
                    if code.contains(pattern) {
                        // Previous-line opt-out: marker on the line above the
                        // forbidden pattern. fmt moves trailing comments above,
                        // so this is the common placement.
                        if prev_for_exempt.contains("// single-funnel-exempt:") {
                            continue;
                        }
                        violations.push(format!(
                            "  {rel}:{} — `{pattern}` outside dispatch funnel: {trimmed}",
                            lineno + 1,
                        ));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "\nNative-command `fun` binding found outside `run_native_body` in `commands/mod.rs`.\n\
             Only that function may destructure and call native MappableCommand variants.\n\
             All bookkeeping (jump list, last_command, dot-repeat, paste session) lives\n\
             there — a second dispatch path silently drops the entire cluster.\n\
             Annotate the violation line (or the line above it) with\n\
             `// single-funnel-exempt: <reason>` only if a deliberate\n\
             second path is introduced with equivalent bookkeeping.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }

    // ── Plugin manifest command-list drift ────────────────────────────────────

    /// Extracts every double-quoted string literal's contents from `s`,
    /// verbatim (no escape processing — plugin command names never contain a
    /// `"`, so a naive quote-delimited split is exact here).
    fn quoted_strings(s: &str) -> Vec<String> {
        s.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    /// The string literals inside `manifest.scm`'s `#:commands '(...)` list —
    /// scoped to that one clause so `#:languages`/the plugin name's own
    /// quoted strings elsewhere in the file are never mistaken for commands.
    /// Empty (not a violation by itself) if the manifest declares no
    /// `#:commands` at all. Comment lines are stripped first — every
    /// `manifest.scm` opens with a `;`-comment header that mentions
    /// `#:commands` in prose, which would otherwise be the *first* (wrong)
    /// match.
    fn manifest_declared_commands(src: &str) -> Vec<String> {
        let code_only: String = src
            .lines()
            .filter(|line| !line.trim_start().starts_with(';'))
            .collect::<Vec<_>>()
            .join("\n");
        let src = &code_only;
        let Some(after) = src.find("#:commands") else {
            return Vec::new();
        };
        let after = &src[after..];
        let Some(open) = after.find('(') else {
            return Vec::new();
        };
        let mut depth = 0i32;
        let mut end = None;
        for (i, c) in after[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return Vec::new();
        };
        quoted_strings(&after[open..end])
    }

    /// Every name in a `(define-command! "name" ...)` call, across every
    /// `*.scm` file directly inside `dir` (plugins don't nest subdirectories).
    fn defined_commands(dir: &std::path::Path) -> Vec<String> {
        let mut names = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return names;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("scm") {
                continue;
            }
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            // Comment lines stripped first, same reason as
            // `manifest_declared_commands` — a doc comment mentioning
            // `define-command!` in prose must never be mistaken for a call.
            let code_only: String = src
                .lines()
                .filter(|line| !line.trim_start().starts_with(';'))
                .collect::<Vec<_>>()
                .join("\n");
            for (idx, _) in code_only.match_indices("define-command!") {
                let after = &code_only[idx + "define-command!".len()..];
                if let Some(name) = quoted_strings(after).into_iter().next() {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Fail oracle: comment out one entry in `core:lsp/manifest.scm`'s
    /// `#:commands` list (e.g. delete `"lsp-hover"`) — this test must fail
    /// naming `lsp-hover` as manifest-missing.
    #[test]
    fn plugin_manifest_commands_match_defined_commands() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let root = std::path::Path::new(&manifest);
        let workspace_root = root.parent().expect("workspace root");
        let plugins_root = workspace_root.join("runtime/plugins/core");

        let mut violations: Vec<String> = Vec::new();

        let Ok(rd) = std::fs::read_dir(&plugins_root) else {
            panic!("cannot read {}", plugins_root.display());
        };
        let mut plugin_dirs: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        plugin_dirs.sort();

        for dir in plugin_dirs {
            let manifest_path = dir.join("manifest.scm");
            if !manifest_path.exists() {
                continue; // no zero-arg declare-plugin activation — nothing to check
            }
            let plugin_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let manifest_src = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path.display()));

            let declared: std::collections::BTreeSet<String> =
                manifest_declared_commands(&manifest_src)
                    .into_iter()
                    .collect();
            let defined: std::collections::BTreeSet<String> =
                defined_commands(&dir).into_iter().collect();

            for missing in declared.difference(&defined) {
                violations.push(format!(
                    "  {plugin_name}: \"{missing}\" is in manifest.scm's #:commands \
                     but no define-command! defines it"
                ));
            }
            for extra in defined.difference(&declared) {
                violations.push(format!(
                    "  {plugin_name}: \"{extra}\" is defined via define-command! but \
                     missing from manifest.scm's #:commands"
                ));
            }
        }

        assert!(
            violations.is_empty(),
            "\nPlugin manifest.scm #:commands drifted from its own define-command! calls.\n\
             A missing manifest entry means that command never triggers lazy activation;\n\
             a stale one is dead weight. Keep both in sync.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }

    // ── write_global / write_buffer single-chokepoint discipline ──────────────

    /// Only `editor::settings_ops::apply_global`/`apply_buffer` (the
    /// chokepoints that write a setting *and* resync derived state) and
    /// `testing::mock_host::MockHost` (which has no editor state to resync
    /// against) may call `settings::write_global`/`settings::write_buffer`
    /// directly. `collect_source_rs` already excludes every `tests.rs`-named
    /// file and `tests/` directory, so `settings/tests.rs`'s own direct calls
    /// (exercising the raw writers in isolation) never reach this scan.
    ///
    /// The `fn write_global`/`fn write_buffer` exclusions distinguish the
    /// definition signature lines themselves (which also contain the
    /// substrings `write_global(`/`write_buffer(`) from an actual call.
    ///
    /// Fail oracle: add `crate::settings::write_global(...)` directly to
    /// `typed_file.rs` — this test must fail naming that line.
    #[test]
    fn write_global_and_write_buffer_only_called_from_allowlist() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let root = std::path::Path::new(&manifest);

        let allowed_files: &[&str] = &[
            "src/settings.rs",            // definition site
            "src/editor/settings_ops.rs", // the chokepoint
            "src/testing/mock_host.rs",   // no editor state to resync against
        ];

        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_source_rs(&root.join("src"), &mut paths);

        let mut violations: Vec<String> = Vec::new();

        for path in &paths {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            if allowed_files.contains(&rel.as_str()) {
                continue;
            }

            let src = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                let code = strip_line_comment(line);
                let is_call = (code.contains("write_global(") && !code.contains("fn write_global"))
                    || (code.contains("write_buffer(") && !code.contains("fn write_buffer"));
                if is_call {
                    violations.push(format!("  {rel}:{} — {trimmed}", lineno + 1));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "\nsettings::write_global/write_buffer called outside their allowlist.\n\
             These are the raw field writers with no derived-state resync —\n\
             production code must go through editor::settings_ops::apply_global/\n\
             apply_buffer instead.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }

    // ── statusline: no raw field write outside the chokepoint ─────────────────

    /// `EditorSettings.statusline` has a manual `write_global` arm (it isn't
    /// generic-macro storage — see `settings.rs`'s "Statusline config" comment)
    /// rather than a derive-generated field, so
    /// `write_global_and_write_buffer_only_called_from_allowlist` can't see a
    /// raw `settings.statusline = …` assignment; only its own literal call
    /// pattern would. This is the same trap door the theme bug (commit
    /// 3c97bd44) exploited: a caller with `&mut EditorState` can always skip
    /// the chokepoint and assign the field directly, so the only case caught
    /// here is the field named literally `statusline` — this scan closes it
    /// for that one field the same way the write_global/write_buffer scan
    /// closes it in general.
    ///
    /// Fail oracle: add `self.state.settings.statusline = …` directly to
    /// `editor/host_impl.rs`'s `configure_statusline` (reverting Fix D) — this
    /// test must fail naming that line.
    #[test]
    fn statusline_field_only_written_from_allowlist() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let root = std::path::Path::new(&manifest);

        let allowed_files: &[&str] = &[
            "src/settings.rs",          // write_global's manual statusline arm
            "src/testing/mock_host.rs", // no editor state to resync against
            "src/editor/lints.rs",      // this scan's own pattern string, not a write
        ];

        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_source_rs(&root.join("src"), &mut paths);

        let mut violations: Vec<String> = Vec::new();

        for path in &paths {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            if allowed_files.contains(&rel.as_str()) {
                continue;
            }

            let src = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                let code = strip_line_comment(line);
                if assigns_statusline(code) {
                    violations.push(format!("  {rel}:{} — {trimmed}", lineno + 1));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "\n.statusline written outside its allowlist.\n\
             Production code must route through editor::settings_ops::apply_global\n\
             (key \"statusline\") instead of assigning the field directly.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }

    // ── assigns_statusline ────────────────────────────────────────────────────

    #[test]
    fn assigns_statusline_distinguishes_comparison() {
        // Fail oracle: revert assigns_statusline to `code.contains(".statusline =")`
        // and the `==` cases below must start failing (they'd report as assignments).
        assert!(assigns_statusline("buf.statusline = new_val;"));
        assert!(assigns_statusline("buf.statusline=new_val;"));
        assert!(!assigns_statusline("if buf.statusline == default {"));
        assert!(!assigns_statusline("if buf.statusline_foo = default {"));
        assert!(!assigns_statusline("let x = buf.statusline;"));
    }

    // ── strip_line_comment ────────────────────────────────────────────────────

    #[test]
    fn strip_line_comment_cases() {
        // Fail oracle: revert strip_line_comment to a naive `line.find("//")`
        // and the string-literal cases below (2nd and 4th) must start failing.
        assert_eq!(strip_line_comment("foo(); // bar"), "foo(); ");
        assert_eq!(strip_line_comment("foo();"), "foo();");
        assert_eq!(strip_line_comment(r#"call("a//b")"#), r#"call("a//b")"#);
        assert_eq!(
            strip_line_comment(r#"call("a//b") // note"#),
            r#"call("a//b") "#
        );
        assert_eq!(
            strip_line_comment(r#"let q = '"'; // c"#),
            r#"let q = '"'; "#
        );
        assert_eq!(
            strip_line_comment(r#""a\"//b""#),
            r#""a\"//b""#,
            "escaped quote must not end the string early"
        );
    }

    // ── User-manual option-table drift ─────────────────────────────────────────

    /// The text between the first occurrence of `heading` and the next
    /// top-level (`\n## `) heading — used to scope the key scan below to
    /// just the two option tables, not the whole file (which has other
    /// backtick-quoted, table-shaped content — the statusline elements
    /// table, the key-string grammar table — that would otherwise produce
    /// false-positive "stale" keys).
    fn section_after<'a>(text: &'a str, heading: &str) -> &'a str {
        let start = text
            .find(heading)
            .unwrap_or_else(|| panic!("heading '{heading}' not found in configuration.md"));
        let after = &text[start + heading.len()..];
        let end = after.find("\n## ").unwrap_or(after.len());
        &after[..end]
    }

    /// Every `` `key` `` in a markdown table's first column: a line trimmed
    /// to start with `` | ` `` (no other content in this file's tables looks
    /// like that once scoped to one `## `-delimited section).
    fn first_column_keys(section: &str) -> std::collections::BTreeSet<String> {
        section
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("| `")?;
                let (key, _) = rest.split_once('`')?;
                (!key.is_empty()).then(|| key.to_string())
            })
            .collect()
    }

    /// Fail oracle: add a new entry to `define_settings!` (any section) or
    /// to `user-manual/docs/configuration.md`'s option tables without the
    /// matching change on the other side — this test fails naming the key
    /// and which direction it's missing.
    #[test]
    fn user_manual_option_tables_match_all_setting_keys() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let manual_path =
            std::path::Path::new(&manifest).join("../user-manual/docs/configuration.md");
        let text = std::fs::read_to_string(&manual_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manual_path.display()));

        let global_documented = first_column_keys(section_after(&text, "## Global options"));
        let buffer_documented = first_column_keys(section_after(&text, "## Buffer options"));
        let mut documented = global_documented.clone();
        documented.extend(buffer_documented.clone());

        let mut code_keys: std::collections::BTreeSet<String> = crate::settings::all_setting_keys()
            .iter()
            .map(|k| k.to_string())
            .collect();
        // "language" has no define_settings! entry by design (see
        // settings.rs's module doc) but is documented in the Buffer options
        // table, so it's added here rather than to all_setting_keys() itself.
        code_keys.insert("language".to_string());

        let missing_from_docs: Vec<_> = code_keys.difference(&documented).collect();
        let stale_in_docs: Vec<_> = documented.difference(&code_keys).collect();

        // A key present in *some* table isn't necessarily under the *right*
        // one — a name-set diff alone can't catch a row that migrated to the
        // wrong heading (exactly what the Scope rework touched). Cross-check
        // each documented key's own table against its declared scope.
        use crate::settings::Scope;
        let misplaced: Vec<String> = global_documented
            .iter()
            .filter(|k| k.as_str() != "language")
            .filter(|k| !crate::settings::setting_scopes(k).contains(&Scope::Global))
            .map(|k| format!("'{k}' is under Global options but its scope list has no Global"))
            .chain(
                buffer_documented
                    .iter()
                    .filter(|k| k.as_str() != "language") // buffer-only by special case, no scope entry
                    .filter(|k| !crate::settings::setting_scopes(k).contains(&Scope::Buffer))
                    .map(|k| {
                        format!("'{k}' is under Buffer options but its scope list has no Buffer")
                    }),
            )
            .collect();

        assert!(
            missing_from_docs.is_empty() && stale_in_docs.is_empty() && misplaced.is_empty(),
            "\nuser-manual/docs/configuration.md option tables drifted from \
             settings::all_setting_keys().\n\
             In code but missing from the docs tables: {missing_from_docs:?}\n\
             In the docs tables but not a real setting key: {stale_in_docs:?}\n\
             Documented under the wrong heading: {misplaced:?}\n"
        );
    }

    // ── `resync_derived_state` completeness ──────────────────────────────────

    /// Every string literal used as a match-arm pattern inside
    /// `editor::settings_ops::resync_derived_state`'s `match key { ... }` —
    /// the key names that function actually has code for, regardless of
    /// whether `define_settings!` declared `resync: true` for them. Brace-depth
    /// tracked (same technique as `manifest_declared_commands` above) so the
    /// scan stops exactly at the function's own closing brace, not some
    /// unrelated later `"..."` literal.
    fn resync_derived_state_arm_keys(src: &str) -> std::collections::BTreeSet<String> {
        let fn_start = src
            .find("fn resync_derived_state(")
            .expect("fn resync_derived_state not found in settings_ops.rs");
        let body_start = src[fn_start..]
            .find('{')
            .expect("no opening brace for resync_derived_state")
            + fn_start;
        let mut depth = 0i32;
        let mut end = body_start;
        for (i, c) in src[body_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        src[body_start..end]
            .lines()
            .filter_map(|line| {
                let rest = line.trim_start().strip_prefix('"')?;
                let (key, after) = rest.split_once('"')?;
                let after = after.trim_start();
                (after.starts_with("=>") || after.starts_with("if")).then(|| key.to_string())
            })
            .collect()
    }

    /// Fail oracle: add a match arm to `resync_derived_state` for a key that
    /// never declares `resync: true` in `define_settings!` — this test fails
    /// naming the key, catching the direction the function's own
    /// `debug_assert!` cannot (that one only catches "declared but no arm").
    #[test]
    fn resync_derived_state_arms_all_declare_resync_true() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let path = std::path::Path::new(&manifest).join("src/editor/settings_ops.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        let undeclared: Vec<String> = resync_derived_state_arm_keys(&src)
            .into_iter()
            .filter(|key| !crate::settings::has_declared_resync(key))
            .collect();

        assert!(
            undeclared.is_empty(),
            "resync_derived_state has a match arm for {undeclared:?} but \
             define_settings! doesn't declare `resync: true` for it — \
             reset_globals's has_declared_resync-gated loop would silently \
             skip resyncing it on :reload-config"
        );
    }
}
