use super::{Backend, SystemClipboard};

#[test]
fn remote_session_selects_osc52() {
    assert!(matches!(
        SystemClipboard::select(true).backend,
        Backend::Osc52
    ));
}
