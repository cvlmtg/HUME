//! # Single native-dispatch funnel
//!
//! All native command variants (`Motion`, `Selection`, `Edit`, `EditorCmd`)
//! must execute exclusively through `commands::run_native_body`
//! (`src/editor/commands/pipeline.rs`) — the one place that destructures a
//! native variant to call its `fun` pointer, with `run_dispatch_pipeline`
//! doing all post-dispatch bookkeeping (paste-session commit, jump-list
//! update, dot-repeat recording) around it.
//!
//! `single_native_dispatch_funnel` scans the editor crate for any line
//! binding a native `MappableCommand`'s `fun` field for execution outside
//! `commands/pipeline.rs`.
//!
//! **Opt-out**: annotate the violation line (or the line above it, so
//! `cargo fmt` doesn't hoist a trailing comment) with
//! `// single-funnel-exempt: <reason>`. Use only for a deliberate second
//! dispatch path with its own equivalent bookkeeping — rare.

use super::{collect_source_rs, scan_forbidden};

/// Forbid any site outside `commands/pipeline.rs` from binding the `fun` field of
/// a native `MappableCommand` variant (`Motion { fun`, `Selection { fun`,
/// `Edit { fun`, `EditorCmd { fun`).
///
/// These patterns mean "I am reaching into a native variant to call its
/// function pointer."  Only `run_native_body` in `commands/pipeline.rs` is
/// allowed to do that — it is the single funnel that the dispatch pipeline
/// wraps with all post-dispatch bookkeeping.  A second naked match would
/// silently drop the bookkeeping cluster (jump list, dot-repeat, paste
/// session) exactly as happened in the original regression.
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

    let src_root = root.join("src");
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    collect_source_rs(&src_root, &mut paths);
    // The allowed file may contain these patterns — it IS the funnel.
    paths.retain(|p| {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            != allowed_file
    });

    let violations: Vec<String> =
        scan_forbidden(&paths, root, forbidden_patterns, "// single-funnel-exempt:")
            .into_iter()
            .map(|v| {
                format!(
                    "  {}:{} — `{}` outside dispatch funnel: {}",
                    v.file, v.lineno, v.pattern, v.trimmed
                )
            })
            .collect();

    assert!(
        violations.is_empty(),
        "\nNative-command `fun` binding found outside `run_native_body` in `commands/pipeline.rs`.\n\
         Only that function may destructure and call native MappableCommand variants.\n\
         All bookkeeping (jump list, dot-repeat, paste session) lives\n\
         there — a second dispatch path silently drops the entire cluster.\n\
         Annotate the violation line (or the line above it) with\n\
         `// single-funnel-exempt: <reason>` only if a deliberate\n\
         second path is introduced with equivalent bookkeeping.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
