//! # `hume-core` stays a leaf
//!
//! `hume-core` holds the data two or more crates share, so every crate can
//! depend on it. That only holds while it depends on no workspace crate: a
//! dependency edge into one would make the shared crate part of that crate's
//! cycle risk. Third-party crates are fine.
//! `hume_core_depends_on_no_workspace_crate` reads its manifest and fails on
//! any workspace package it declares, normal, dev or build, renamed or not.

use std::str::FromStr;

use toml::{Table, Value};

/// The package each dependency entry names: its `package` key when the entry
/// renames one, else the entry's own key. Covers each dependency table at the
/// top level or under a `[target.<cfg>]` table.
fn declared_packages(manifest: &Table) -> Vec<&str> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let targets = manifest
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values().filter_map(Value::as_table));
    std::iter::once(manifest)
        .chain(targets)
        .flat_map(|table| {
            KINDS
                .iter()
                .filter_map(move |kind| table.get(*kind)?.as_table())
        })
        .flat_map(|deps| {
            deps.iter().map(|(key, spec)| {
                spec.get("package")
                    .and_then(Value::as_str)
                    .unwrap_or(key.as_str())
            })
        })
        .collect()
}

/// The packages `manifest` declares that are among `members`.
fn workspace_dependencies<'a>(manifest: &'a Table, members: &[String]) -> Vec<&'a str> {
    declared_packages(manifest)
        .into_iter()
        .filter(|name| members.iter().any(|member| member == name))
        .collect()
}

fn read_manifest(path: &std::path::Path) -> Table {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    Table::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid TOML: {err}", path.display()))
}

/// The `[package] name` of every member the root manifest lists.
fn workspace_packages(root: &std::path::Path) -> Vec<String> {
    let manifest = read_manifest(&root.join("Cargo.toml"));
    let members = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(Value::as_array)
        .expect("the root Cargo.toml has no `workspace.members` array");
    members
        .iter()
        .map(|member| {
            let dir = member.as_str().expect("a workspace member is a string");
            let crate_manifest = read_manifest(&root.join(dir).join("Cargo.toml"));
            crate_manifest
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{dir}/Cargo.toml has no `package.name`"))
                .to_owned()
        })
        .collect()
}

#[test]
fn hume_core_depends_on_no_workspace_crate() {
    let root = arch_lints::workspace_root();
    let members = workspace_packages(&root);
    assert!(
        members.iter().any(|name| name == "hume-core"),
        "workspace_packages missed hume-core: Cargo.toml parsing broke"
    );
    let manifest = read_manifest(&root.join("hume-core/Cargo.toml"));
    let found = workspace_dependencies(&manifest, &members);
    assert!(
        found.is_empty(),
        "hume-core depends on workspace crates {found:?}; it may depend only on third-party crates"
    );
}

#[test]
fn declared_packages_reads_every_dependency_table() {
    let manifest = Table::from_str(
        "[package]\nname = \"x\"\n\n[dependencies]\nslotmap.workspace = true\nrope = { package = \"hume-rope\", path = \"../hume-rope\" }\n\n[dependencies.hume-grid]\npath = \"../hume-grid\"\n\n[dev-dependencies]\ntempfile = \"3\"\n\n[target.'cfg(unix)'.build-dependencies]\ncc = \"1\"\n\n[features]\ntest-util = []\n",
    )
    .expect("valid TOML");
    let mut names = declared_packages(&manifest);
    names.sort_unstable();
    assert_eq!(
        names,
        ["cc", "hume-grid", "hume-rope", "slotmap", "tempfile"]
    );
}

#[test]
fn workspace_dependencies_names_each_workspace_package_however_spelled() {
    let manifest = Table::from_str(
        "[package]\nname = \"x\"\n\n[dependencies]\nglobset = \"0.4\"\nrope = { package = \"hume-rope\", path = \"../hume-rope\" }\n\n[dev-dependencies.hume-grid]\npath = \"../hume-grid\"\n",
    )
    .expect("valid TOML");
    let members = ["hume-rope".to_owned(), "hume-grid".to_owned()];
    let mut names = workspace_dependencies(&manifest, &members);
    names.sort_unstable();
    assert_eq!(names, ["hume-grid", "hume-rope"]);
}
