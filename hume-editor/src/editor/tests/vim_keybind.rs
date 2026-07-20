use super::*;
use pretty_assertions::assert_eq;

// `core:vim-keybind` end-to-end plugin tests live in `unix/vim_keybind.rs`
// (Scheme require strings embed OS paths); this half covers the default
// keymap, which is platform-neutral.

// ── Defaults do not bind these keys without the plugin ─────────────────────

#[test]
fn without_plugin_dollar_caret_zero_ctrl6_are_noops() {
    let mut ed = editor_from("-[h]>ello world\n");
    let before = state(&ed);
    ed.handle_key(key('$'));
    assert_eq!(
        state(&ed),
        before,
        "$ must be inert without core:vim-keybind"
    );
    ed.handle_key(key('^'));
    assert_eq!(
        state(&ed),
        before,
        "^ must be inert without core:vim-keybind"
    );
    ed.handle_key(key('0'));
    assert_eq!(
        state(&ed),
        before,
        "0 must be inert without core:vim-keybind"
    );
    ed.handle_key(key_ctrl('6'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+6 must be inert without core:vim-keybind"
    );
}
