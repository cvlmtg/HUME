use super::*;
use hume_core::{ListTier, LspFeature};

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

fn register(registry: &mut Registry, server: &str) -> RegistryWrite {
    register_from(registry, server, ConfigOrigin::User)
}

fn register_from(registry: &mut Registry, server: &str, origin: ConfigOrigin) -> RegistryWrite {
    registry.register(name(server), config(server), origin)
}

fn entry(server: &str, filter: FeatureFilter) -> ListEntry {
    ListEntry {
        name: name(server),
        filter,
    }
}

/// Sets `language`'s explicit list from `origin` to `servers`, each
/// unfiltered.
fn list(registry: &mut Registry, language: &str, origin: ConfigOrigin, servers: &[&str]) {
    registry.set_list(
        language.to_string(),
        ListTier::Explicit,
        origin,
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
    list(&mut registry, "typescript", ConfigOrigin::User, &["tsls"]);
    list(&mut registry, "tsx", ConfigOrigin::Plugin, &["tsls"]);
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
    list(&mut registry, "python", ConfigOrigin::Plugin, &["ty"]);
    assert_eq!(names(&registry, "python"), vec!["ty"]);
    assert!(registry.languages_of(&name("ruff")).is_empty());
}

#[test]
fn reregistering_a_name_reports_the_replacement() {
    let mut registry = Registry::default();
    assert_eq!(register(&mut registry, "ty"), RegistryWrite::Unregistered);
    assert_eq!(register(&mut registry, "ty"), RegistryWrite::Replaced);
}

#[test]
fn a_lower_origin_neither_replaces_nor_removes_a_higher_origins_registration() {
    let mut registry = Registry::default();
    register_from(&mut registry, "ty", ConfigOrigin::User);
    assert_eq!(
        register_from(&mut registry, "ty", ConfigOrigin::Plugin),
        RegistryWrite::Shadowed
    );
    assert_eq!(
        registry.unregister(&name("ty"), ConfigOrigin::Plugin),
        RegistryWrite::Shadowed
    );
    assert_eq!(registry.origin(&name("ty")), Some(ConfigOrigin::User));
    assert_eq!(registry.get(&name("ty")), Some(&config("ty")));
}

#[test]
fn a_higher_origin_replaces_a_lower_origins_registration() {
    let mut registry = Registry::default();
    register_from(&mut registry, "ty", ConfigOrigin::Plugin);
    assert_eq!(
        register_from(&mut registry, "ty", ConfigOrigin::User),
        RegistryWrite::Replaced
    );
    assert_eq!(registry.origin(&name("ty")), Some(ConfigOrigin::User));
}

#[test]
fn a_list_may_name_any_registered_server() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "taplo");
    list(
        &mut registry,
        "python",
        ConfigOrigin::User,
        &["taplo", "ty"],
    );
    assert_eq!(names(&registry, "python"), vec!["taplo", "ty"]);
}

#[test]
fn a_list_is_exact_and_excludes_unlisted() {
    let mut registry = Registry::default();
    for server in ["ty", "ruff", "jedi"] {
        register(&mut registry, server);
    }
    list(&mut registry, "python", ConfigOrigin::User, &["ruff", "ty"]);
    assert_eq!(names(&registry, "python"), vec!["ruff", "ty"]);
}

#[test]
fn a_higher_origins_list_beats_a_lower_ones_in_either_order() {
    for (higher, lower) in [
        (ConfigOrigin::User, ConfigOrigin::Plugin),
        (ConfigOrigin::Plugin, ConfigOrigin::Bundled),
    ] {
        for higher_first in [true, false] {
            let mut registry = Registry::default();
            register(&mut registry, "ty");
            register(&mut registry, "ruff");
            if higher_first {
                list(&mut registry, "python", higher, &["ty"]);
                list(&mut registry, "python", lower, &["ruff"]);
            } else {
                list(&mut registry, "python", lower, &["ruff"]);
                list(&mut registry, "python", higher, &["ty"]);
            }
            assert_eq!(
                names(&registry, "python"),
                vec!["ty"],
                "{higher:?} over {lower:?}, higher_first={higher_first}"
            );
        }
    }
}

