use super::*;

#[test]
fn read_capped_discards_bytes_past_the_cap_but_keeps_reading() {
    let data = b"hello world".repeat(10);
    let captured = read_capped(&data[..], 5);
    assert_eq!(captured, b"hello");
}

#[test]
fn read_bounded_returns_everything_under_the_limit() {
    let captured = read_bounded(&b"hello"[..], 5).expect("under limit");
    assert_eq!(captured, b"hello");
}

#[test]
fn read_bounded_errors_on_overflow_instead_of_truncating() {
    let data = [b'x'; 100];
    let err = read_bounded(&data[..], 10).expect_err("must fail, not truncate");
    assert!(
        err.to_string().contains("10"),
        "error should name the limit: {err}"
    );
}

/// A `Read` that fails after handing back a prefix: the shape of a
/// pipe going bad mid-stream (EIO, ENOMEM), independent of any real
/// syscall.
struct FlakyReader {
    prefix: &'static [u8],
    served: bool,
}

impl Read for FlakyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.served {
            self.served = true;
            buf[..self.prefix.len()].copy_from_slice(self.prefix);
            return Ok(self.prefix.len());
        }
        Err(io::Error::other("simulated pipe failure"))
    }
}

#[test]
fn read_bounded_propagates_a_mid_stream_read_error() {
    let reader = FlakyReader {
        prefix: b"partial",
        served: false,
    };
    let err = read_bounded(reader, 1024).expect_err("read error must propagate");
    assert!(err.to_string().contains("simulated pipe failure"));
}

#[test]
fn read_capped_swallows_a_mid_stream_read_error_and_keeps_the_prefix() {
    let reader = FlakyReader {
        prefix: b"partial",
        served: false,
    };
    // Lenient: stderr is diagnostic-only, so a truncated
    // prefix beats losing the whole capture over a transient read
    // error. `read_bounded` is the counterpart that does not accept
    // this tradeoff for stdout.
    let captured = read_capped(reader, 1024);
    assert_eq!(captured, b"partial");
}
