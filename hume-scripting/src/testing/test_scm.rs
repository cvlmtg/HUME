//! Init-source builders shared by tests that need a plugin's body evaluated
//! during init.

/// Init source that loads `plugin` and then activates its main entry inline.
/// A plugin with a `manifest.scm` stays lazy under `load-plugin!` alone, so a
/// test that needs the body to have run calls this instead. `config` is the
/// Steel source of a `#:config` value.
pub fn eager_load_scm(plugin: &str, config: Option<&str>) -> String {
    let config = config.map(|c| format!(" #:config {c}")).unwrap_or_default();
    format!("(load-plugin! \"{plugin}\"{config})\n(%activate-plugin-inline! \"{plugin}\" #f)")
}
