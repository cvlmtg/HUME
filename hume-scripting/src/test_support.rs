//! Test support types for scripting crate unit tests.
//!
//! Lives in the scripting crate because it needs access to private types
//! (`SteelCtx`, `HostBundle`).

use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

use hume_engine::pipeline::{BufferId, PaneId};

use crate::attribution::PluginStack;
use crate::context::SteelCtx;
use crate::log::LogLevel;
use crate::null_host::NullHost;
use crate::types::PendingLanguageReg;
use crate::{HostBundle, ScriptingRegistries};

/// Backing storage for [`SteelCtx`] in scripting-crate unit tests.
///
/// Uses [`NullHost`] — sufficient for tests that only need to check that
/// scripting guards (`is_init`, `require_cmd_ctx!`, etc.) fire correctly,
/// or that `cmd_queue` accumulates correctly, without real editor state.
pub(crate) struct SteelCtxTestHarness {
    pub(crate) host: NullHost,
    pub(crate) plugin_stack: PluginStack,
    pub(crate) registries: ScriptingRegistries,
    pub(crate) pending_messages: Vec<(LogLevel, String)>,
    pub(crate) pending_language_regs: Vec<PendingLanguageReg>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) runtime_dir: Option<PathBuf>,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
}

impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            host: NullHost,
            plugin_stack: PluginStack::default(),
            registries: ScriptingRegistries {
                cmd_owners: std::collections::HashMap::new(),
                hooks: Default::default(),
                lazy_registry: Default::default(),
                declared_plugins: Vec::new(),
                command_table: std::collections::HashMap::new(),
                activation_depth: 0,
            },
            pending_messages: Vec::new(),
            pending_language_regs: Vec::new(),
            data_dir: None,
            runtime_dir: None,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a `SteelCtx` in init mode (`is_init = true`).
    pub(crate) fn ctx_init(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            plugin_stack,
            registries,
            pending_messages,
            pending_language_regs,
            data_dir,
            runtime_dir,
            interrupt_flag,
        } = self;
        SteelCtx::new_init(
            host,
            HostBundle {
                registries,
                plugin_stack,
                pending_messages,
                pending_language_regs,
                data_dir: data_dir.as_deref(),
                runtime_dir: runtime_dir.as_deref(),
                interrupt_flag: Arc::clone(interrupt_flag),
            },
            Default::default(),
        )
    }

    /// Build a `SteelCtx` in command mode (`is_init = false`).
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            plugin_stack,
            registries,
            pending_messages,
            pending_language_regs,
            data_dir,
            runtime_dir,
            interrupt_flag,
        } = self;
        SteelCtx::new_command(
            host,
            HostBundle {
                registries,
                plugin_stack,
                pending_messages,
                pending_language_regs,
                data_dir: data_dir.as_deref(),
                runtime_dir: runtime_dir.as_deref(),
                interrupt_flag: Arc::clone(interrupt_flag),
            },
            PaneId::default(),
            BufferId::default(),
            None,
        )
    }
}
