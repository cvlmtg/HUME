//! # `requirements.scm` drift
//!
//! `runtime/plugins/core/lsp-install/requirements.scm` is generated from the
//! checked-in `sources.scm` by `scripts/sync-lsp-sources.py --requirements-only`, offline and
//! deterministically. Editing `sources.scm` (or the generator) without
//! regenerating leaves the runtime deciding what is installable from stale
//! data. `requirements_scm_matches_its_generator` runs the generator's
//! `--check` mode and fails when the checked-in file differs.

#[cfg(unix)]
#[test]
fn requirements_scm_matches_its_generator() {
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
