//! Every test runs `sh`, so all of them are unix-only.

#[cfg(unix)]
mod unix;
