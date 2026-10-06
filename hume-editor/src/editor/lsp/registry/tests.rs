use super::*;
use hume_scripting::LspFeature;

fn touch(path: &Path) {
    std::fs::write(path, b"").unwrap();
}

#[test]
fn marker_in_immediate_parent_wins() {
    let tmp = tempfile::tempdir().unwrap();
    touch(&tmp.path().join("Cargo.toml"));
    let file = tmp.path().join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);

    let root = resolve_root(
        &file,
        &["Cargo.toml".to_string()],
        &PathBuf::from("/should-not-be-used"),
    );
    assert_eq!(root, tmp.path());
}

#[test]
fn marker_in_grandparent_is_found_by_walking_up() {
    let tmp = tempfile::tempdir().unwrap();
    touch(&tmp.path().join("Cargo.toml"));
    let file = tmp.path().join("src/nested/deep.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);

    let root = resolve_root(&file, &["Cargo.toml".to_string()], &PathBuf::from("/cwd"));
    assert_eq!(root, tmp.path());
}

#[test]
fn no_marker_anywhere_falls_back_to_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);
    let cwd = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&cwd).unwrap();

    let root = resolve_root(&file, &["Cargo.toml".to_string()], &cwd);
    assert_eq!(root, cwd);
}

#[test]
fn directory_marker_like_dot_git_is_matched() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    let file = tmp.path().join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);

    let root = resolve_root(&file, &[".git".to_string()], &PathBuf::from("/cwd"));
    assert_eq!(root, tmp.path());
}

fn name(s: &str) -> ServerName {
    ServerName::parse(s).expect("valid test server name")
}

fn config(command: &str) -> LspServerConfig {
    LspServerConfig {
        command: command.to_string(),
        args: Vec::new(),
        init_options: None,
        settings: None,
        env: Vec::new(),
    }
}

fn register(registry: &mut Registry, server: &str) -> bool {
    registry.register(name(server), config(server))
}

fn entry(server: &str, filter: FeatureFilter) -> ListEntry {
    ListEntry {
        name: name(server),
        filter,
    }
}

/// Sets `language`'s `layer` list to `servers`, each unfiltered.
fn list(registry: &mut Registry, language: &str, layer: ListLayer, servers: &[&str]) {
    registry.set_list(
        language.to_string(),
        layer,
        Some(
            servers
                .iter()
                .map(|server| entry(server, FeatureFilter::All))
                .collect(),
        ),
    );
}

fn planned(registry: &Registry, language: &str) -> Vec<(String, FeatureFilter)> {
    registry
        .plan(language)
        .into_iter()
        .map(|p| (p.name.to_string(), p.filter))
        .collect()
}

fn names(registry: &Registry, language: &str) -> Vec<String> {
    planned(registry, language)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

fn only(features: &[LspFeature]) -> FeatureFilter {
    FeatureFilter::Only(features.iter().copied().collect())
}

#[test]
fn a_server_serves_the_languages_whose_lists_name_it() {
    let mut registry = Registry::default();
    register(&mut registry, "tsls");
    list(&mut registry, "typescript", ListLayer::User, &["tsls"]);
    list(&mut registry, "tsx", ListLayer::Default, &["tsls"]);
    assert_eq!(names(&registry, "typescript"), vec!["tsls"]);
    assert_eq!(names(&registry, "tsx"), vec!["tsls"]);
    assert!(names(&registry, "rust").is_empty());
    assert_eq!(
        registry.languages_of(&name("tsls")),
        vec!["tsx", "typescript"]
    );
}

#[test]
fn a_registered_server_no_list_names_serves_nothing() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    list(&mut registry, "python", ListLayer::Default, &["ty"]);
    assert_eq!(names(&registry, "python"), vec!["ty"]);
    assert!(registry.languages_of(&name("ruff")).is_empty());
}

#[test]
fn reregistering_a_name_reports_the_replacement() {
    let mut registry = Registry::default();
    assert!(!register(&mut registry, "ty"));
    assert!(register(&mut registry, "ty"));
}

#[test]
fn a_list_may_name_any_registered_server() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "taplo");
    list(&mut registry, "python", ListLayer::User, &["taplo", "ty"]);
    assert_eq!(names(&registry, "python"), vec!["taplo", "ty"]);
}

