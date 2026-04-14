#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub(crate) mod dirs;

use std::io;

/// Probe the terminal for kitty keyboard protocol support.
///
/// Dispatches to the platform-specific implementation in `unix` or `windows`.
/// Each implementation sends `\x1B[?u` (kitty flags query), `\x1B[>q`
/// (XTVERSION), and `\x1B[c` (DA1 sentinel) to the terminal and inspects the
/// raw response bytes. Returns `Ok(true)` if the terminal supports kitty
/// keyboard protocol push, `Ok(false)` otherwise.
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
/// to the `\x1B[?u` query. DA1 sequences (`ESC [ ? <digits> c`) are skipped
/// over — they don't indicate kitty support but don't rule it out either, since
/// both responses may appear in the same buffer.
fn has_kitty_response(buf: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0x1B && buf[i + 1] == b'[' && buf[i + 2] == b'?' {
            let mut j = i + 3;
            while j < buf.len() {
                match buf[j] {
                    b'u' => return true,          // kitty flags response
                    b'c' => break,                // DA1 — skip and keep scanning
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
fn has_kitty_xtversion(buf: &[u8]) -> bool {
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
    // kitty, ghostty, and foot also respond to the query, so they're redundant
    // here but harmless as a fallback.
    name.starts_with(b"WezTerm")
        || name.starts_with(b"kitty")
        || name.starts_with(b"ghostty")
        || name.starts_with(b"foot")
}

/// Returns true once the buffer contains a complete DA1 response (`ESC [ ? <digits> c`),
/// which signals the terminal has finished responding to all queries.
fn has_da1_response(buf: &[u8]) -> bool {
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
    use super::{has_da1_response, has_kitty_response, has_kitty_xtversion};

    // ── has_kitty_response ────────────────────────────────────────────────────

    #[test]
    fn kitty_response_before_da1() {
        assert!(has_kitty_response(b"\x1B[?0u\x1B[?62;22c"));
    }

    #[test]
    fn kitty_response_after_da1() {
        // Race condition case: DA1 arrives first, kitty response second.
        assert!(has_kitty_response(b"\x1B[?62;22c\x1B[?0u"));
    }

    #[test]
    fn kitty_response_with_multi_semicolon_flags() {
        assert!(has_kitty_response(b"\x1B[?1;2;3u"));
    }

    #[test]
    fn kitty_response_preceded_by_noise() {
        assert!(has_kitty_response(b"noise\x1B[?0u"));
    }

    #[test]
    fn no_kitty_response_from_da1_only() {
        assert!(!has_kitty_response(b"\x1B[?1;0c"));
    }

    #[test]
    fn no_kitty_response_from_empty() {
        assert!(!has_kitty_response(b""));
    }

    #[test]
    fn no_kitty_response_from_three_byte_boundary() {
        assert!(!has_kitty_response(b"\x1B[?"));
    }

    // ── has_da1_response ──────────────────────────────────────────────────────

    #[test]
    fn da1_detected() {
        assert!(has_da1_response(b"\x1B[?1;0c"));
    }

    #[test]
    fn da1_detected_after_kitty_response() {
        assert!(has_da1_response(b"\x1B[?0u\x1B[?62;22c"));
    }

    #[test]
    fn no_da1_from_empty() {
        assert!(!has_da1_response(b""));
    }

    #[test]
    fn incomplete_da1_does_not_match() {
        assert!(!has_da1_response(b"\x1B[?62;2"));
    }

    #[test]
    fn no_da1_from_three_byte_boundary() {
        assert!(!has_da1_response(b"\x1B[?"));
    }

    // ── has_kitty_xtversion ───────────────────────────────────────────────────

    #[test]
    fn xtversion_wezterm() {
        let buf = b"\x1BP>|WezTerm 20240203-110809-5046fc22\x1B\\\x1B[?65;4;6;18;22c";
        assert!(!has_kitty_response(buf));
        assert!(has_kitty_xtversion(buf));
    }

    #[test]
    fn xtversion_kitty() {
        assert!(has_kitty_xtversion(b"\x1BP>|kitty(0.35.2)\x1B\\\x1B[?1c"));
    }

    #[test]
    fn xtversion_ghostty() {
        assert!(has_kitty_xtversion(b"\x1BP>|ghostty 1.0.0\x1B\\\x1B[?1c"));
    }

    #[test]
    fn xtversion_foot() {
        assert!(has_kitty_xtversion(b"\x1BP>|foot(1.17.0)\x1B\\\x1B[?1c"));
    }

    #[test]
    fn xtversion_iterm2_not_matched() {
        assert!(!has_kitty_xtversion(b"\x1BP>|iTerm2 3.5\x1B\\\x1B[?1;2c"));
    }

    #[test]
    fn xtversion_no_response() {
        assert!(!has_kitty_xtversion(b"\x1B[?1;2c"));
    }

    #[test]
    fn xtversion_incomplete_no_st() {
        assert!(!has_kitty_xtversion(b"\x1BP>|WezTerm 20240203"));
    }

    #[test]
    fn xtversion_empty_name() {
        // Empty name should not match any known terminal.
        assert!(!has_kitty_xtversion(b"\x1BP>|\x1B\\\x1B[?1c"));
    }
}
