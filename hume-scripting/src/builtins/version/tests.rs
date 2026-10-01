use super::*;

fn int(n: usize) -> SteelVal {
    SteelVal::IntV(n as isize)
}

fn list_items(v: SteelVal) -> Vec<SteelVal> {
    match v {
        SteelVal::ListV(l) => l.into_iter().collect(),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn hume_version_lists_package_triple_then_commit() {
    let items = list_items(hume_version(&[]).unwrap());
    let [major, minor, patch, commit] = items.as_slice() else {
        panic!("expected 4 elements, got {items:?}");
    };
    let expected: Vec<isize> = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect();
    assert_eq!(
        [major, minor, patch],
        [
            &SteelVal::IntV(expected[0]),
            &SteelVal::IntV(expected[1]),
            &SteelVal::IntV(expected[2])
        ]
    );
    assert!(
        matches!(commit, SteelVal::StringV(s) if !s.is_empty())
            || *commit == SteelVal::BoolV(false),
        "commit must be a non-empty string or #f, got {commit:?}"
    );
}

#[test]
fn hume_version_rejects_args() {
    assert!(hume_version(&[int(1)]).is_err());
}

#[test]
fn hume_version_at_least_compares_against_running_version() {
    assert_eq!(
        hume_version_at_least(&[int(0), int(0), int(0)]).unwrap(),
        SteelVal::BoolV(true)
    );
    assert_eq!(
        hume_version_at_least(&[int(9999), int(0), int(0)]).unwrap(),
        SteelVal::BoolV(false)
    );
}

#[test]
fn hume_version_at_least_rejects_wrong_arity_and_types() {
    assert!(hume_version_at_least(&[int(0), int(0)]).is_err());
    assert!(hume_version_at_least(&[int(0), int(0), int(0), int(0)]).is_err());
    assert!(hume_version_at_least(&[int(0), SteelVal::StringV("x".into()), int(0)]).is_err());
}
