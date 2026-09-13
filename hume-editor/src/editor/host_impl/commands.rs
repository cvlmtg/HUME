//! `CommandHost` — moved out of `host_impl.rs`'s per-capability split.

use crate::editor::registry::{MappableCommand, TypedBody, TypedCommand};

use super::EditorHostImpl;
use hume_scripting::host::CommandHost;

impl<'a> EditorHostImpl<'a> {
    /// Shared guard behind `register_lazy_command`/`register_lazy_typed_command`:
    /// both claim `name` in the same registry namespace and differ only in
    /// what they insert on success. Returns `Ok(true)` when the caller should
    /// go on to insert its stub, `Ok(false)` for a duplicate `declare-plugin`
    /// call by the same plugin (first declaration wins, nothing to insert).
    fn claim_lazy_name(
        &self,
        name: &str,
        plugin: &hume_scripting::attribution::PluginId,
    ) -> Result<bool, String> {
        if let Some(owner) = self.state.config.registry.lazy_owner(name) {
            return if owner == plugin {
                Ok(false)
            } else {
                Err(format!("'{name}' already claimed by lazy plugin '{owner}'"))
            };
        }
        if self.state.config.registry.contains(name) {
            return Err(format!("'{name}' conflicts with an existing command"));
        }
        Ok(true)
    }
}

impl<'a> CommandHost for EditorHostImpl<'a> {
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        // `get_mappable` misses a name claimed by a typed command (it returns
        // `None` for a `Command::Typed` entry, same as for a free name) —
        // `contains` is the kind-agnostic check needed to reject that case too.
        let claimed_by_other = match self.state.config.registry.get_mappable(&def.name) {
            Some(MappableCommand::Lazy { .. }) => false,
            Some(_) => true,
            None => self.state.config.registry.contains(&def.name),
        };
        if claimed_by_other {
            return Err(format!(
                "define-command!: '{}' conflicts with existing command",
                def.name
            ));
        }
        self.state
            .config
            .registry
            .register(MappableCommand::SteelBacked {
                name: def.name.into(),
                doc: def.doc.into(),
                arity: def.arity,
                is_variadic: def.is_variadic,
                inline_output: def.inline_output,
                repeatable: def.repeatable,
            });
        Ok(())
    }

    fn register_typed_command(
        &mut self,
        def: hume_scripting::SteelTypedCmdDef,
    ) -> Result<(), String> {
        let claimed_by_other = match self.state.config.registry.get_typed(&def.name) {
            Some(tc) => !matches!(tc.body, TypedBody::Lazy(_)),
            None => self.state.config.registry.contains(&def.name),
        };
        if claimed_by_other {
            return Err(format!(
                "define-typed-command!: '{}' conflicts with existing command",
                def.name
            ));
        }
        self.state.config.registry.register_typed(TypedCommand {
            name: def.name.into(),
            doc: def.doc.into(),
            aliases: &[],
            body: TypedBody::Steel {
                arity: def.arity,
                is_variadic: def.is_variadic,
                inline_output: def.inline_output,
            },
            completer: None,
        });
        Ok(())
    }

    fn unregister_command(&mut self, name: &str) {
        self.state.config.registry.unregister(name);
    }

    fn register_lazy_command(
        &mut self,
        name: &str,
        plugin: &hume_scripting::attribution::PluginId,
    ) -> Result<(), String> {
        if self.claim_lazy_name(name, plugin)? {
            self.state.config.registry.register(MappableCommand::Lazy {
                name: name.to_owned().into(),
                plugin: plugin.clone(),
            });
        }
        Ok(())
    }

    fn register_lazy_typed_command(
        &mut self,
        name: &str,
        plugin: &hume_scripting::attribution::PluginId,
    ) -> Result<(), String> {
        if self.claim_lazy_name(name, plugin)? {
            self.state.config.registry.register_typed(TypedCommand {
                name: name.to_owned().into(),
                // No doc for a not-yet-loaded stub — mirrors `MappableCommand::Lazy`,
                // which carries no doc field at all (`MappableCommand::doc()`
                // answers `""` for it too).
                doc: std::borrow::Cow::Borrowed(""),
                aliases: &[],
                body: TypedBody::Lazy(plugin.clone()),
                completer: None,
            });
        }
        Ok(())
    }

    /// The plugin owning `name`'s `Lazy` stub — mappable or typed alike.
    /// See [`crate::editor::registry::CommandRegistry::lazy_owner`].
    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::attribution::PluginId> {
        self.state.config.registry.lazy_owner(name).cloned()
    }

    /// The plugin owning `name`'s *mappable* `Lazy` stub only.
    /// See [`crate::editor::registry::CommandRegistry::lazy_mappable_owner`].
    fn lazy_mappable_command_owner(
        &self,
        name: &str,
    ) -> Option<hume_scripting::attribution::PluginId> {
        self.state
            .config
            .registry
            .lazy_mappable_owner(name)
            .cloned()
    }

    fn unregister_lazy_stubs_of(&mut self, plugin: &hume_scripting::attribution::PluginId) {
        self.state.config.registry.unregister_lazy_stubs_of(plugin);
    }

    fn is_valid_register_name(&self, ch: char) -> bool {
        hume_ops::register::is_valid_register_name(ch)
    }

    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.state
            .config
            .registry
            .get_mappable(name)
            .map(MappableCommand::is_native)
            .ok_or_else(|| format!("unknown command: {name}"))
    }

    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        let Some(cmd) = self.state.config.registry.get_mappable(name).cloned() else {
            return Err(format!("unknown command: {name}"));
        };
        if !cmd.is_native() {
            return Err(format!(
                "{name} is not a native command — use call! instead of call-native!"
            ));
        }
        // Arm the register prefix so register-aware commands (yank, delete,
        // paste-after, …) route to the right destination.
        if let Some(r) = register {
            self.state.register_prefix =
                Some(crate::editor::register_ops::RegisterPrefix::Selected(r));
        }
        // Delegate to the shared pipeline — all bookkeeping (paste session, jump
        // list, dot-repeat) lives there so the sync path is identical to the
        // keypress path.
        crate::editor::commands::run_dispatch_pipeline(
            self.state,
            self.view,
            cmd,
            crate::editor::dispatch::CmdCtx {
                // `count` came from `parse_count_extend`, which decodes a
                // Steel-side count of 0 to `None` — the script's way of asking
                // for "as if no count was typed" (move-down/move-up read this
                // as visual-line movement instead of buffer-line movement).
                count,
                extend,
            },
        );
        // Clear the prefix when we armed it, so it does not bleed into the
        // next interactive command.
        if register.is_some() {
            self.state.register_prefix = None;
        }
        Ok(())
    }
}
