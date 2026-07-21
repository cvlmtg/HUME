//! Platform/architecture identification for the LSP server installer.

/// The install-target identifier for the current platform, matching the
/// `hume-target` vocabulary used in `runtime/scheme/lsp-sources.scm`
/// (`darwin-arm64` | `darwin-x64` | `linux-x64` | `windows-x64`).
///
/// Returns `None` on any other platform/architecture combination — callers
/// treat that as "no seeded install source can match here", not an error.
pub const fn hume_target() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("darwin-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("darwin-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
