//! # `init.scm.example` core-plugin coverage drift
//!
//! `runtime/init.scm.example` is a hand-maintained list of core plugins —
//! active `(load-plugin ...)`/`(declare-plugin ...)` calls for the ones on by
//! default, commented-out samples for the rest — and nothing cross-checks it
//! against `runtime/plugins/core/`. A new core plugin can ship and be fully
//! documented in the user manual while the example a fresh install is told to
//! copy never mentions it.
//!
//! `init_scm_example_lists_every_core_plugin` scans `runtime/plugins/core/*`
//! for the on-disk plugin set and `runtime/init.scm.example` for every
//! `core:`-named `load-plugin`/`declare-plugin` call — active or commented
//! out — and asserts the two sets match exactly.

use super::quoted_strings;

/// Directory names under `runtime/plugins/core/`, as `core:<name>` ids.
fn on_disk_core_plugins(plugins_root: &std::path::Path) -> std::collections::BTreeSet<String> {
    let rd = std::fs::read_dir(plugins_root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", plugins_root.display()));
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| format!("core:{}", e.file_name().to_string_lossy()))
        .collect()
}

/// Every `core:`-named plugin passed to `(load-plugin ...)` or
/// `(declare-plugin ...)` in `example`, active or commented out. Anchored on
/// the two loader verbs — not every quoted string in the file — so a plugin
/// merely named in prose (a README pointer, a doc URL) doesn't count as
/// "listed": it must appear in copy-pasteable call form.
fn example_core_plugins(example: &str) -> std::collections::BTreeSet<String> {
    example
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().trim_start_matches(';').trim_start();
            let call = rest
                .strip_prefix("(load-plugin ")
                .or_else(|| rest.strip_prefix("(declare-plugin "))?;
            quoted_strings(call).into_iter().next()
        })
        .filter(|name| name.starts_with("core:"))
        .collect()
}

/// Fail oracle: `mv runtime/plugins/core/git-diff runtime/plugins/core/gitdiff`
/// (on-disk name no longer matches the example's `"core:git-diff"`) — this
/// test must fail listing both the missing `core:gitdiff` and the stale
/// `core:git-diff`. Commenting out the `core:git-diff` line's plugin name
/// entirely produces the same missing-entry failure.
#[test]
fn init_scm_example_lists_every_core_plugin() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let workspace_root = std::path::Path::new(&manifest)
        .parent()
        .expect("workspace root");
    let plugins_root = workspace_root.join("runtime/plugins/core");
    let example_path = workspace_root.join("runtime/init.scm.example");

    let on_disk = on_disk_core_plugins(&plugins_root);
    assert!(
        !on_disk.is_empty(),
        "no plugin directories found under {} — this lint would silently \
         check nothing",
        plugins_root.display()
    );

    let example = std::fs::read_to_string(&example_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", example_path.display()));
    let listed = example_core_plugins(&example);

    let mut violations: Vec<String> = Vec::new();
    for missing in on_disk.difference(&listed) {
        violations.push(format!(
            "  {missing}: exists under runtime/plugins/core/ but is not \
             load-plugin'd or declare-plugin'd (active or commented) in \
             runtime/init.scm.example"
        ));
    }
    for stale in listed.difference(&on_disk) {
        violations.push(format!(
            "  {stale}: listed in runtime/init.scm.example but has no \
             directory under runtime/plugins/core/"
        ));
    }

    assert!(
        violations.is_empty(),
        "\nruntime/init.scm.example has drifted from runtime/plugins/core/.\n\
         Every core plugin must appear there, active or as a commented-out \
         sample. Violations:\n{}\n",
        violations.join("\n")
    );
}
