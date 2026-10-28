use super::*;
use crate::test_support::hash_field;

fn int(n: usize) -> SteelVal {
    SteelVal::IntV(n as isize)
}

#[test]
fn hume_version_names_package_triple_and_commit() {
    let version = hume_version().unwrap();
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

/// Both builtins refuse a wrong argument count through real registration,
/// with Steel's message naming the builtin.
#[test]
fn version_builtins_reject_a_wrong_argument_count() {
    for (expr, message) in [
        (
            "(hume-version 1)",
            "hume-version expected 0 arguments, got 1",
        ),
        (
            "(hume-version>=? 0 0)",
            "hume-version>=? expected 3 argument, got 2",
        ),
        (
            "(hume-version>=? 0 0 0 0)",
            "hume-version>=? expected 3 argument, got 4",
        ),
    ] {
        let mut host = crate::ScriptingHost::new(&hume_platform::dirs::Dirs::none());
        let err = host
            .eval_source(expr, &mut crate::null_host::NullHost)
            .expect_err(expr);
        assert!(err.contains("ArityMismatch"), "{expr}: {err}");
        assert!(err.contains(message), "{expr}: {err}");
    }
}

#[test]
fn hume_version_at_least_compares_against_running_version() {
    assert_eq!(
        hume_version_at_least(int(0), int(0), int(0)).unwrap(),
        SteelVal::BoolV(true)
    );
    assert_eq!(
        hume_version_at_least(int(9999), int(0), int(0)).unwrap(),
        SteelVal::BoolV(false)
    );
}

#[test]
fn hume_version_at_least_rejects_a_non_integer_component() {
    let err = hume_version_at_least(int(0), SteelVal::StringV("x".into()), int(0))
        .expect_err("a string component raises");
    assert!(err.to_string().contains("minor"), "{err}");
}
