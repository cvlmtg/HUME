use super::*;

fn id(name: &str) -> PluginId {
    PluginId::parse(name).unwrap()
}

fn loaded(n: isize) -> Option<SteelVal> {
    Some(SteelVal::IntV(n))
}

#[test]
fn note_keeps_the_first_spelling_and_order() {
    let mut records = PluginRecords::default();
    records.note(id("user/Foo"), None);
    records.note(id("core:b"), None);
    records.note(id("user/foo"), None);
    assert_eq!(
        records.plum_names().collect::<Vec<_>>(),
        ["user/Foo", "core:b"]
    );
}

#[test]
fn loaded_replaces_declared_and_declared_never_downgrades() {
    let mut records = PluginRecords::default();
    let p = id("user/p");
    records.note(p.clone(), None);
    assert!(!records.was_loaded(&p));
    assert!(records.config(&p).is_none());

    records.note(p.clone(), loaded(1));
    records.note(p.clone(), None);
    assert!(records.was_loaded(&p));
    assert!(matches!(records.config(&p), Some(SteelVal::IntV(1))));

    records.note(p.clone(), loaded(2));
    assert!(matches!(records.config(&p), Some(SteelVal::IntV(2))));
}

#[test]
fn plum_names_skip_local_files() {
    let mut records = PluginRecords::default();
    records.note(id("./mine.scm"), None);
    records.note(id("user/p"), None);
    assert_eq!(records.plum_names().collect::<Vec<_>>(), ["user/p"]);
}

#[test]
fn resolve_returns_the_previous_resolution() {
    let mut records = PluginRecords::default();
    let p = id("user/p");
    records.note(p.clone(), None);
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
        records.note(id(name), None);
        records.resolve(&id(name), res).unwrap();
    }
    records.note(id("user/d"), None);
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
