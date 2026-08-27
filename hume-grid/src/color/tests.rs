use super::*;

#[test]
fn lerp_zero_factor_is_identity() {
    let c = Rgb(10, 20, 30);
    assert_eq!(c.lerp(Rgb(200, 200, 200), 0.0), c);
}

#[test]
fn lerp_full_factor_reaches_target() {
    let target = Rgb(200, 100, 50);
    assert_eq!(Rgb(10, 20, 30).lerp(target, 1.0), target);
}

#[test]
fn lerp_half_factor_is_the_midpoint() {
    // Oracle: the midpoint of each channel, computed by hand rather than
    // through the implementation's own expression.
    assert_eq!(
        Rgb(0, 100, 255).lerp(Rgb(100, 200, 255), 0.5),
        Rgb(50, 150, 255)
    );
}

#[test]
fn lerp_rounds_rather_than_truncating() {
    // 0 -> 1 at 0.5 is 0.5, which truncation would floor to 0.
    assert_eq!(Rgb(0, 0, 0).lerp(Rgb(1, 1, 1), 0.5), Rgb(1, 1, 1));
}

#[test]
fn lerp_toward_a_darker_target_darkens() {
    // The dim effect's real direction: a bright fg blended toward a dark bg.
    assert_eq!(
        Rgb(255, 255, 255).lerp(Rgb(0, 0, 0), 0.5),
        Rgb(128, 128, 128)
    );
}
