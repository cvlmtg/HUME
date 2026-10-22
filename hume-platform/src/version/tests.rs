use super::*;

#[test]
fn at_least_compares_components_in_order() {
    assert!(at_least(0, 0, 0));
    assert!(at_least(MAJOR, MINOR, PATCH));
    assert!(!at_least(MAJOR, MINOR, PATCH + 1));
    assert!(!at_least(MAJOR, MINOR + 1, 0));
    assert!(!at_least(MAJOR + 1, 0, 0));
    if let Some(lower_minor) = MINOR.checked_sub(1) {
        assert!(at_least(MAJOR, lower_minor, u32::MAX));
    }
    if let Some(lower_major) = MAJOR.checked_sub(1) {
        assert!(at_least(lower_major, u32::MAX, u32::MAX));
    }
}
