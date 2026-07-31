//! # Single native-dispatch funnel
//!
//! All native command variants (`Motion`, `Selection`, `Edit`, `EditorCmd`)
//! must execute exclusively through `commands::run_native_body`
//! (`src/editor/commands/pipeline.rs`) — the one place that destructures a
//! native variant to call its `fun` pointer, with `run_dispatch_pipeline`
//! doing all post-dispatch bookkeeping (paste-session commit, jump-list
//! update, dot-repeat recording, `last_command` stamping) around it.
//!
//! `single_native_dispatch_funnel` scans the editor crate for any line
//! binding a native `MappableCommand`'s `fun` field for execution outside
//! `commands/pipeline.rs`.
//!
//! **Opt-out**: annotate the violation line (or the line above it, so
//! `cargo fmt` doesn't hoist a trailing comment) with
//! `// single-funnel-exempt: <reason>`. Use only for a deliberate second
//! dispatch path with its own equivalent bookkeeping — rare.

use super::{collect_source_rs, strip_line_comment};

/// Forbid any site outside `commands/pipeline.rs` from binding the `fun` field of
/// a native `MappableCommand` variant (`Motion { fun`, `Selection { fun`,
/// `Edit { fun`, `EditorCmd { fun`).
///
/// These patterns mean "I am reaching into a native variant to call its
/// function pointer."  Only `run_native_body` in `commands/pipeline.rs` is
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
    // Each entry below is the pattern list itself, not a dispatch site — hence
    // the trailing exempt marker on every one (a leading comment above the
    // array only covers its first element once `cargo fmt` expands it).
    let forbidden_patterns: &[&str] = &[
        "Motion { fun",    // single-funnel-exempt: pattern list, not a dispatch site
        "Selection { fun", // single-funnel-exempt: pattern list, not a dispatch site
        "Edit { fun",      // single-funnel-exempt: pattern list, not a dispatch site
        "EditorCmd { fun", // single-funnel-exempt: pattern list, not a dispatch site
    ];

    // Only this file is the single legal executor of native commands.
    let allowed_file = "src/editor/commands/pipeline.rs";

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
        "\nNative-command `fun` binding found outside `run_native_body` in `commands/pipeline.rs`.\n\
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
