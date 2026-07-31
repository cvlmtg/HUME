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
//! # statusline: no raw field write outside the chokepoint
//!
//! `EditorSettings.statusline` has a manual `write_global` arm (it isn't
//! generic-macro storage), so the write_global/write_buffer scan above can't
//! see a raw `settings.statusline = …` assignment; only its own literal call
//! pattern would. `statusline_field_only_written_from_allowlist` closes that
//! gap for the one field named literally `statusline`.

use super::{collect_source_rs, strip_line_comment};

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
        "src/settings.rs",                       // definition site
        "src/editor/settings_ops.rs",            // the chokepoint
        "src/testing/mock_host.rs",              // no editor state to resync against
        "src/editor/lints/statusline_writes.rs", // this scan's own pattern strings, not a call
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

/// `true` if `code` assigns to a `.statusline` field (`x.statusline = …`),
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
        "src/settings.rs",                       // write_global's manual statusline arm
        "src/testing/mock_host.rs",              // no editor state to resync against
        "src/editor/lints/statusline_writes.rs", // this scan's own pattern string, not a write
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
