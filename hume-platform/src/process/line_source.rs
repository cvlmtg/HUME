//! Streaming a child process's stdout into complete lines.
//!
//! [`LineSplitter`] is the pure, allocation-light half of the picker's
//! external-command source (`docs/FUZZY-FINDERS.md` B5): a reader thread
//! feeds it arbitrary-sized chunks off a pipe, and it yields only complete
//! lines, carrying any trailing partial line across chunk boundaries. Kept
//! separate from the spawn/thread machinery so the boundary-carry logic is
//! unit-testable without ever touching a real process.

/// Splits a byte stream into complete lines on `delim`, carrying a trailing
/// partial line across `push_chunk` calls.
///
/// `\n`-delimited streams also get `\r` stripped from the line's end
/// (Windows CRLF output); NUL-delimited streams (`#:nul #t`, e.g.
/// `git ls-files -z`) do not, since NUL-separated records have no CRLF
/// convention to strip.
pub struct LineSplitter {
    delim: u8,
    strip_cr: bool,
    carry: Vec<u8>,
}

impl LineSplitter {
    pub fn new(delim: u8) -> Self {
        Self {
            delim,
            strip_cr: delim == b'\n',
            carry: Vec::new(),
        }
    }

    /// Split `chunk` into complete lines. A line that doesn't end in `delim`
    /// by the end of `chunk` is carried and prefixed onto the next call's
    /// (or [`finish`](Self::finish)'s) output — it never appears here.
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, &byte) in chunk.iter().enumerate() {
            if byte == self.delim {
                let mut line = std::mem::take(&mut self.carry);
                line.extend_from_slice(&chunk[start..i]);
                lines.push(self.finish_line(line));
                start = i + 1;
            }
        }
        self.carry.extend_from_slice(&chunk[start..]);
        lines
    }

    /// The trailing unterminated line at end-of-stream, if any bytes are
    /// still carried; `None` if the stream ended cleanly on `delim`.
    pub fn finish(&mut self) -> Option<String> {
        if self.carry.is_empty() {
            None
        } else {
            let line = std::mem::take(&mut self.carry);
            Some(self.finish_line(line))
        }
    }

    fn finish_line(&self, mut line: Vec<u8>) -> String {
        if self.strip_cr && line.last() == Some(&b'\r') {
            line.pop();
        }
        String::from_utf8_lossy(&line).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_multiple_lines_in_one_chunk() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\nb\nc\n"), vec!["a", "b", "c"]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn carries_a_partial_line_across_chunk_boundary() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"ab"), Vec::<String>::new());
        assert_eq!(s.push_chunk(b"c\nd\n"), vec!["abc", "d"]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn carries_across_a_chunk_boundary_that_lands_exactly_on_a_delimiter() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"abc\n"), vec!["abc"]);
        assert_eq!(s.push_chunk(b"def\n"), vec!["def"]);
    }

    #[test]
    fn delimiter_as_first_byte_of_a_later_chunk_still_closes_the_carry() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"abc"), Vec::<String>::new());
        assert_eq!(s.push_chunk(b"\ndef\n"), vec!["abc", "def"]);
    }

    #[test]
    fn nul_mode_preserves_interior_newlines_and_carriage_returns() {
        let mut s = LineSplitter::new(b'\0');
        assert_eq!(s.push_chunk(b"a\r\n\0b\0"), vec!["a\r\n", "b"]);
    }

    #[test]
    fn newline_mode_strips_trailing_carriage_return() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\r\nb\r\n"), vec!["a", "b"]);
    }

    #[test]
    fn finish_emits_trailing_unterminated_line() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\nb"), vec!["a"]);
        assert_eq!(s.finish(), Some("b".to_string()));
    }

    #[test]
    fn finish_after_a_cleanly_terminated_stream_is_none() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\n"), vec!["a"]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn interior_empty_lines_are_emitted() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\n\nb\n"), vec!["a", "", "b"]);
    }

    #[test]
    fn invalid_utf8_is_lossy_replaced_not_dropped() {
        let mut s = LineSplitter::new(b'\n');
        let lines = s.push_chunk(b"\xffbad\n");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{FFFD}'), "got: {:?}", lines[0]);
    }
}
