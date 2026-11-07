use super::*;

#[test]
fn a_share_taking_some_cells_reads_back_as_those_cells() {
    for space in 2..=1000u16 {
        for cells in 1..space {
            assert_eq!(
                SplitShare::taking(cells, space).of(space),
                cells,
                "{cells} of {space}"
            );
        }
    }
}

/// Each member's width in `split` laid out over `usable` columns, no seams.
fn member_widths(split: &Split, usable: u16) -> Vec<u16> {
    let area = Rect {
        x: 0,
        y: 0,
        width: usable,
        height: 1,
    };
    split
        .member_rects(area, false)
        .map(|(rect, _)| rect.width)
        .collect()
}

/// Panics when a member of `split` is narrower one column wider.
fn assert_no_member_shrinks_as_it_grows(split: &Split) {
    for usable in 0..500u16 {
        let (was, now) = (
            member_widths(split, usable),
            member_widths(split, usable + 1),
        );
        assert!(
            was.iter().zip(&now).all(|(was, now)| now >= was),
            "{:?}: {was:?} at {usable} became {now:?}",
            split.seams
        );
    }
}

#[test]
fn an_equal_split_never_shrinks_a_member_as_it_grows() {
    for members in 2..=8usize {
        assert_no_member_shrinks_as_it_grows(&split_of(members, Seams::Equal));
    }
}

fn split_of(members: usize, seams: Seams) -> Split {
    Split {
        direction: Direction::Horizontal,
        members: vec![LayoutTree::Leaf(PaneId::default()); members],
        seams,
    }
}

#[test]
fn an_equal_split_gives_its_leftover_cells_to_its_last_members() {
    for members in 2..=8usize {
        let split = split_of(members, Seams::Equal);
        for usable in 0..=500u16 {
            let (each, left) = (usable / members as u16, usize::from(usable) % members);
            let mut expected = vec![each; members];
            for width in &mut expected[members - left..] {
                *width += 1;
            }
            assert_eq!(
                member_widths(&split, usable),
                expected,
                "{members} in {usable}"
            );
        }
    }
}

#[test]
fn an_equal_split_written_out_as_shares_keeps_its_cells() {
    for members in 2..=8usize {
        let equal = split_of(members, Seams::Equal);
        let placed = split_of(members, Seams::Placed(Seams::Equal.shares(members)));
        for usable in 0..=1000u16 {
            assert_eq!(
                member_widths(&placed, usable),
                member_widths(&equal, usable),
                "{members} in {usable}"
            );
        }
    }
}

#[test]
fn a_placed_split_never_shrinks_a_member_as_it_grows() {
    let share = |bits: u32| SplitShare::from_bits(NonZeroU32::new(bits).expect("not zero"));
    for shares in [
        vec![share(0x2000_0000), share(0x5555_5555), share(0xC000_0000)],
        vec![share(0x3333_3333); 4],
        vec![share(1), share(u32::MAX), share(0x8000_0001)],
        vec![
            share(0x1234_5678),
            share(0x9ABC_DEF0),
            share(0x0F0F_0F0F),
            share(0x7777_7777),
        ],
    ] {
        assert_no_member_shrinks_as_it_grows(&split_of(shares.len() + 1, Seams::Placed(shares)));
    }
}

#[test]
fn a_split_share_rebuilds_the_seam_it_was_taken_from() {
    for lo in [0, 5, ONE / 3, ONE - 7] {
        for at in [lo + 1, lo + (ONE - lo) / 2, ONE - 1] {
            assert_eq!(SplitShare::placing(lo, at).within(lo, ONE), at);
        }
    }
}

#[test]
#[should_panic(expected = "one share for each member but the last")]
fn shares_that_do_not_suit_the_members_are_refused() {
    let mut split = split_of(3, Seams::Equal);

    split.set_seams(Seams::Placed(vec![SplitShare::HALF]));
}
