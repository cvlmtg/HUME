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

/// Convert a `PluginId::parse` error string into a Steel `Generic` error.
fn steel_parse_err(e: String) -> SteelErr {
    SteelErr::new(ErrorKind::Generic, e)
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(push-declared-plugin! name)` — validate `name` and append to the
/// declared-plugins list (case-insensitive dedup).
///
/// Raises a Steel error for malformed names, aborting `init.scm`.
pub(crate) fn push_declared_plugin(args: &[SteelVal]) -> SteelResult {
    let name = one_string(args, "push-declared-plugin!")?;
    // Validate before recording so declared-plugins never contains junk.
    PluginId::parse(&name).map_err(steel_parse_err)?;
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
    let plugin_id = PluginId::parse(&name).map_err(steel_parse_err)?;
    super::with_ctx(|ctx| {
        ctx.plugin_stack.push(plugin_id);
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
    data_dir: Option<&std::path::Path>,
) -> Result<Option<std::path::PathBuf>, String> {
    let plugin_id = PluginId::parse(name)?;
    let path = match plugin_id {
        PluginId::Core(core_name) => {
            runtime_dir.map(|rt| {
                rt.join("plugins").join("core").join(&core_name).join("plugin.scm")
            })
        }
        // When data_dir is None (HOME/APPDATA unset), user plugins cannot be
        // resolved — return None rather than panicking.
        PluginId::User { user, repo } => data_dir.map(|d| {
            d.join("plugins").join(&user).join(&repo).join("plugin.scm")
        }),
    };
    // Probe existence without a pre-flight `.exists()` to avoid TOCTOU.
    // NotFound → plugin absent (Ok(None)); other errors propagate.
    match path {
        None => Ok(None),
        Some(p) => match std::fs::metadata(&p) {
            Ok(_) => Ok(Some(p)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot stat plugin path '{}': {e}", p.display())),
        },
    }
}

/// `(resolve-plugin-path name)` — return the resolved path string if the
/// plugin file exists on disk, or `#f` if absent.  Raises a Steel error for
/// malformed names.
pub(crate) fn resolve_plugin_path(args: &[SteelVal]) -> SteelResult {
    let name = one_string(args, "resolve-plugin-path")?;
    super::with_ctx(|ctx| {
        let path = resolve_path_for_name(&name, ctx.runtime_dir.as_deref(), ctx.data_dir.as_deref())
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
//
// Parsing tests (valid/invalid plugin names, segments) live in
// `scripting::ledger::tests` alongside `PluginId::parse`.  The tests here
// cover only the builtins' Steel-facing behaviour.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_core_plugin_name() {
        let id = PluginId::parse("core:helix-surround").unwrap();
        assert!(matches!(id, PluginId::Core(n) if n == "helix-surround"));
    }

    #[test]
    fn parse_user_plugin_name() {
        let id = PluginId::parse("user/repo").unwrap();
        assert!(matches!(id, PluginId::User { ref user, ref repo } if user == "user" && repo == "repo"));
    }

    #[test]
    fn parse_invalid_names() {
        assert!(PluginId::parse("bad").is_err());
        assert!(PluginId::parse("core:").is_err());
        assert!(PluginId::parse("a/b/c").is_err());
        assert!(PluginId::parse("/repo").is_err());
        assert!(PluginId::parse("user/").is_err());
        assert!(PluginId::parse("core:..").is_err());
        assert!(PluginId::parse("../evil").is_err());
    }

    #[test]
    fn valid_core_segment() {
        // Segments that should pass through PluginId::parse successfully.
        assert!(PluginId::parse("core:helix-surround").is_ok());
        assert!(PluginId::parse("core:plum").is_ok());
        assert!(PluginId::parse("core:v1.2.3").is_ok());
    }

    #[test]
    fn invalid_segments() {
        // Segment validation exercised via PluginId::parse.
        assert!(PluginId::parse("core:").is_err());       // empty
        assert!(PluginId::parse("core:.").is_err());      // dot
        assert!(PluginId::parse("core:..").is_err());     // dotdot
        assert!(PluginId::parse("./b").is_err());         // slash without user
        assert!(PluginId::parse("a\0b/repo").is_err());   // NUL in user
    }
}
