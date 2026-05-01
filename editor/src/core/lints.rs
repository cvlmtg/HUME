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
        "src/editor/commands.rs",
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
}
