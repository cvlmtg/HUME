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
/// meets `same_origin` at its own level.
fn expected(
    held: Option<ConfigOrigin>,
    by: ConfigOrigin,
    same_origin: WriteOutcome,
) -> WriteOutcome {
    let Some(held) = held else {
        return WriteOutcome::Applied;
    };
    if level(held) < level(by) {
        WriteOutcome::Applied
    } else if level(held) > level(by) {
        WriteOutcome::Shadowed
    } else {
        same_origin
    }
}

/// Every held origin (or none) and writing origin.
fn cases() -> impl Iterator<Item = (Option<ConfigOrigin>, ConfigOrigin)> {
    std::iter::once(None)
        .chain(ORIGINS.into_iter().map(Some))
        .flat_map(|held| ORIGINS.into_iter().map(move |by| (held, by)))
}

fn slot<P: TiePolicy>(held: Option<ConfigOrigin>) -> Ranked<&'static str, P> {
    Ranked {
        held: held.map(|by| ("old", by)),
        tie: PhantomData,
    }
}

fn rank_follows_the_rule<P: TiePolicy>(same_origin: WriteOutcome) {
    for (held, by) in cases() {
        assert_eq!(
            slot::<P>(held).rank(by),
            expected(held, by, same_origin),
            "{held:?} {by:?}"
        );
    }
}

fn claim_records_the_origin_only_when_the_write_applies<P: TiePolicy>(same_origin: WriteOutcome) {
    for (held, by) in cases() {
        let mut claim = slot::<P>(held).as_claim();

        let outcome = claim.claim(by);

        let want = expected(held, by, same_origin);
        assert_eq!(outcome, want, "{held:?} {by:?}");
        assert_eq!(claim.by(), if want.is_applied() { Some(by) } else { held });
    }
}

fn write_replaces_the_value_with_its_origin_only_when_it_applies<P: TiePolicy>(
    same_origin: WriteOutcome,
) {
    for (held, by) in cases() {
        let mut slot = slot::<P>(held);

        let written = slot.write("new", by);

        let want = expected(held, by, same_origin);
        assert_eq!(written.outcome, want, "{held:?} {by:?}");
        let (value, origin, displaced) = match (want.is_applied(), held) {
            (true, Some(_)) => ("new", Some(by), Some("old")),
            (true, None) => ("new", Some(by), None),
            (false, _) => ("old", held, None),
        };
        assert_eq!(written.displaced, displaced, "{held:?} {by:?}");
        assert_eq!(slot.by(), origin);
        assert_eq!(slot.get().copied(), origin.map(|_| value));
    }
}

fn clear_empties_the_slot_only_when_it_applies<P: TiePolicy>(same_origin: WriteOutcome) {
    for (held, by) in cases() {
        let mut slot = slot::<P>(held);

        let written = slot.clear(by);

        let want = expected(held, by, same_origin);
        assert_eq!(written.outcome, want, "{held:?} {by:?}");
        let (origin, displaced) = match want.is_applied() {
            true => (None, held.map(|_| "old")),
            false => (held, None),
        };
        assert_eq!(written.displaced, displaced, "{held:?} {by:?}");
        assert_eq!(slot.by(), origin);
        assert_eq!(slot.get().copied(), origin.map(|_| "old"));
    }
}

#[test]
fn rank_follows_the_rule_for_every_pair_under_each_policy() {
    rank_follows_the_rule::<FirstWins>(WriteOutcome::Ignored);
    rank_follows_the_rule::<LaterWins>(WriteOutcome::Applied);
}

#[test]
fn claim_records_the_origin_only_when_the_write_applies_under_each_policy() {
    claim_records_the_origin_only_when_the_write_applies::<FirstWins>(WriteOutcome::Ignored);
    claim_records_the_origin_only_when_the_write_applies::<LaterWins>(WriteOutcome::Applied);
}

#[test]
fn write_replaces_the_value_with_its_origin_only_when_it_applies_under_each_policy() {
    write_replaces_the_value_with_its_origin_only_when_it_applies::<FirstWins>(
        WriteOutcome::Ignored,
    );
    write_replaces_the_value_with_its_origin_only_when_it_applies::<LaterWins>(
        WriteOutcome::Applied,
    );
}

#[test]
fn clear_empties_the_slot_only_when_it_applies_under_each_policy() {
    clear_empties_the_slot_only_when_it_applies::<FirstWins>(WriteOutcome::Ignored);
    clear_empties_the_slot_only_when_it_applies::<LaterWins>(WriteOutcome::Applied);
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
