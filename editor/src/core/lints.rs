//! Compile-time lints enforced as `cargo test` unit tests.
//!
//! Each lint scans a curated list of source files for patterns that violate an
//! architectural rule.  Tests fail with a human-readable violation list so the
//! offending line is easy to locate and fix.
//!
//! # Borrow-disjoint discipline
//!
//! `Editor` is a flat state container.  Calling a `&mut self` method from
//! another `&mut self` method borrows the whole struct, forcing workarounds
//! (`.clone()`, `mem::take`, pre-collect) on every borrow conflict.  The fix
//! is to convert facility methods to free functions that take only the fields
//! they need, letting Rust prove the borrows are disjoint.
//!
//! `no_self_mut_method_calls_in_editor_module` enforces this by denylisting
//! the old facility methods.  A method enters the denylist the moment it is
//! identified as a borrow-conflict source.  Once the method is deleted, set
//! `migrated: true` so the test also confirms the `fn` definition is gone and
//! cannot be accidentally re-added.
//!
//! **Opt-out**: annotate a line with `// borrow-disjoint-exempt: <reason>`
//! where `<reason>` is one of: `dispatcher`, `recursive-replay`,
//! `borrow-shape-cannot-decompose`.
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
//! `src/auto_pairs.rs`, and `src/helpers.rs` for the forbidden patterns.
//!
//! **Opt-out**: annotate a line with `// grapheme-safe: <reason>` where
//! `<reason>` explains why raw arithmetic is safe (e.g. ASCII-only delimiter
//! scanning, grapheme-boundary-aligned exclusive-to-inclusive conversion).

#[cfg(test)]
mod tests {
    struct DeniedMethod {
        name: &'static str,
        /// `true` once the method has been deleted from `impl Editor`.
        /// The test then also checks that no `fn <name>` definition exists in
        /// the scan files (guards against accidental re-introduction).
        migrated: bool,
    }

    const DENY: &[DeniedMethod] = &[
        // ── Slice 2: doc-edit family ──────────────────────────────────────────
        DeniedMethod { name: "doc_edit",              migrated: true },
        DeniedMethod { name: "doc_edit_grouped",      migrated: true },
        DeniedMethod { name: "doc_undo",              migrated: true },
        DeniedMethod { name: "doc_redo",              migrated: true },
        DeniedMethod { name: "apply_motion",          migrated: true },
        DeniedMethod { name: "propagate_cs_to_panes", migrated: true },
        // ── Slice 3: search-state family ─────────────────────────────────────
        DeniedMethod { name: "clear_buffer_search",   migrated: true },
        DeniedMethod { name: "update_buffer_matches", migrated: true },
        DeniedMethod { name: "update_pane_cursor",    migrated: true },
    ];

    /// Files whose call sites are subject to the denylist.
    ///
    /// `doc_ops.rs`, `search_ops.rs`, etc. are the *destination* modules and
    /// are intentionally excluded — they may define free functions with the
    /// same names.
    const SCAN_FILES: &[&str] = &[
        "src/editor/commands/mod.rs",
        "src/editor/commands/mode.rs",
        "src/editor/commands/edit.rs",
        "src/editor/commands/find.rs",
        "src/editor/commands/scroll.rs",
        "src/editor/commands/search.rs",
        "src/editor/commands/jump.rs",
        "src/editor/commands/typed_file.rs",
        "src/editor/commands/typed_buffer.rs",
        "src/editor/commands/typed_misc.rs",
        "src/editor/mappings.rs",
        "src/editor/mod.rs",
        "src/editor/visual_move.rs",
    ];

    /// Deny `self.<name>(` and `ed.<name>(` call patterns for every entry in
    /// `DENY`.  When `migrated: true`, also deny `fn <name>` definitions so
    /// deleted methods cannot be silently re-added.
    ///
    /// Lines containing `// borrow-disjoint-exempt: <reason>` are skipped.
    #[test]
    fn no_self_mut_method_calls_in_editor_module() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
        let root = std::path::Path::new(&manifest);

        let mut violations: Vec<String> = Vec::new();

        for rel in SCAN_FILES {
            let path = root.join(rel);
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();

                // Skip pure comment lines.
                if trimmed.starts_with("//") {
                    continue;
                }
                // Opt-out for explicitly approved sites.
                if line.contains("// borrow-disjoint-exempt:") {
                    continue;
                }

                // Strip any inline comment so historical references in
                // comments don't trigger false positives.
                let code = match line.find("//") {
                    Some(idx) => &line[..idx],
                    None => line,
                };

                for d in DENY {
                    // Call patterns: `self.<name>(` and `ed.<name>(`.
                    // Excludes sub-field calls (`self.field.<name>(`) because
                    // those only borrow the sub-field, not all of `*self`.
                    let call_self = format!("self.{}(", d.name);
                    let call_ed   = format!("ed.{}(", d.name);
                    if code.contains(&call_self) || code.contains(&call_ed) {
                        violations.push(format!(
                            "  {rel}:{} — forbidden call `{}`: {trimmed}",
                            lineno + 1,
                            d.name,
                        ));
                    }

                    // Definition pattern: only checked after migration.
                    if d.migrated {
                        let def = format!("fn {}(", d.name);
                        if code.contains(&def) {
                            violations.push(format!(
                                "  {rel}:{} — deleted method `{}` re-introduced: {trimmed}",
                                lineno + 1,
                                d.name,
                            ));
                        }
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "\nBorrow-disjoint discipline violated.\n\
             Replace `self.<method>(…)` with the corresponding free function \
             from `doc_ops`, `search_ops`, etc., passing the fields it needs \
             as separate parameters.  See `editor/src/core/lints.rs` for the \
             full rule.\n\
             Violations:\n{}\n",
            violations.join("\n"),
        );
    }

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
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_source_rs(&root.join("src/ops"), &mut paths);
        paths.push(root.join("src/auto_pairs.rs"));
        paths.push(root.join("src/helpers.rs"));

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
}
