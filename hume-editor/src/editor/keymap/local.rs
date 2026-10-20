//! Named keymaps attached to buffers, looked up before the global keymap.

use std::borrow::Cow;

use hume_engine::pipeline::BufferId;
use rustc_hash::FxHashMap;
use termina::event::KeyEvent;

use super::{BindMode, Keymap, WalkResult};

/// Every named keymap, and which buffers each is attached to.
#[derive(Default)]
pub(in crate::editor) struct LocalKeymaps {
    maps: FxHashMap<Box<str>, Keymap>,
    /// Per buffer, the attached names, oldest first. Every name is a key of
    /// `maps`, and no buffer maps to an empty list.
    attached: FxHashMap<BufferId, Vec<Box<str>>>,
}

/// An attach or detach named a keymap nothing has bound into.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::editor) struct UnknownKeymap;

/// What a key sequence resolves to in a buffer.
pub(in crate::editor) struct Resolved {
    pub(in crate::editor) walk: WalkResult,
    /// The match is an Extend-mode override, which runs its command without
    /// extending.
    pub(in crate::editor) extend_override: bool,
}

impl LocalKeymaps {
    /// Bind `keys` in the keymap `name`, creating the keymap if needed.
    pub(in crate::editor) fn bind(
        &mut self,
        name: String,
        mode: BindMode,
        keys: &[KeyEvent],
        command: Cow<'static, str>,
        force_extend: bool,
    ) {
        self.maps
            .entry(name.into_boxed_str())
            .or_insert_with(Keymap::empty)
            .bind_user_with_extend(mode, keys, command, force_extend);
    }

    /// Remove a binding from the keymap `name`, which is never created here.
    pub(in crate::editor) fn unbind(&mut self, name: &str, mode: BindMode, keys: &[KeyEvent]) {
        if let Some(keymap) = self.maps.get_mut(name) {
            keymap.unbind_user(mode, keys);
        }
    }

    /// Attach the keymap `name` to `buffer` as its newest keymap. Attaching
    /// one that is already attached keeps its place.
    pub(in crate::editor) fn attach(
        &mut self,
        buffer: BufferId,
        name: &str,
    ) -> Result<(), UnknownKeymap> {
        let (name, _) = self.maps.get_key_value(name).ok_or(UnknownKeymap)?;
        let names = self.attached.entry(buffer).or_default();
        if !names.contains(name) {
            names.push(name.clone());
        }
        Ok(())
    }

    /// Detach the keymap `name` from `buffer`, if it is attached.
    pub(in crate::editor) fn detach(
        &mut self,
        buffer: BufferId,
        name: &str,
    ) -> Result<(), UnknownKeymap> {
        if !self.maps.contains_key(name) {
            return Err(UnknownKeymap);
        }
        if let Some(names) = self.attached.get_mut(&buffer) {
            names.retain(|n| n.as_ref() != name);
            if names.is_empty() {
                self.attached.remove(&buffer);
            }
        }
        Ok(())
    }

    pub(in crate::editor) fn forget_buffer(&mut self, buffer: BufferId) {
        self.attached.remove(&buffer);
    }

    /// The names attached to `buffer`, oldest first.
    #[cfg(test)]
    pub(in crate::editor) fn attached(&self, buffer: BufferId) -> Vec<&str> {
        self.attached
            .get(&buffer)
            .into_iter()
            .flatten()
            .map(AsRef::as_ref)
            .collect()
    }

    /// Resolve `keys` in `buffer`: its attached keymaps, newest first, then
    /// `global`. The first keymap the keys match in, as a binding or as the
    /// start of one, decides. `BindMode::Extend` looks keys up as Extend mode
    /// does: each keymap's extend overrides, then its normal bindings.
    pub(in crate::editor) fn resolve(
        &self,
        global: &Keymap,
        buffer: BufferId,
        mode: BindMode,
        keys: &[KeyEvent],
    ) -> Resolved {
        let attached = self.attached.get(&buffer).into_iter().flatten().rev();
        attached
            .map(|name| &self.maps[name])
            .chain(std::iter::once(global))
            .find_map(|keymap| keymap.resolve(mode, keys))
            .unwrap_or(Resolved {
                walk: WalkResult::NoMatch,
                extend_override: false,
            })
    }
}

impl crate::editor::Editor {
    /// Resolve `keys` in the focused buffer. Every key handler looks keys up
    /// here.
    pub(in crate::editor) fn resolve_keys(&self, mode: BindMode, keys: &[KeyEvent]) -> Resolved {
        let config = &self.state.config;
        config
            .local_keymaps
            .resolve(&config.keymap, self.focused_buffer_id(), mode, keys)
    }
}

#[cfg(test)]
mod tests;
