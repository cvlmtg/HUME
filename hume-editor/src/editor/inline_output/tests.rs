use super::*;

#[test]
fn is_open_tracks_frame_count_through_nesting() {
    let mut io = InlineOutput::default();
    assert!(!io.is_open());
    io.push("outer", Some(ActiveTui::Headless), false);
    assert!(io.is_open());
    let depth_inner = io.push("inner", Some(ActiveTui::Headless), false);
    assert!(io.is_open());
    io.truncate(depth_inner);
    assert!(io.is_open(), "outer frame must still be on the stack");
    io.truncate(0);
    assert!(!io.is_open());
}

/// The scenario `InlineOutput::truncate`'s own doc names: a `call!`
/// nested two levels deep raises and the raise is caught above it,
/// leaving its frame unpaired. The outer `call!`'s own restore must
/// still remove both the leaked frame and its own, not just the top
/// one a blind pop would take.
///
/// A single `self.frames.pop()` that ignores `depth` would remove only the
/// leak and leave `frame_count()` at `2`.
#[test]
fn truncate_drops_a_leaked_descendant_frame_above_its_own() {
    let mut io = InlineOutput::default();
    io.push("outer", Some(ActiveTui::Headless), false);
    let depth_middle = io.push("middle", Some(ActiveTui::Headless), false);
    io.push("inner-leak", Some(ActiveTui::Headless), false); // raised and was caught; never truncated
    io.truncate(depth_middle);
    assert_eq!(
        io.frame_count(),
        1,
        "middle's own frame and the leaked descendant above it must both \
         be gone, leaving only outer"
    );
}

#[test]
fn needs_enter_fires_once_per_session_regardless_of_nesting() {
    let mut io = InlineOutput::default();
    io.push("outer", Some(ActiveTui::Headless), false);
    assert_eq!(io.needs_enter().map(|(name, _, _)| name), Some("outer"));
    io.mark_entered(false, false, false, ActiveTui::Headless);
    assert!(io.needs_enter().is_none());
    // A nested call! after the outer already entered must not re-enter.
    io.push("inner", Some(ActiveTui::Headless), false);
    assert!(io.needs_enter().is_none(), "already entered this session");
}

#[test]
fn needs_enter_skips_a_frame_with_no_active_tui() {
    let mut io = InlineOutput::default();
    io.push("cmd", None, false);
    assert!(io.is_open(), "gate still opens off the event loop");
    assert!(io.needs_enter().is_none());
}

/// `needs_enter` must return the pushing frame's own captured `kitty`,
/// not some ambient default: the whole reason `Frame` captures it
/// rather than a caller re-reading `Editor::kitty_enabled` later (a
/// different, or differently-configured, host may disagree).
#[test]
fn needs_enter_returns_the_frame_s_own_captured_kitty() {
    let mut io = InlineOutput::default();
    io.push("cmd", Some(ActiveTui::Headless), true);
    assert_eq!(io.needs_enter().map(|(_, _, kitty)| kitty), Some(true));
}

#[test]
fn truncate_to_zero_preserves_entered_and_ran() {
    let mut io = InlineOutput::default();
    io.push("cmd", Some(ActiveTui::Headless), false);
    io.mark_entered(true, true, false, ActiveTui::Headless);
    io.truncate(0);
    assert!(!io.is_open());
    assert!(io.take_ran(), "ran must survive a full truncate");
    assert!(
        io.take_entered().is_some(),
        "entered must survive a full truncate"
    );
}

#[test]
fn take_entered_and_take_ran_clear_after_reading() {
    let mut io = InlineOutput::default();
    io.push("cmd", Some(ActiveTui::Headless), false);
    io.mark_entered(false, false, false, ActiveTui::Headless);
    assert!(io.take_entered().is_some());
    assert!(io.take_ran());
    assert!(io.take_entered().is_none(), "must not report entered twice");
    assert!(!io.take_ran(), "must not report ran twice");
}