#[test]
fn user_list_is_exact_and_excludes_unlisted() {
    let mut registry = Registry::default();
    for server in ["ty", "ruff", "jedi"] {
        register(&mut registry, server);
    }
    list(&mut registry, "python", ListLayer::User, &["ruff", "ty"]);
    assert_eq!(names(&registry, "python"), vec!["ruff", "ty"]);
}

#[test]
fn default_list_is_exact_and_excludes_unlisted() {
    let mut registry = Registry::default();
    for server in ["jedi", "ty", "ruff"] {
        register(&mut registry, server);
    }
    list(&mut registry, "python", ListLayer::Default, &["ruff", "ty"]);
    assert_eq!(names(&registry, "python"), vec!["ruff", "ty"]);
}

#[test]
fn user_list_beats_default_regardless_of_call_order() {
    for user_first in [true, false] {
        let mut registry = Registry::default();
        register(&mut registry, "ty");
        register(&mut registry, "ruff");
        if user_first {
            list(&mut registry, "python", ListLayer::User, &["ty"]);
            list(&mut registry, "python", ListLayer::Default, &["ruff"]);
        } else {
            list(&mut registry, "python", ListLayer::Default, &["ruff"]);
            list(&mut registry, "python", ListLayer::User, &["ty"]);
        }
        assert_eq!(
            names(&registry, "python"),
            vec!["ty"],
            "user_first={user_first}"
        );
    }
}

#[test]
fn clearing_the_user_list_falls_back_to_the_default_list() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    list(&mut registry, "python", ListLayer::Default, &["ruff"]);
    list(&mut registry, "python", ListLayer::User, &["ty"]);
    registry.set_list("python".to_string(), ListLayer::User, None);
    assert_eq!(names(&registry, "python"), vec!["ruff"]);
}

#[test]
fn list_entry_naming_an_unregistered_server_applies_once_it_registers() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    list(&mut registry, "python", ListLayer::User, &["ruff", "ty"]);
    assert_eq!(names(&registry, "python"), vec!["ty"]);
    register(&mut registry, "ruff");
    assert_eq!(names(&registry, "python"), vec!["ruff", "ty"]);
}

#[test]
fn only_and_except_filters_reach_the_effective_list() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    let except = FeatureFilter::Except([LspFeature::Hover].into_iter().collect());
    registry.set_list(
        "python".to_string(),
        ListLayer::User,
        Some(vec![
            entry("ty", except),
            entry("ruff", only(&[LspFeature::Format, LspFeature::Diagnostics])),
        ]),
    );
    assert_eq!(
        planned(&registry, "python"),
        vec![
            ("ty".to_string(), except),
            (
                "ruff".to_string(),
                only(&[LspFeature::Format, LspFeature::Diagnostics])
            ),
        ]
    );
}

#[test]
fn unregister_removes_the_server_from_every_language() {
    let mut registry = Registry::default();
    register(&mut registry, "tsls");
    register(&mut registry, "eslint");
    list(
        &mut registry,
        "typescript",
        ListLayer::User,
        &["tsls", "eslint"],
    );
    list(&mut registry, "tsx", ListLayer::User, &["tsls"]);
    assert!(registry.unregister(&name("tsls")));
    assert!(!registry.unregister(&name("tsls")));
    assert_eq!(names(&registry, "typescript"), vec!["eslint"]);
    assert!(names(&registry, "tsx").is_empty());
    assert!(!registry.contains(&name("tsls")));
}

#[test]
fn a_root_cache_walks_once_per_directory_and_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("web");
    let file = project.join("src/app.ts");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);
    touch(&project.join("package.json"));
    let markers = vec!["package.json".to_string()];

    let mut cache = RootCache::default();
    assert_eq!(cache.root_for(&markers, &file, tmp.path()), project);
    std::fs::remove_file(project.join("package.json")).unwrap();
    assert_eq!(
        cache.root_for(&markers, &file, tmp.path()),
        project,
        "the same directory and markers reuse the first walk"
    );
    assert_eq!(
        RootCache::default().root_for(&markers, &file, tmp.path()),
        tmp.path(),
        "a fresh cache walks again"
    );
}

#[test]
fn a_language_without_roots_resolves_to_the_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("web/src/app.ts");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    touch(&file);
    assert_eq!(
        RootCache::default().root_for(&[], &file, tmp.path()),
        tmp.path()
    );
}
