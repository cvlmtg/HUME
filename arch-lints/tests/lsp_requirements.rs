//! # `requirements/` drift
//!
//! `runtime/plugins/core/lsp-install/data/requirements/` is generated from the
//! checked-in `data/sources/` by `scripts/sync-lsp-sources.py --requirements-only`, offline and
//! deterministically. Editing a sources file (or the generator) without
//! regenerating leaves the runtime deciding what is installable from stale
//! data. `requirements_match_their_generator` runs the generator's `--check`
//! mode and fails when any checked-in file differs, is missing, or is extra.

#[cfg(unix)]
#[test]
fn requirements_match_their_generator() {
    let root = arch_lints::workspace_root();
    let output = std::process::Command::new("python3")
        .arg(root.join("scripts/sync-lsp-sources.py"))
        .arg("--requirements-only")
        .arg("--check")
        .output()
        .expect("python3 must be on PATH to check generated data");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
