use super::*;

#[test]
fn run_inline_output_missing_binary_is_io_error() {
    assert!(run_inline_output("definitely-not-a-real-binary-xyz", &[], None, &[]).is_err());
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