#[test]
fn a_later_list_from_one_origin_replaces_the_earlier() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    list(&mut registry, "python", ConfigOrigin::Plugin, &["ty"]);
    list(&mut registry, "python", ConfigOrigin::Plugin, &["ruff"]);
    assert_eq!(names(&registry, "python"), vec!["ruff"]);
}

#[test]
fn clearing_a_list_falls_back_to_the_next_lower_origins() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    list(&mut registry, "python", ConfigOrigin::Bundled, &["ruff"]);
    list(&mut registry, "python", ConfigOrigin::User, &["ty"]);
    registry.set_list(
        "python".to_string(),
        ListTier::Explicit,
        ConfigOrigin::User,
        None,
    );
    assert_eq!(names(&registry, "python"), vec!["ruff"]);
}

#[test]
fn clearing_a_lower_origins_list_leaves_a_higher_one() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    list(&mut registry, "python", ConfigOrigin::Plugin, &["ruff"]);
    list(&mut registry, "python", ConfigOrigin::User, &["ty"]);
    registry.set_list(
        "python".to_string(),
        ListTier::Explicit,
        ConfigOrigin::Plugin,
        None,
    );
    assert_eq!(names(&registry, "python"), vec!["ty"]);
}

/// Sets `language`'s default list from `origin` to `servers`, each
/// unfiltered.
fn default_list(registry: &mut Registry, language: &str, origin: ConfigOrigin, servers: &[&str]) {
    let entries = servers
        .iter()
        .map(|server| entry(server, FeatureFilter::All))
        .collect();
    registry.set_list(
        language.to_string(),
        ListTier::Default,
        origin,
        Some(entries),
    );
}

#[test]
fn a_default_list_applies_only_while_no_explicit_list_is_set() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    default_list(&mut registry, "python", ConfigOrigin::User, &["ruff"]);
    assert_eq!(names(&registry, "python"), vec!["ruff"]);

    list(&mut registry, "python", ConfigOrigin::Bundled, &["ty"]);
    assert_eq!(names(&registry, "python"), vec!["ty"]);

    registry.set_list(
        "python".to_string(),
        ListTier::Explicit,
        ConfigOrigin::Bundled,
        None,
    );
    assert_eq!(names(&registry, "python"), vec!["ruff"]);
}

#[test]
fn among_default_lists_the_highest_origin_applies() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    register(&mut registry, "ruff");
    default_list(&mut registry, "python", ConfigOrigin::User, &["ty"]);
    default_list(&mut registry, "python", ConfigOrigin::Plugin, &["ruff"]);
    assert_eq!(names(&registry, "python"), vec!["ty"]);
}

#[test]
fn set_list_reports_a_higher_origins_list_in_the_same_tier() {
    let mut registry = Registry::default();
    let set = |registry: &mut Registry, tier, origin| {
        registry.set_list(
            "python".to_string(),
            tier,
            origin,
            Some(vec![entry("ty", FeatureFilter::All)]),
        )
    };
    assert!(!set(
        &mut registry,
        ListTier::Explicit,
        ConfigOrigin::Plugin
    ));
    assert!(!set(&mut registry, ListTier::Explicit, ConfigOrigin::User));
    assert!(set(&mut registry, ListTier::Explicit, ConfigOrigin::Plugin));
    assert!(
        !set(&mut registry, ListTier::Default, ConfigOrigin::Plugin),
        "an explicit list does not hide a default write"
    );
}

#[test]
fn list_entry_naming_an_unregistered_server_applies_once_it_registers() {
    let mut registry = Registry::default();
    register(&mut registry, "ty");
    list(&mut registry, "python", ConfigOrigin::User, &["ruff", "ty"]);
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
        ListTier::Explicit,
        ConfigOrigin::User,
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
        ConfigOrigin::User,
        &["tsls", "eslint"],
    );
    list(&mut registry, "tsx", ConfigOrigin::User, &["tsls"]);
    assert_eq!(
        registry.unregister(&name("tsls"), ConfigOrigin::User),
        RegistryWrite::Replaced
    );
    assert_eq!(
        registry.unregister(&name("tsls"), ConfigOrigin::User),
        RegistryWrite::Unregistered
    );
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
