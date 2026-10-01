//! The running editor's version, stamped at build time.
//!
//! Dev builds carry the next release's number (the version is bumped right
//! after each tag), so [`at_least`] compares numbers only and a dev build
//! counts as the version it is heading toward.

/// `CARGO_PKG_VERSION` plus the `-<sha>` suffix a non-release build carries.
pub const DISPLAY: &str = concat!(env!("CARGO_PKG_VERSION"), env!("HUME_VERSION_SUFFIX"));

pub const MAJOR: u32 = component(env!("CARGO_PKG_VERSION_MAJOR"));
pub const MINOR: u32 = component(env!("CARGO_PKG_VERSION_MINOR"));
pub const PATCH: u32 = component(env!("CARGO_PKG_VERSION_PATCH"));

/// Short git sha of the build, `"unknown"` when built without git, and `None`
/// on a build from an exact release tag.
pub const COMMIT: Option<&str> = if env!("HUME_BUILD_RELEASE").is_empty() {
    Some(env!("HUME_BUILD_COMMIT"))
} else {
    None
};

/// Whether the running version is at least `major.minor.patch`.
pub fn at_least(major: u32, minor: u32, patch: u32) -> bool {
    (MAJOR, MINOR, PATCH) >= (major, minor, patch)
}

const fn component(digits: &str) -> u32 {
    match u32::from_str_radix(digits, 10) {
        Ok(n) => n,
        Err(_) => panic!("CARGO_PKG_VERSION component is not a number"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_compares_components_in_order() {
        assert!(at_least(0, 0, 0));
        assert!(at_least(MAJOR, MINOR, PATCH));
        assert!(!at_least(MAJOR, MINOR, PATCH + 1));
        assert!(!at_least(MAJOR, MINOR + 1, 0));
        assert!(!at_least(MAJOR + 1, 0, 0));
        if let Some(lower_minor) = MINOR.checked_sub(1) {
            assert!(at_least(MAJOR, lower_minor, u32::MAX));
        }
        if let Some(lower_major) = MAJOR.checked_sub(1) {
            assert!(at_least(lower_major, u32::MAX, u32::MAX));
        }
    }
}
