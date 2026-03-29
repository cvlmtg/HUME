#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::io;

/// Probe the terminal for kitty keyboard protocol support.
///
/// Sends `\x1B[?u` (kitty flags query), `\x1B[>q` (XTVERSION), and `\x1B[c`
/// (DA1 sentinel) in one write. Reads back bytes until the DA1 response
/// arrives (indicating the terminal has finished replying), then checks for
/// either a kitty flags reply or a recognised terminal name via XTVERSION.
///
/// Must be called after `enable_raw_mode()`.
pub(crate) fn probe_kitty_support() -> io::Result<bool> {
    #[cfg(unix)]
    {
        unix::probe_kitty_support()
    }
    #[cfg(windows)]
    {
        windows::probe_kitty_support()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(false)
    }
}

/// Scan raw terminal response bytes for a kitty keyboard protocol reply.
///
/// Looks for the pattern `ESC [ ? <digits> u` which is the terminal's response
/// to the `\x1B[?u` query. The DA1 response (`ESC [ ? <digits> c`) is treated
/// as a sentinel that the terminal doesn't support kitty.
pub(super) fn has_kitty_response(buf: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0x1B && buf[i + 1] == b'[' && buf[i + 2] == b'?' {
            let mut j = i + 3;
            while j < buf.len() {
                match buf[j] {
                    b'u' => return true,          // kitty flags response
                    b'c' => break,                // DA1 — not kitty, keep scanning
                    b'0'..=b'9' | b';' => j += 1,
                    _ => break,                   // unexpected byte, abandon sequence
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}

/// Check XTVERSION response (`ESC P > | <name> ESC \`) against terminals known
/// to support kitty keyboard protocol push but that may not respond to the
/// `\x1B[?u` query (e.g. older WezTerm releases).
///
/// XTVERSION is sent alongside the kitty query and DA1 sentinel as a fallback
/// identification mechanism. Its response arrives before DA1.
pub(super) fn is_kitty_from_xtversion(buf: &[u8]) -> bool {
    // Find the DCS introducer for XTVERSION: ESC P > |
    let Some(pos) = buf.windows(4).position(|w| w == b"\x1BP>|") else {
        return false;
    };
    let name_start = pos + 4;
    // Find the String Terminator: ESC \
    let Some(st_pos) = buf[name_start..].windows(2).position(|w| w == b"\x1B\\") else {
        return false;
    };
    let name = &buf[name_start..name_start + st_pos];
    // Terminals confirmed to support kitty push regardless of query support.
    // kitty and ghostty also respond to the query, so they're redundant here
    // but harmless as a fallback.
    name.starts_with(b"WezTerm")
        || name.starts_with(b"kitty")
        || name.starts_with(b"ghostty")
        || name.starts_with(b"foot")
}

/// Returns true once the buffer contains a complete DA1 response (`ESC [ ? <digits> c`),
/// which signals the terminal has finished responding to both queries.
pub(super) fn has_da1_response(buf: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0x1B && buf[i + 1] == b'[' && buf[i + 2] == b'?' {
            let mut j = i + 3;
            while j < buf.len() {
                match buf[j] {
                    b'c' => return true,
                    b'0'..=b'9' | b';' => j += 1,
                    _ => break,
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{has_da1_response, has_kitty_response, is_kitty_from_xtversion};

    // Typical kitty terminal: sends kitty flags first, then DA1.
    #[test]
    fn detects_kitty_response_before_da1() {
        assert!(has_kitty_response(b"\x1B[?0u\x1B[?62;22c"));
    }

    // Non-kitty terminal: sends only DA1.
    #[test]
    fn no_false_positive_from_da1_only() {
        let buf = b"\x1B[?1;0c";
        assert!(!has_kitty_response(buf));
        assert!(has_da1_response(buf));
    }

    // DA1 arrives before kitty response (race condition: should still detect).
    #[test]
    fn detects_kitty_response_after_da1() {
        assert!(has_kitty_response(b"\x1B[?62;22c\x1B[?0u"));
    }

    // No response at all (timeout path).
    #[test]
    fn empty_buffer_is_not_kitty() {
        assert!(!has_kitty_response(b""));
        assert!(!has_da1_response(b""));
    }

    // DA1 not yet complete — read loop should keep going.
    #[test]
    fn incomplete_da1_does_not_trigger_stop() {
        assert!(!has_da1_response(b"\x1B[?62;2"));
    }

    // WezTerm: no kitty query response, but XTVERSION identifies it.
    #[test]
    fn wezterm_detected_via_xtversion() {
        // Simulates WezTerm 20240203: only XTVERSION + DA1, no kitty query response.
        let buf = b"\x1BP>|WezTerm 20240203-110809-5046fc22\x1B\\\x1B[?65;4;6;18;22c";
        assert!(!has_kitty_response(buf));
        assert!(is_kitty_from_xtversion(buf));
    }

    // Non-kitty terminal (e.g. Terminal.app): XTVERSION might not be present,
    // and if it is, should not match.
    #[test]
    fn unknown_terminal_not_detected_via_xtversion() {
        assert!(!is_kitty_from_xtversion(b"\x1B[?1;2c")); // no XTVERSION
        assert!(!is_kitty_from_xtversion(b"\x1BP>|iTerm2 3.5\x1B\\\x1B[?1;2c"));
    }

    // Incomplete XTVERSION (missing ST) should not match.
    #[test]
    fn incomplete_xtversion_does_not_match() {
        assert!(!is_kitty_from_xtversion(b"\x1BP>|WezTerm 20240203")); // no ESC \
    }
}
