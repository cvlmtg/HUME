use super::*;

#[test]
fn edges_are_half_open() {
    let r = Rect::new(2, 3, 10, 5);
    assert_eq!((r.left(), r.top()), (2, 3));
    assert_eq!((r.right(), r.bottom()), (12, 8));
}

#[test]
fn edges_saturate_rather_than_wrapping() {
    let r = Rect::new(u16::MAX - 1, u16::MAX - 1, 10, 10);
    assert_eq!(r.right(), u16::MAX);
    assert_eq!(r.bottom(), u16::MAX);
}

#[test]
fn area_widens_past_u16() {
    // 65535 * 65535 overflows u16 and even u16::MAX as a count; the answer
    // has to survive as a real number.
    assert_eq!(Rect::new(0, 0, 1000, 1000).area(), 1_000_000);
}

#[test]
fn empty_when_either_axis_is_zero() {
    assert!(Rect::new(0, 0, 0, 5).is_empty());
    assert!(Rect::new(0, 0, 5, 0).is_empty());
    assert!(!Rect::new(0, 0, 1, 1).is_empty());
}

#[test]
fn contains_includes_the_top_left_and_excludes_the_bottom_right() {
    let r = Rect::new(2, 3, 4, 4); // x 2..6, y 3..7
    assert!(r.contains(Position::new(2, 3)));
    assert!(r.contains(Position::new(5, 6)));
    assert!(!r.contains(Position::new(6, 6)));
    assert!(!r.contains(Position::new(5, 7)));
    assert!(!r.contains(Position::new(1, 3)));
    assert!(!r.contains(Position::new(2, 2)));
}

#[test]
fn empty_rect_contains_nothing() {
    assert!(!Rect::new(4, 4, 0, 0).contains(Position::new(4, 4)));
}

#[test]
fn inset_shrinks_both_sides() {
    // Oracle: a 1-cell border around a 10x5 box leaves 8x3 at (3, 4).
    assert_eq!(Rect::new(2, 3, 10, 5).inset(1, 1), Rect::new(3, 4, 8, 3));
}

#[test]
fn inset_asymmetric_axes() {
    assert_eq!(Rect::new(0, 0, 10, 10).inset(2, 3), Rect::new(2, 3, 6, 4));
}

#[test]
fn inset_past_the_rect_yields_empty_not_wrapped() {
    // The case a hand-written `width - 2` gets wrong: it wraps to 65535.
    let r = Rect::new(5, 5, 1, 1).inset(1, 1);
    assert!(r.is_empty());
    assert_eq!((r.width, r.height), (0, 0));
}

#[test]
fn centered_places_evenly() {
    // Oracle: 10 wide inside 20 leaves 5 on each side.
    assert_eq!(
        Rect::new(0, 0, 20, 10).centered(10, 4),
        Rect::new(5, 3, 10, 4)
    );
}

#[test]
fn centered_biases_up_and_left_on_odd_leftover() {
    // 20 - 9 = 11 leftover; 5 left, 6 right.
    assert_eq!(
        Rect::new(0, 0, 20, 10).centered(9, 3),
        Rect::new(5, 3, 9, 3)
    );
}

#[test]
fn centered_respects_the_outer_origin() {
    assert_eq!(
        Rect::new(4, 6, 20, 10).centered(10, 4),
        Rect::new(9, 9, 10, 4)
    );
}

#[test]
fn centered_clamps_to_the_outer_rect() {
    assert_eq!(
        Rect::new(2, 2, 6, 4).centered(100, 100),
        Rect::new(2, 2, 6, 4)
    );
}
