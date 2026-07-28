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
//!
//! # Generated `.scm` header drift
//!
//! `scripts/sync-grammars.py` and `scripts/sync-lsp-sources.py` each hold a
//! Python triple-quoted `*_HEADER` template that becomes the leading comment
//! block of a generated `runtime/scheme/*.scm` file. Hand-editing the
//! generated file's header (as opposed to its data rows) drifts it from the
//! template with nothing to catch it — the next routine sync run silently
//! overwrites the hand-edit with the stale template text.
//! `generated_scm_headers_match_their_generator_templates` compares each
//! template against its generated file's leading lines verbatim, treating a
//! line containing a `{sha}`/`{tag}` format slot as matching on its prefix
//! before the `{` rather than byte-for-byte.
//!
//! # `EditorState`/`Editor` field classification drift
//!
//! `Editor::reset_config_state` resets `EditorState.config: ConfigState`
//! wholesale (a field added there is reset by construction — see
//! `ConfigState`'s own doc), but every *other* field on `EditorState` — and
//! on `Editor` itself, which `reset_config_state` also reaches directly for
//! `lsp`/`timer_wheel`/`timer_payloads` — needs a human decision: does
//! `:reload-config` reset it too (like `settings`, via
//! `settings_ops::reset_globals`), or does it survive untouched (buffers,
//! panes, undo history, registers, …)? Nothing enforced that decision get
//! made — a field added directly to `EditorState` instead of nested inside
//! `ConfigState`, or a field added to `Editor` itself, would silently
//! default to "survives", correct for most fields but wrong for one that
//! should have reset.
//!
//! `editor_state_fields_are_classified`/`editor_fields_are_classified`
//! extract every top-level field name from each struct's body and diff it
//! against `EDITOR_STATE_FIELD_CLASSIFICATION`/`EDITOR_FIELD_CLASSIFICATION`
//! in both directions: a new field with no entry fails naming it (forcing a
//! classification decision at the point it's added, not silently); a stale
//! entry for a since-removed field also fails, so the list can't rot into a
//! document nobody trusts.

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
        // one — a name-set diff alone can't catch a row filed under the
        // wrong heading. Cross-check each documented key's own table
        // against its declared scope.
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

    // ── Generated `.scm` header drift ──────────────────────────────────────────

    /// Pull `{const_name} = """\` … `"""`'s content out of a sync script's
    /// source. None of these headers contain three consecutive `"`
    /// characters, so searching for the raw `"""` delimiter (rather than
    /// parsing Python string escapes) is exact here.
    fn extract_header_template(script_src: &str, const_name: &str) -> String {
        let marker = format!("{const_name} = \"\"\"\\\n");
        let start = script_src
            .find(&marker)
            .unwrap_or_else(|| panic!("`{const_name} = \"\"\"\\` not found in sync script"))
            + marker.len();
        let rest = &script_src[start..];
        let end = rest
            .find("\"\"\"")
            .unwrap_or_else(|| panic!("no closing `\"\"\"` found for {const_name}"));
        rest[..end].to_string()
    }

    /// Compare `template` against `generated`'s leading lines. A template
    /// line holding a `.format()` slot (`{sha}`, `{tag}`) matches on its
    /// prefix up to the `{` rather than byte-for-byte, since the generated
    /// file has the placeholder already substituted.
    ///
    /// Also checks that `generated` has no *extra* `;;;` header lines beyond
    /// what `template` accounts for — a hand-appended header paragraph the
    /// template was never updated for is exactly the drift this lint exists
    /// to catch, and comparing template lines only (with no check on
    /// anything left over in `generated`) would miss it entirely.
    fn header_drift(template: &str, generated: &str) -> Option<String> {
        let mut t_lines = template.lines().enumerate();
        let mut g_lines = generated.lines();
        for (i, t) in &mut t_lines {
            let lineno = i + 1;
            let Some(g) = g_lines.next() else {
                return Some(format!(
                    "generated file has only {lineno} header line(s), template has more"
                ));
            };
            match t.find('{') {
                Some(brace) if !g.starts_with(&t[..brace]) => {
                    return Some(format!(
                        "line {lineno}: template prefix {:?} not found in generated line {g:?}",
                        &t[..brace]
                    ));
                }
                None if t != g => {
                    return Some(format!("line {lineno}: template {t:?} != generated {g:?}"));
                }
                _ => {}
            }
        }
        if let Some(extra) = g_lines.next().filter(|line| line.starts_with(";;;")) {
            return Some(format!(
                "generated file has a header line the template doesn't account for: {extra:?}"
            ));
        }
        None
    }

    /// Fail oracle: append a header paragraph to a generated file without
    /// touching its template — the false negative `header_drift` used to
    /// have, since it only compared *template* lines and never checked
    /// whether `generated` had anything left over. A template exactly as
    /// long as the generated header (the common case) passed clean either
    /// way, so this needs a case where `generated` genuinely has more.
    #[test]
    fn header_drift_catches_a_hand_appended_header_paragraph() {
        let template = ";;; one\n;;; two";
        let generated = ";;; one\n;;; two\n;;; three (hand-added, template never touched)";
        assert!(
            header_drift(template, generated).is_some(),
            "an extra ;;; header line beyond the template must be flagged as drift"
        );
    }

    /// A generated file's first non-header line (data, not `;;;` comment)
    /// must not be misread as header drift — `header_drift` stops comparing
    /// once the template runs out, so anything after the header proper is
    /// out of scope for this check.
    #[test]
    fn header_drift_ignores_non_comment_lines_past_the_header() {
        let template = ";;; one\n;;; two";
        let generated = ";;; one\n;;; two\n(define-language! \"rust\" ...)";
        assert_eq!(
            header_drift(template, generated),
            None,
            "a data line past the header must not be flagged as header drift"
        );
    }

    /// Fail oracle: hand-edit a sentence in `runtime/scheme/languages.scm`'s
    /// header without updating `LANGUAGES_HEADER` in `sync-grammars.py` —
    /// this test must fail naming the diverging line.
    #[test]
    fn generated_scm_headers_match_their_generator_templates() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let workspace_root = std::path::Path::new(&manifest)
            .parent()
            .expect("workspace root");

        let triples = [
            (
                "scripts/sync-grammars.py",
                "LANGUAGES_HEADER",
                "runtime/scheme/languages.scm",
            ),
            (
                "scripts/sync-grammars.py",
                "GRAMMAR_SOURCES_HEADER",
                "runtime/scheme/grammar-sources.scm",
            ),
            (
                "scripts/sync-grammars.py",
                "LSP_SERVERS_HEADER",
                "runtime/scheme/lsp-servers.scm",
            ),
            (
                "scripts/sync-lsp-sources.py",
                "LSP_SOURCES_HEADER",
                "runtime/scheme/lsp-sources.scm",
            ),
        ];

        let mut violations: Vec<String> = Vec::new();
        for (script_rel, const_name, generated_rel) in triples {
            let script_path = workspace_root.join(script_rel);
            let script_src = std::fs::read_to_string(&script_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", script_path.display()));
            let template = extract_header_template(&script_src, const_name);

            let generated_path = workspace_root.join(generated_rel);
            let generated = std::fs::read_to_string(&generated_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", generated_path.display()));

            if let Some(reason) = header_drift(&template, &generated) {
                violations.push(format!(
                    "{generated_rel} (template {const_name} in {script_rel}): {reason}"
                ));
            }
        }

        assert!(
            violations.is_empty(),
            "\nA generated runtime/scheme/*.scm header drifted from its sync script's \
             template — the next sync run will silently overwrite the hand-edit with \
             the stale template text. Update the *_HEADER constant to match.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }

    // ── `EditorState` field classification drift ────────────────────────────────

    /// `(field name, classification)` for every `EditorState` field other
    /// than `config` (exempt — `ConfigState`'s wholesale rebuild classifies
    /// itself; see its own doc). Three buckets, by how `:reload-config`'s
    /// reset treats the field:
    /// - `"config: …"` — reset outside `ConfigState`'s own rebuild, by a
    ///   named mechanism in `reset_config_state`.
    /// - `"accounting: …"` — deliberately read, not reset, to judge the
    ///   reload itself.
    /// - `"preserved"` (optionally with a one-clause reason where it isn't
    ///   obvious) — untouched: buffer content/undo/panes/registers/macros/
    ///   search/mode, every transient per-dispatch or per-frame flag, and
    ///   every `Arc` overlay view (self-healing per frame regardless of
    ///   `config`, so resetting the model they mirror is enough).
    const EDITOR_STATE_FIELD_CLASSIFICATION: &[(&str, &str)] = &[
        (
            "buffers",
            "config: clear_languages_all/clear_overrides_all reset language + \
             overrides; content, undo history, and everything else survive",
        ),
        (
            "settings",
            "config: settings_ops::reset_globals rebuilds EditorSettings wholesale",
        ),
        (
            "message_log",
            "accounting: typed_reload_config diffs this before/after the reset \
             to decide whether to report success — resetting it would defeat that",
        ),
        ("mode", "preserved"),
        ("pending_keys", "preserved"),
        ("count", "preserved"),
        ("wait_char", "preserved"),
        ("pending_char", "preserved"),
        ("registers", "preserved"),
        ("kill_ring", "preserved"),
        ("clipboard", "preserved"),
        ("register_prefix", "preserved"),
        ("last_command", "preserved"),
        ("last_paste", "preserved"),
        ("should_quit", "preserved"),
        ("terminate_exit_code", "preserved"),
        ("minibuf", "preserved"),
        ("minibuf_completion", "preserved"),
        ("status_msg", "preserved"),
        ("summary_ttl", "preserved"),
        ("last_find", "preserved"),
        ("search", "preserved"),
        ("focused_pane_id", "preserved"),
        ("panes", "preserved"),
        ("history", "preserved"),
        ("force_full_redraw", "preserved"),
        ("inline_output", "preserved"),
        ("inline_output_entered", "preserved: test-only seam"),
        ("motion_format_scratch", "preserved"),
        ("visual_move_target_cols", "preserved"),
        ("last_repeatable_action", "preserved"),
        ("selection_recipe", "preserved"),
        ("pending_repeat", "preserved"),
        ("insert_session", "preserved"),
        ("autoindent_pending", "preserved"),
        ("explicit_count", "preserved"),
        ("pending_ctrl_extend", "preserved"),
        ("macro_recording", "preserved"),
        ("macro_pending", "preserved"),
        ("replay_queue", "preserved"),
        ("skip_macro_record", "preserved"),
        ("dispatching_typed_command", "preserved"),
        ("is_replaying", "preserved"),
        ("mouse_drag_anchor", "preserved"),
        ("cwd", "preserved"),
        ("lsp_completion_dismiss_pending", "preserved"),
        (
            "completion_menu_view",
            "preserved: Arc view, self-healing per-frame regardless of config",
        ),
        (
            "minibuf_completion_view",
            "preserved: Arc view, self-healing per-frame",
        ),
        (
            "diagnostic_scopes",
            "preserved: ScopeIds are registry-relative, not theme-relative — \
             survive a theme reset",
        ),
        ("inlay_hint_scope", "preserved: registry-relative ScopeId"),
        (
            "virtual_text_fallback_scope",
            "preserved: registry-relative ScopeId",
        ),
        (
            "runtime_scope_cache",
            "preserved: registry-relative ScopeIds",
        ),
        ("popup_view", "preserved: Arc view, self-healing per-frame"),
        (
            "popup_band_view",
            "preserved: Arc view, self-healing per-frame",
        ),
        ("menu_view", "preserved: Arc view, self-healing per-frame"),
        ("drawer_view", "preserved: Arc view, self-healing per-frame"),
        ("picker_view", "preserved: Arc view, self-healing per-frame"),
        ("wake", "preserved: cross-thread waker infra, not config"),
    ];

    /// `(field name, classification)` for every `Editor` field other than
    /// `state` and `view` — both exempt: `state: EditorState` is governed by
    /// `EDITOR_STATE_FIELD_CLASSIFICATION` above, and `view: EngineView` is a
    /// whole rendering-state struct from another crate whose own
    /// config-relevant piece (`view.theme`) is already covered by
    /// `settings_ops::reset_globals`'s doc. Same three buckets as
    /// `EDITOR_STATE_FIELD_CLASSIFICATION`.
    const EDITOR_FIELD_CLASSIFICATION: &[(&str, &str)] = &[
        (
            "kitty_enabled",
            "preserved: the probe result reset_config_state itself reads to \
             rebuild ConfigState's keymap with the same kitty defaults",
        ),
        (
            "scripting",
            "config: typed_reload_config drops this to None directly (not via \
             reset_config_state) right before init_scripting rebuilds it",
        ),
        (
            "builtin_cmd_names",
            "config: overwritten wholesale by init_scripting from the fresh \
             registry, every call including a reload's",
        ),
        ("parse_worker", "preserved"),
        ("parse_worker_disconnect_logged", "preserved"),
        (
            "timer_wheel",
            "config: reset_config_state cancels only the Steel-thunk-payload \
             entries (paired with timer_payloads below); native \
             ViewportDebounce timers survive",
        ),
        (
            "timer_payloads",
            "config: reset_config_state removes only the Steel-thunk-payload \
             entries, paired 1:1 with the timer_wheel cancellations above",
        ),
        (
            "viewport_debounce",
            "preserved: indexes the native ViewportDebounce timers that \
             themselves survive the reset",
        ),
        ("last_viewport_key", "preserved"),
        (
            "virtual_lines_synced",
            "preserved: staleness after a reload is forced by \
             DecorationStores::reset bumping the generation counter, not by \
             resetting this map directly",
        ),
        ("lsp", "config: LspState::reset_config()"),
        ("tui_active", "preserved"),
        ("terminal", "preserved"),
        (
            "applied_mouse_mode",
            "preserved: prepare_frame reconciles it lazily against \
             state.settings after a reload, same as any runtime \
             :set mouse-enabled/mouse-select change",
        ),
    ];

    /// Extract top-level field names from a Rust struct's body text (the
    /// text strictly between its outer `{` and matching `}`). Strips
    /// `///`/`//` comment lines and `#[...]` attribute lines first (so a
    /// doc comment mentioning a colon, or `#[cfg(test)]` on its own line,
    /// is never mistaken for a field), then splits on depth-0 commas —
    /// tracking `(){}[]<>` nesting so a field's own generic type (e.g.
    /// `Vec<(BufferId, Option<String>)>`, or a wrapped multi-line type)
    /// is never mistaken for a field boundary — and takes the last
    /// identifier before each segment's first depth-0 `:` (skipping `::`
    /// path separators, including the ones inside `pub(in crate::editor)`)
    /// as that field's name.
    fn struct_field_names(body: &str) -> Vec<String> {
        let stripped: String = body
            .lines()
            .filter(|line| {
                let t = line.trim();
                !t.starts_with("///") && !t.starts_with("//") && !t.starts_with('#')
            })
            .collect::<Vec<_>>()
            .join(" ");

        fn field_name_in_segment(seg: &[char]) -> Option<String> {
            let mut depth = 0i32;
            let mut colon_at = None;
            for (i, &c) in seg.iter().enumerate() {
                match c {
                    '(' | '[' | '{' | '<' => depth += 1,
                    ')' | ']' | '}' | '>' => depth -= 1,
                    ':' if depth == 0 => {
                        let is_path_sep =
                            seg.get(i + 1) == Some(&':') || (i > 0 && seg[i - 1] == ':');
                        if !is_path_sep {
                            colon_at = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            seg[..colon_at?]
                .iter()
                .collect::<String>()
                .split_whitespace()
                .next_back()
                .map(str::to_string)
        }

        let chars: Vec<char> = stripped.chars().collect();
        let mut names = Vec::new();
        let mut depth = 0i32;
        let mut seg_start = 0usize;
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '(' | '[' | '{' | '<' => depth += 1,
                ')' | ']' | '}' | '>' => depth -= 1,
                ',' if depth == 0 => {
                    if let Some(name) = field_name_in_segment(&chars[seg_start..i]) {
                        names.push(name);
                    }
                    seg_start = i + 1;
                }
                _ => {}
            }
        }
        if let Some(name) = field_name_in_segment(&chars[seg_start..]) {
            names.push(name);
        }
        names
    }

    /// Exercises the tricky patterns actually present in `EditorState`: a
    /// doc comment, an own-line attribute, a field whose type wraps onto a
    /// second line, a generic with an internal tuple (nested commas), and
    /// `pub(in crate::editor)` visibility (a `::` path separator inside the
    /// parens, before the field's own `:`).
    #[test]
    fn struct_field_names_handles_wrapped_generics_and_attributes() {
        let body = r#"
            /// doc comment: mentions a colon, must not be read as a field
            pub(crate) buffers: BufferStore,
            #[cfg(test)]
            pub(crate) inline_output_entered: bool,
            pub(crate) minibuf_completion_view:
                Arc<RwLock<Option<crate::ui::completion_overlay::MinibufCompletionView>>>,
            pub(super) pending_hooks: Vec<(hume_scripting::hooks::HookId, Vec<steel::rvals::SteelVal>)>,
            pub(in crate::editor) completion: Option<completion::CompletionSession>,
        "#;
        assert_eq!(
            struct_field_names(body),
            vec![
                "buffers",
                "inline_output_entered",
                "minibuf_completion_view",
                "pending_hooks",
                "completion",
            ]
        );
    }

    /// Extract the top-level field names of `struct_decl` (e.g.
    /// `"pub(crate) struct EditorState {"`) as found in `src`, excluding
    /// `exempt` (fields governed by their own separate classification —
    /// `EditorState.config`, `Editor.state`/`Editor.view`).
    fn struct_fields_excluding(
        src: &str,
        struct_decl: &str,
        exempt: &[&str],
    ) -> std::collections::BTreeSet<String> {
        let struct_start = src
            .find(struct_decl)
            .unwrap_or_else(|| panic!("{struct_decl:?} not found in editor/mod.rs"));
        let body_start = src[struct_start..]
            .find('{')
            .expect("no opening brace for struct")
            + struct_start;
        let mut depth = 0i32;
        let mut body_end = body_start;
        for (i, c) in src[body_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_end = body_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        struct_field_names(&src[body_start + 1..body_end])
            .into_iter()
            .filter(|f| !exempt.contains(&f.as_str()))
            .collect()
    }

    /// Diff `fields` against `classification`'s names in both directions —
    /// shared assertion body for `editor_state_fields_are_classified` and
    /// `editor_fields_are_classified`. `struct_name`/`const_name` only shape
    /// the panic messages.
    fn assert_fields_classified(
        fields: &std::collections::BTreeSet<String>,
        classification: &[(&str, &str)],
        struct_name: &str,
        const_name: &str,
    ) {
        let classified: std::collections::BTreeSet<&str> =
            classification.iter().map(|(name, _)| *name).collect();

        let unclassified: Vec<&String> = fields
            .iter()
            .filter(|f| !classified.contains(f.as_str()))
            .collect();
        assert!(
            unclassified.is_empty(),
            "{struct_name} gained new field(s) {unclassified:?} with no entry in \
             {const_name} — decide whether :reload-config's reset should touch \
             it (add the mechanism to reset_config_state and classify it \
             \"config: …\" here), or whether it genuinely survives a reload \
             (classify it \"preserved\"), then add the entry"
        );

        let stale: Vec<&str> = classified
            .iter()
            .filter(|name| !fields.contains(name.to_string().as_str()))
            .copied()
            .collect();
        assert!(
            stale.is_empty(),
            "{const_name} lists {stale:?}, which is no longer a field on \
             {struct_name} — remove the stale entry"
        );
    }

    /// Fail oracle: add a field directly to `EditorState` (outside
    /// `config: ConfigState`) without adding a matching entry to
    /// `EDITOR_STATE_FIELD_CLASSIFICATION` — this test fails naming the
    /// field, in either direction (new unclassified field, or a stale
    /// entry for a field that no longer exists).
    #[test]
    fn editor_state_fields_are_classified() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let path = std::path::Path::new(&manifest).join("src/editor/mod.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        let fields = struct_fields_excluding(&src, "pub(crate) struct EditorState {", &["config"]);
        assert_fields_classified(
            &fields,
            EDITOR_STATE_FIELD_CLASSIFICATION,
            "EditorState",
            "EDITOR_STATE_FIELD_CLASSIFICATION",
        );
    }

    /// Fail oracle: add a field directly to `Editor` (outside `state` and
    /// `view`, both separately governed) without a matching entry in
    /// `EDITOR_FIELD_CLASSIFICATION` — same two-directional check as
    /// `editor_state_fields_are_classified`, for the fields
    /// `reset_config_state` reaches through `&mut self` directly rather than
    /// through `self.state.config`.
    #[test]
    fn editor_fields_are_classified() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let path = std::path::Path::new(&manifest).join("src/editor/mod.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        let fields =
            struct_fields_excluding(&src, "pub(crate) struct Editor {", &["state", "view"]);
        assert_fields_classified(
            &fields,
            EDITOR_FIELD_CLASSIFICATION,
            "Editor",
            "EDITOR_FIELD_CLASSIFICATION",
        );
    }
}
