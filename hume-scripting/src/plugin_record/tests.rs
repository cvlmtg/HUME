use super::*;

fn id(name: &str) -> PluginId {
    PluginId::parse(name).unwrap()
}

fn loaded(n: isize) -> Request {
    Request::Loaded {
        config: SteelVal::IntV(n),
    }
}

#[test]
fn note_keeps_the_first_spelling_and_order() {
    let mut records = PluginRecords::default();
    records.note(id("user/Foo"), Request::Declared);
    records.note(id("core:b"), Request::Declared);
    records.note(id("user/foo"), Request::Declared);
    assert_eq!(
        records.plum_names().collect::<Vec<_>>(),
        ["user/Foo", "core:b"]
    );
}

#[test]
fn loaded_replaces_declared_and_declared_never_downgrades() {
    let mut records = PluginRecords::default();
    let p = id("user/p");
    records.note(p.clone(), Request::Declared);
    assert!(!records.was_loaded(&p));
    assert!(records.config(&p).is_none());

    records.note(p.clone(), loaded(1));
    records.note(p.clone(), Request::Declared);
    assert!(records.was_loaded(&p));
    assert!(matches!(records.config(&p), Some(SteelVal::IntV(1))));

    records.note(p.clone(), loaded(2));
    assert!(matches!(records.config(&p), Some(SteelVal::IntV(2))));
}

#[test]
fn plum_names_skip_local_files() {
    let mut records = PluginRecords::default();
    records.note(id("./mine.scm"), Request::Declared);
    records.note(id("user/p"), Request::Declared);
    assert_eq!(records.plum_names().collect::<Vec<_>>(), ["user/p"]);
}

#[test]
fn resolve_returns_the_previous_resolution() {
    let mut records = PluginRecords::default();
    let p = id("user/p");
    records.note(p.clone(), Request::Declared);
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
fn unresolved_rows_lists_only_absent_and_failed() {
    let mut records = PluginRecords::default();
    for (name, res) in [
        ("user/a", Resolution::Absent),
        ("user/b", Resolution::Declared),
        ("user/c", Resolution::ManifestFailed),
    ] {
        records.note(id(name), Request::Declared);
        records.resolve(&id(name), res).unwrap();
    }
    records.note(id("user/d"), Request::Declared);
    let rows: Vec<_> = records
        .unresolved_rows()
        .map(|(p, r)| (p.to_string(), r))
        .collect();
    assert_eq!(
        rows,
        [
            ("user/a".to_string(), Resolution::Absent),
            ("user/c".to_string(), Resolution::ManifestFailed)
        ]
    );
}
