use super::*;

fn id(name: &str) -> PluginId {
    PluginId::parse(name).unwrap()
}

fn loaded(n: isize) -> Option<LoadCall> {
    Some(LoadCall {
        config: Rooted::new(SteelVal::IntV(n)),
        git_ref: None,
    })
}

#[test]
fn note_keeps_the_first_spelling_and_order() {
    let mut records = PluginRecords::default();
    records.note(&id("user/Foo"), None);
    records.note(&id("core:b"), None);
    records.note(&id("user/foo"), None);
    assert_eq!(
        records.plum_names().collect::<Vec<_>>(),
        ["user/Foo", "core:b"]
    );
}

/// A declared entry is found under any casing of its plugin, and only for
/// the entry file it named.
#[test]
fn declared_names_the_entry_whatever_the_plugin_casing() {
    use crate::attribution::{EntryFile, EntryId};

    let mut records = PluginRecords::default();
    records.note_declared(&EntryId::main(id("user/Foo")));
    assert!(records.declared(&EntryId::main(id("user/foo"))));
    let extra = EntryId::new(id("user/foo"), EntryFile::parse("extra.scm").unwrap());
    assert!(!records.declared(&extra));
    assert_eq!(records.plum_names().collect::<Vec<_>>(), ["user/Foo"]);
}

#[test]
fn loaded_replaces_declared_and_declared_never_downgrades() {
    let mut records = PluginRecords::default();
    let p = id("user/p");
    records.note(&p, None);
    assert!(!records.was_loaded(&p));
    assert!(records.config(&p).is_none());

    records.note(&p, loaded(1));
    records.note(&p, None);
    assert!(records.was_loaded(&p));
    assert!(matches!(records.config(&p), Some(SteelVal::IntV(1))));

    records.note(&p, loaded(2));
    assert!(matches!(records.config(&p), Some(SteelVal::IntV(2))));
}

#[test]
fn plum_names_skip_local_files() {
    let mut records = PluginRecords::default();
    records.note(&id("./mine.scm"), None);
    records.note(&id("user/p"), None);
    assert_eq!(records.plum_names().collect::<Vec<_>>(), ["user/p"]);
}

#[test]
fn resolve_returns_the_previous_resolution() {
    let mut records = PluginRecords::default();
    let p = id("user/p");
    records.note(&p, None);
    assert_eq!(records.resolution(&p), None);
    assert_eq!(records.resolve(&p, Resolution::Absent), Ok(None));
    assert_eq!(
        records.resolve(&p, Resolution::Absent),
        Ok(Some(Resolution::Absent))
    );
    assert!(
        records
            .resolve(&id("user/other"), Resolution::Absent)
            .is_err()
    );
}

#[test]
fn entryless_rows_lists_absent_and_failed_with_their_labels() {
    let mut records = PluginRecords::default();
    for (name, res) in [
        ("user/a", Resolution::Absent),
        ("user/c", Resolution::ManifestFailed),
    ] {
        records.note(&id(name), None);
        records.resolve(&id(name), res).unwrap();
    }
    records.note(&id("user/d"), None);
    let rows: Vec<_> = records
        .entryless_rows()
        .map(|(p, state)| (p.to_string(), state))
        .collect();
    assert_eq!(
        rows,
        [
            ("user/a".to_string(), "absent"),
            ("user/c".to_string(), "failed")
        ]
    );
}
