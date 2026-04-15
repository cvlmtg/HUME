//! Plugin lifecycle builtins: `push-declared-plugin!`, `push-loaded-plugin!`,
//! `push-current-plugin!`, `pop-current-plugin!`, `resolve-plugin-path`,
//! `declared-plugins`, `loaded-plugins`.
//!
//! These are called from the Scheme-side `load-plugin` wrapper defined in the
//! bootstrap; see `mod.rs` for the Scheme source.

use steel::rvals::{IntoSteelVal, SteelVal};
use steel::rerrs::{ErrorKind, SteelErr};

use crate::scripting::ledger::PluginId;
use super::one_string;

// ── Helpers ───────────────────────────────────────────────────────────────────

type SteelResult = Result<SteelVal, SteelErr>;

// ── Plugin name validation ────────────────────────────────────────────────────

enum ParsedName {
    Core(String),
    User { user: String, repo: String },
}

/// A valid path segment must be non-empty, not `.` or `..`, and contain no
/// `/`, `\`, or NUL characters.  Dots elsewhere are permitted (e.g. `v1.2`).
fn is_valid_segment(s: &str) -> bool {
    // Reject `.` and `..` explicitly before the char scan — dots are otherwise
    // allowed, so without this check `..` would pass the char loop.
    if s.is_empty() || s == "." || s == ".." {
        return false;
    }
    s.chars().all(|c| c != '/' && c != '\\' && c != '\0')
}

/// Parse and validate a plugin name.
///
/// Valid forms:
/// - `core:<name>` — core plugin in the runtime directory
/// - `<user>/<repo>` — third-party plugin in the data directory
///
/// Returns a Steel error for any other form.
fn parse_plugin_name(name: &str) -> Result<ParsedName, SteelErr> {
    if let Some(core_name) = name.strip_prefix("core:") {
        // Case-insensitive prefix match handled by strip_prefix("core:") since
        // the caller normalises. Actually, the STEEL.md requires case-preserving
        // storage but case-insensitive identity — we validate the segment only.
        if !is_valid_segment(core_name) {
            return Err(SteelErr::new(ErrorKind::Generic,
                format!("invalid plugin name '{name}': core name must be a non-empty path segment")));
        }
        return Ok(ParsedName::Core(core_name.to_string()));
    }
    // Try "user/repo" form — exactly one slash separating two valid segments.
    if let Some((user, repo)) = name.split_once('/') {
        // repo must not contain further slashes
        if repo.contains('/') {
            return Err(SteelErr::new(ErrorKind::Generic,
                format!("invalid plugin name '{name}': expected user/repo with exactly one slash")));
        }
        if !is_valid_segment(user) || !is_valid_segment(repo) {
            return Err(SteelErr::new(ErrorKind::Generic,
                format!("invalid plugin name '{name}': user and repo must be non-empty valid path segments")));
        }
        return Ok(ParsedName::User { user: user.to_string(), repo: repo.to_string() });
    }
    Err(SteelErr::new(ErrorKind::Generic,
        format!("invalid plugin name '{name}': expected 'core:<name>' or '<user>/<repo>'")))
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(push-declared-plugin! name)` — validate `name` and append to the
/// declared-plugins list (case-insensitive dedup).
///
/// Raises a Steel error for malformed names, aborting `init.scm`.
pub(crate) fn push_declared_plugin(args: &[SteelVal]) -> SteelResult {
    let name = one_string(args, "push-declared-plugin!")?;
    // Validate before recording so declared-plugins never contains junk.
    parse_plugin_name(&name)?;
    super::with_ctx(|ctx| {
        if !ctx.declared_plugins.iter().any(|d| d.eq_ignore_ascii_case(&name)) {
            ctx.declared_plugins.push(name);
        }
        Ok(SteelVal::Void)
    })
}

/// `(push-loaded-plugin! name)` — append to the loaded-plugins list
/// (case-insensitive dedup, no validation — caller already validated).
pub(crate) fn push_loaded_plugin(args: &[SteelVal]) -> SteelResult {
    let name = one_string(args, "push-loaded-plugin!")?;
    super::with_ctx(|ctx| {
        if !ctx.loaded_plugins.iter().any(|l| l.eq_ignore_ascii_case(&name)) {
            ctx.loaded_plugins.push(name);
        }
        Ok(SteelVal::Void)
    })
}

/// `(push-current-plugin! name)` — push `name` onto the `CURRENT_PLUGIN`
/// attribution stack.  Called from `dynamic-wind`'s before-thunk inside
/// the Scheme-side `load-plugin`.
pub(crate) fn push_current_plugin(args: &[SteelVal]) -> SteelResult {
    let name = one_string(args, "push-current-plugin!")?;
    super::with_ctx(|ctx| {
        ctx.plugin_stack.push(PluginId::new(name));
        Ok(SteelVal::Void)
    })
}

/// `(pop-current-plugin!)` — pop the top entry from the `CURRENT_PLUGIN`
/// stack.  Called from `dynamic-wind`'s after-thunk.  Raises a Steel error
/// on empty stack (the before/after pairing should always be balanced).
pub(crate) fn pop_current_plugin(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "pop-current-plugin! expects 0 args, got {}", args.len());
    }
    super::with_ctx(|ctx| {
        if ctx.plugin_stack.is_empty() {
            steel::stop!(Generic => "pop-current-plugin!: attribution stack is already empty");
        }
        ctx.plugin_stack.pop();
        Ok(SteelVal::Void)
    })
}

/// Pure path resolution: given a plugin name and the runtime / data directories,
/// return the resolved `PathBuf` if the plugin file exists on disk, or `None`.
///
/// Called by both the `resolve-plugin-path` Steel builtin (which uses `with_ctx`
/// to supply the dirs) and by [`crate::scripting::ScriptingHost::reload_plugin`]
/// (which accesses the dirs directly on `ScriptFacingCtx`).
pub(crate) fn resolve_path_for_name(
    name: &str,
    runtime_dir: Option<&std::path::Path>,
    data_dir: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    let parsed = parse_plugin_name(name)
        .map_err(|e| e.to_string())?;
    let path = match parsed {
        ParsedName::Core(core_name) => {
            runtime_dir.map(|rt| {
                rt.join("plugins").join("core").join(&core_name).join("plugin.scm")
            })
        }
        ParsedName::User { user, repo } => Some(
            data_dir.join("plugins").join(&user).join(&repo).join("plugin.scm"),
        ),
    };
    Ok(path.filter(|p| p.exists()))
}

/// `(resolve-plugin-path name)` — return the resolved path string if the
/// plugin file exists on disk, or `#f` if absent.  Raises a Steel error for
/// malformed names.
pub(crate) fn resolve_plugin_path(args: &[SteelVal]) -> SteelResult {
    let name = one_string(args, "resolve-plugin-path")?;
    super::with_ctx(|ctx| {
        let path = resolve_path_for_name(&name, ctx.runtime_dir.as_deref(), &ctx.data_dir)
            .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
        match path {
            Some(p) => Ok(SteelVal::StringV(p.to_string_lossy().into_owned().into())),
            None    => Ok(SteelVal::BoolV(false)),
        }
    })
}

/// `(loaded-plugins)` — return a Steel list of all loaded plugin names.
pub(crate) fn loaded_plugins(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "loaded-plugins expects 0 args, got {}", args.len());
    }
    super::with_ctx(|ctx| {
        let vals: Vec<SteelVal> = ctx
            .loaded_plugins
            .iter()
            .map(|s| SteelVal::StringV(s.as_str().into()))
            .collect();
        vals.into_steelval()
    })
}

