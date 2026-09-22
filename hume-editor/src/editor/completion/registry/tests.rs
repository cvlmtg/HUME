use super::*;

fn steel_entry(name: &str, target: SourceTarget) -> SourceEntry {
    SourceEntry {
        name: name.into(),
        match_kind: MatchKind::Fuzzy,
        priority: 0,
        body: SourceBody::Steel {
            proc: SteelVal::Void,
            target,
        },
    }
}

#[test]
fn register_under_a_fresh_name_adds() {
    let mut reg = SourceRegistry::with_defaults();
    let before = reg.entries.len();
    let outcome = reg.register(steel_entry(
        "my-source",
        SourceTarget::Buffer(BufferToken::Word),
    ));
    assert!(matches!(outcome, RegisterOutcome::Added));
    assert_eq!(reg.entries.len(), before + 1);
}

#[test]
fn register_under_a_same_target_name_replaces_in_place() {
    let mut reg = SourceRegistry::with_defaults();
    let id_before = reg.id_of("path").expect("native path source registered");
    let outcome = reg.register(steel_entry(
        "path",
        SourceTarget::Minibuf(MinibufToken::Custom),
    ));
    assert!(matches!(outcome, RegisterOutcome::Replaced));
    let id_after = reg.id_of("path").expect("still registered");
    assert_eq!(
        id_before, id_after,
        "SourceId must stay stable across a same-target replace"
    );
    assert!(matches!(reg.get(id_after).body, SourceBody::Steel { .. }));
}

/// A native minibuffer source (`"path"`, `"command"`, `"set"`, …) replaced
/// by a Steel `'buffer` source under the same name would silently break
/// `:e`'s path completion — the registry must refuse the write rather than
/// clobber it.
#[test]
fn register_under_a_taken_name_with_a_different_target_is_refused() {
    let mut reg = SourceRegistry::with_defaults();
    let original_id = reg.id_of("path").expect("native path source registered");
    let outcome = reg.register(steel_entry("path", SourceTarget::Buffer(BufferToken::Word)));
    match outcome {
        RegisterOutcome::TargetMismatch(existing) => {
            assert_eq!(existing, SourceTarget::Minibuf(MinibufToken::Custom));
        }
        other => panic!(
            "expected TargetMismatch, got a different outcome: entries len check below\n{other:?}"
        ),
    }
    // The native entry must be untouched.
    let id_after = reg.id_of("path").expect("still registered");
    assert_eq!(original_id, id_after);
    assert!(matches!(
        reg.get(id_after).body,
        SourceBody::NativeDelegated(_)
    ));
}
