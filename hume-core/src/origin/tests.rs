use super::*;

const ORIGINS: [ConfigOrigin; 3] = [
    ConfigOrigin::Bundled,
    ConfigOrigin::Plugin,
    ConfigOrigin::User,
];

fn level(origin: ConfigOrigin) -> u8 {
    match origin {
        ConfigOrigin::Bundled => 0,
        ConfigOrigin::Plugin => 1,
        ConfigOrigin::User => 2,
    }
}

/// The rule stated apart from `rank`'s own comparison: a write applies to an
/// empty slot or one a lower level set, is shadowed by a higher level, and
/// meets its own level's tie rule.
fn expected(held: Option<ConfigOrigin>, by: ConfigOrigin, tie: Tie) -> WriteOutcome {
    let Some(held) = held else {
        return WriteOutcome::Applied;
    };
    if level(held) < level(by) {
        WriteOutcome::Applied
    } else if level(held) > level(by) {
        WriteOutcome::Shadowed
    } else if tie == Tie::LaterWins {
        WriteOutcome::Applied
    } else {
        WriteOutcome::Ignored
    }
}

/// Every held origin (or none), writing origin and tie.
fn cases() -> impl Iterator<Item = (Option<ConfigOrigin>, ConfigOrigin, Tie)> {
    std::iter::once(None)
        .chain(ORIGINS.into_iter().map(Some))
        .flat_map(|held| ORIGINS.into_iter().map(move |by| (held, by)))
        .flat_map(|(held, by)| [Tie::FirstWins, Tie::LaterWins].map(|tie| (held, by, tie)))
}

#[test]
fn rank_follows_the_rule_for_every_pair_and_tie() {
    for (held, by, tie) in cases() {
        assert_eq!(
            by.rank(held, tie),
            expected(held, by, tie),
            "{held:?} {by:?} {tie:?}"
        );
    }
}

#[test]
fn claim_records_the_origin_only_when_the_write_applies() {
    for (held, by, tie) in cases() {
        let mut slot = held;

        let outcome = by.claim(&mut slot, tie);

        let want = expected(held, by, tie);
        assert_eq!(outcome, want);
        assert_eq!(slot, if want.is_applied() { Some(by) } else { held });
    }
}

#[test]
fn ranked_write_replaces_the_value_with_its_origin_only_when_it_applies() {
    for (held, by, tie) in cases() {
        let mut slot = held.map(|by| Ranked { value: "old", by });

        let outcome = Ranked::write(&mut slot, "new", by, tie);

        let want = expected(held, by, tie);
        assert_eq!(outcome, want);
        let (value, origin) = match want.is_applied() {
            true => ("new", Some(by)),
            false => ("old", held),
        };
        assert_eq!(slot.as_ref().map(|r| r.by), origin);
        assert_eq!(slot.as_ref().map(|r| r.value), origin.map(|_| value));
    }
}

#[test]
fn by_origin_iterates_highest_first_and_keeps_each_value_apart() {
    let mut layers = ByOrigin::<u8>::default();
    for origin in ORIGINS {
        *layers.get_mut(origin) = level(origin) + 10;
    }

    let seen: Vec<_> = layers
        .iter()
        .map(|(origin, value)| (origin, *value))
        .collect();

    assert_eq!(
        seen,
        [
            (ConfigOrigin::User, 12),
            (ConfigOrigin::Plugin, 11),
            (ConfigOrigin::Bundled, 10),
        ]
    );
}

#[test]
fn by_origin_above_is_every_higher_origin_highest_first() {
    let mut layers = ByOrigin::<u8>::default();
    for origin in ORIGINS {
        *layers.get_mut(origin) = level(origin) + 10;
    }

    let above = |origin| layers.above(origin).copied().collect::<Vec<_>>();

    assert_eq!(above(ConfigOrigin::Bundled), [12, 11]);
    assert_eq!(above(ConfigOrigin::Plugin), [12]);
    assert_eq!(above(ConfigOrigin::User), Vec::<u8>::new());
}
