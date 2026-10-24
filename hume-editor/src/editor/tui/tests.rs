use super::*;

#[test]
fn off_has_no_active_handle() {
    assert!(Tui::Off.terminal().is_none());
    assert!(Tui::Off.as_active().is_none());
}

#[test]
fn headless_converts_to_an_active_headless_handle() {
    let active = Tui::OnHeadless.as_active().expect("OnHeadless is active");
    assert!(matches!(active, ActiveTui::Headless));
}
