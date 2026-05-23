use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

use engine::pipeline::{BufferId, PaneId};

use crate::editor::keymap::Keymap;
use crate::settings::EditorSettings;

use super::attribution::PluginStack;
use super::context::SteelCtx;
use super::hooks::HookRegistry;
use super::lazy::LazyRegistry;
use super::types::{EditorSteelRefs, PendingLanguageReg};
use super::HostBundle;

/// Backing storage for [`SteelCtx`] in unit tests.
///
/// Because `SteelCtx<'a>` borrows all persistent state by reference, tests
/// need owned storage to borrow from.  Create one of these, then call
/// [`SteelCtxTestHarness::ctx`] to get a `SteelCtx<'_>` that borrows from it.
#[cfg(test)]
pub(crate) struct SteelCtxTestHarness {
    pub(crate) settings: EditorSettings,
    pub(crate) keymap: Keymap,
    pub(crate) plugin_stack: PluginStack,
    pub(crate) cmd_owners: std::collections::HashMap<String, String>,
    pub(crate) hooks: HookRegistry,
    pub(crate) lazy_registry: LazyRegistry,
    pub(crate) declared_plugins: Vec<String>,
    pub(crate) pending_messages: Vec<(crate::editor::Severity, String)>,
    pub(crate) pending_language_regs: Vec<PendingLanguageReg>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) runtime_dir: Option<PathBuf>,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
}

#[cfg(test)]
impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            settings: EditorSettings::default(),
            keymap: Keymap::default(),
            plugin_stack: PluginStack::default(),
            cmd_owners: std::collections::HashMap::new(),
            hooks: HookRegistry::default(),
            lazy_registry: LazyRegistry::default(),
            declared_plugins: Vec::new(),
            pending_messages: Vec::new(),
            pending_language_regs: Vec::new(),
            data_dir: None,
            runtime_dir: None,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a `SteelCtx` in command mode (`is_init = false`) borrowing from
    /// this harness.  Inspect harness fields after the call to read side-effects.
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        let Self {
            settings,
            keymap,
            plugin_stack,
            cmd_owners,
            hooks,
            lazy_registry,
            declared_plugins,
            pending_messages,
            pending_language_regs,
            data_dir,
            runtime_dir,
            interrupt_flag,
        } = self;
        SteelCtx::new_command(
            HostBundle {
                plugin_stack,
                cmd_owners,
                hooks,
                lazy_registry,
                declared_plugins,
                pending_messages,
                pending_language_regs,
                data_dir: data_dir.as_deref(),
                runtime_dir: runtime_dir.as_deref(),
                interrupt_flag: Arc::clone(interrupt_flag),
            },
            EditorSteelRefs {
                settings,
                keymap,
                focused_pane_id: PaneId::default(),
                focused_buffer_id: BufferId::default(),
                buffers: None,
                engine_view: None,
                pane_state: None,
                pane_jumps: None,
            },
            None,
        )
    }
}
