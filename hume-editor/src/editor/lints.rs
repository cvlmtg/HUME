//! Compile-time lints enforced as `cargo test` unit tests.
//!
//! Each lint scans a curated list of source files for patterns that violate an
//! architectural rule.  Tests fail with a human-readable violation list so the
//! offending line is easy to locate and fix.
//!
//! # Grapheme-cluster discipline
//!
//! All position advances in motion and selection code must go through
//! `next_grapheme_boundary` / `prev_grapheme_boundary` — never raw
//! `pos += 1` / `pos -= 1`.  Stepping by 1 skips over combining codepoints
//! (e.g. `é` = U+0065 + U+0301) instead of advancing by a full grapheme
//! cluster.
//!
//! `no_raw_char_stepping_in_motion_code` recursively scans `src/ops/`,
//! `src/auto_pairs.rs`, and `hume-editing/src/lines.rs` + `hume-editing/src/word.rs`
//! for the forbidden patterns.
//!
//! **Opt-out**: annotate a line with `// grapheme-safe: <reason>` where
//! `<reason>` explains why raw arithmetic is safe (e.g. ASCII-only delimiter
//! scanning, grapheme-boundary-aligned exclusive-to-inclusive conversion).
//!
//! # Single native-dispatch funnel
//!
//! All native command variants (`Motion`, `Selection`, `Edit`, `EditorCmd`) must
//! be executed **exclusively** through `commands::dispatch_native`
//! (`src/editor/commands/mod.rs`).  That function is the single place that
//! performs all post-dispatch bookkeeping: paste-session commit, jump-list update,
//! dot-repeat recording, and `last_command` stamping.
//!
//! The original regression: a second dispatch path copied only the bare `match`
//! arms and none of the bookkeeping.  Commands ran correctly but silently dropped
//! the whole side-effect cluster.
//!
//! `single_native_dispatch_funnel` scans the editor crate for any line that
//! binds the `fun` field of a native `MappableCommand` variant for execution.
//! Only `src/editor/commands/mod.rs` is allowed to do that.
//!
//! **Opt-out**: annotate a line with `// single-funnel-exempt: <reason>`.  Use
//! only when a deliberate second dispatch path is introduced with its own
//! equivalent bookkeeping (which should be exceedingly rare).

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
        paths.push(root.join("src/auto_pairs.rs"));
        // lines.rs and word.rs live in the editing crate — scan them from there.
        // (helpers.rs was split into these two modules.)
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
                let code = match line.find("//") {
                    Some(idx) => &line[..idx],
                    None => line,
                };

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
    /// function pointer."  Only `dispatch_native` in `commands/mod.rs` is
    /// allowed to do that — it is the single funnel that carries all
    /// post-dispatch bookkeeping.  A second naked match would silently drop
    /// the bookkeeping cluster (jump list, last_command, dot-repeat, paste
    /// session) exactly as happened in the original regression.
    ///
    /// Opt-out: annotate the line with `// single-funnel-exempt: <reason>`.
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

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();

                if trimmed == "#[cfg(test)]" {
                    saw_cfg_test = true;
                }
                if saw_cfg_test && trimmed.starts_with("mod tests") {
                    in_test_block = true;
                    test_entry_depth = brace_depth;
                    saw_cfg_test = false;
                }

                let opens  = line.chars().filter(|&c| c == '{').count() as i64;
                let closes = line.chars().filter(|&c| c == '}').count() as i64;
                brace_depth += opens - closes;
                if in_test_block && brace_depth <= test_entry_depth {
                    in_test_block = false;
                }

                if in_test_block { continue; }

                if trimmed.starts_with("//") { continue; }

                if line.contains("// single-funnel-exempt:") { continue; }

                let code = match line.find("//") {
                    Some(idx) => &line[..idx],
                    None => line,
                };

                for pattern in forbidden_patterns {
                    if code.contains(pattern) {
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
            "\nNative-command `fun` binding found outside `dispatch_native` in `commands/mod.rs`.\n\
             Only that function may destructure and call native MappableCommand variants.\n\
             All bookkeeping (jump list, last_command, dot-repeat, paste session) lives\n\
             there — a second dispatch path silently drops the entire cluster.\n\
             Annotate with `// single-funnel-exempt: <reason>` only if a deliberate\n\
             second path is introduced with equivalent bookkeeping.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

