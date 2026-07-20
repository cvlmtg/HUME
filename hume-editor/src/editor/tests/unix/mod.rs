//! Tests that cannot run on Windows, gated once at the `mod unix;`
//! declaration in the parent — files in here need no `#[cfg]` attributes.
//!
//! Most tests here load Steel plugins from disk: Scheme `require` strings
//! embed OS paths, and backslashes are not escaped in Steel string literals.
//! The rest exercise unix-only behavior directly (e.g. `HUME_RUNTIME`
//! resolution, `set_cwd` against canonicalized paths).
//!
//! A test file with both portable and unix-only tests is split into a
//! same-named file here holding the unix-only half.

use super::*;

mod cd;
mod lsp_actions;
mod lsp_completion_feature;
mod lsp_diagnostics_inline;
mod lsp_diagnostics_nav;
mod lsp_format;
mod lsp_goto;
mod lsp_hover;
mod lsp_inlay_feature;
mod lsp_packaging;
mod lsp_references;
mod lsp_rename;
mod lsp_sighelp;
mod plugins;
mod scripting_effects;
mod scripting_grammar;
mod scripting_lsp_install;
mod tutor;
mod vim_keybind;
