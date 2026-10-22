//! Plugin activation.
//!
//! ## Activation state machine
//!
//! ```text
//! Declared ──── %activate-plugin-inline! ────► Loading ──┬──► Loaded
//!                                                        └──► Failed
//!
//! Loaded / Failed / Loading / absent  ──► no-op (#f guard in %begin-lazy-activation!)
//! ```
//!
//! All plugin activation is synchronous/inline:
//! - `load-plugin!` (init.scm): the BOOTSTRAP Scheme wrapper calls `%load-plugin!`,
//!   which either returns a manifest to evaluate (lazy) or asks for
//!   `%activate-plugin-inline!` (inline body eval via `hm.eval-string`).
//! - Lazy keypress dispatch: `%dispatch-command!` activates the owner inline when
//!   no mappable proc is defined yet, then retries.
//! - Event/language activations: `activate_plugin_inline` (Rust) bounces into
//!   `(%activate-plugin-inline! id)` by a direct function call (not source),
//!   using the `ScriptingHost`'s one persistent watchdog.

#[cfg(test)]
mod tests;
