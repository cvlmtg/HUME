use super::*;
use crate::test_support::hash_field;

fn int(n: usize) -> SteelVal {
    SteelVal::IntV(n as isize)
}

#[test]
fn hume_version_names_package_triple_and_commit() {
    let version = hume_version(&[]).unwrap();
    let expected: Vec<isize> = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect();
    assert_eq!(
        [
            hash_field(&version, "major"),
            hash_field(&version, "minor"),
            hash_field(&version, "patch")
        ],
        [
            SteelVal::IntV(expected[0]),
            SteelVal::IntV(expected[1]),
            SteelVal::IntV(expected[2])
        ]
    );
    let commit = hash_field(&version, "commit");
    assert!(
        matches!(&commit, SteelVal::StringV(s) if !s.is_empty())
            || commit == SteelVal::BoolV(false),
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