/// `(declared-plugins)` — return a Steel list of all declared third-party
/// (non-`core:*`) plugin names.  Used by PLUM to know what to install.
pub(crate) fn declared_plugins(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "declared-plugins expects 0 args, got {}", args.len());
    }
    super::with_ctx(|ctx| {
        let vals: Vec<SteelVal> = ctx
            .declared_plugins
            .iter()
            .filter(|name| !name.to_ascii_lowercase().starts_with("core:"))
            .map(|s| SteelVal::StringV(s.as_str().into()))
            .collect();
        vals.into_steelval()
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_core_segment() {
        assert!(is_valid_segment("helix-surround"));
        assert!(is_valid_segment("plum"));
        assert!(is_valid_segment("v1.2.3"));
    }

    #[test]
    fn invalid_segments() {
        assert!(!is_valid_segment(""));
        assert!(!is_valid_segment("."));
        assert!(!is_valid_segment(".."));
        assert!(!is_valid_segment("a/b"));
        assert!(!is_valid_segment("a\\b"));
        assert!(!is_valid_segment("a\0b"));
    }

    #[test]
    fn parse_core_plugin_name() {
        let p = parse_plugin_name("core:helix-surround").unwrap();
        assert!(matches!(p, ParsedName::Core(n) if n == "helix-surround"));
    }

    #[test]
    fn parse_user_plugin_name() {
        let p = parse_plugin_name("user/repo").unwrap();
        assert!(matches!(p, ParsedName::User { user, repo } if user == "user" && repo == "repo"));
    }

    #[test]
    fn parse_invalid_names() {
        assert!(parse_plugin_name("bad").is_err());
        assert!(parse_plugin_name("core:").is_err());
        assert!(parse_plugin_name("a/b/c").is_err());
        assert!(parse_plugin_name("/repo").is_err());
        assert!(parse_plugin_name("user/").is_err());
        assert!(parse_plugin_name("core:..").is_err());
        assert!(parse_plugin_name("../evil").is_err());
    }
}
