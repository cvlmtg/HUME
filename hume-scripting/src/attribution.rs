//! Plugin attribution types: `PluginId`, `Owner`, and `AttributionStack`.
//!
//! `AttributionStack` tracks whose code is running; `SteelCtx::owner` reads
//! it. The owner is recorded with every proc a script hands HUME (a
//! command, a hook handler, a callback), so the proc runs as the code that
//! defined it, and is exposed to Steel via `(command-plugin …)`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// A validated plugin identity: case-preserving for display and disk paths,
/// case-insensitive for equality and hashing.
///
/// Three valid forms:
/// - `Core(name)`: a bundled core plugin: `core:<name>`
/// - `User { user, repo }`: a third-party plugin: `<user>/<repo>`
/// - `Local(path)`: one `.scm` file beside `init.scm`: `./<path>.scm`, holding `<path>.scm`
///
/// `"SomeUser/CoolPlugin"` and `"someuser/coolplugin"` are equal on
/// case-insensitive filesystems (APFS, NTFS) while the original casing is
/// preserved for display and path construction.
#[derive(Debug, Clone)]
pub enum PluginId {
    Core(String),
    User { user: String, repo: String },
    Local(String),
}

impl PluginId {
    /// Parse and validate a plugin name string.
    ///
    /// Valid forms:
    /// - `core:<name>`: bundled core plugin
    /// - `<user>/<repo>`: third-party plugin (exactly one `/`)
    /// - `./<path>.scm`: a local file, relative to `init.scm`'s directory
    ///
    /// Segments must be non-empty, must not be `.` or `..`, and must not
    /// contain `/`, `\`, `"`, `:`, or NUL, ensuring the components are safe
    /// to use as filesystem path segments.  Validated by
    /// [`hume_platform::path::is_safe_segment`].
    ///
    /// Returns `Err(message)` for any other form.
    pub fn parse(name: &str) -> Result<Self, String> {
        if let Some(core_name) = name.strip_prefix("core:") {
            if !hume_platform::path::is_safe_segment(core_name) {
                return Err(format!(
                    "invalid plugin name '{name}': core name must be a non-empty path segment"
                ));
            }
            return Ok(PluginId::Core(core_name.to_string()));
        }
        if let Some(rel) = name.strip_prefix("./") {
            let mut segments = rel.rsplit('/');
            let last = segments.next().unwrap_or_default();
            let safe = segments.all(hume_platform::path::is_safe_segment);
            if !safe || EntryFile::parse(last).is_err() {
                return Err(format!(
                    "invalid local plugin path '{name}': expected './<file>.scm' relative to \
                     init.scm's directory, with no '.' or '..' segments"
                ));
            }
            return Ok(PluginId::Local(rel.to_string()));
        }
        if let Some((user, repo)) = name.split_once('/') {
            if repo.contains('/') {
                return Err(format!(
                    "invalid plugin name '{name}': expected user/repo with exactly one slash"
                ));
            }
            if !hume_platform::path::is_safe_segment(user)
                || !hume_platform::path::is_safe_segment(repo)
            {
                return Err(format!(
                    "invalid plugin name '{name}': user and repo must be non-empty valid path segments"
                ));
            }
            return Ok(PluginId::User {
                user: user.to_string(),
                repo: repo.to_string(),
            });
        }
        Err(format!(
            "invalid plugin name '{name}': expected 'core:<name>', '<user>/<repo>' or './<file>.scm'"
        ))
    }

    /// [`PluginId::parse`] for a plugin that lives in a plugin directory:
    /// a local file is an error.
    pub fn parse_installed(name: &str) -> Result<Self, String> {
        match Self::parse(name)? {
            PluginId::Local(_) => Err(format!(
                "'{name}' is a local file, not an installed plugin; declare it with \
                 (declare-plugin! \"{name}\" #:commands …)"
            )),
            id => Ok(id),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, PluginId::Local(_))
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginId::Core(name) => write!(f, "core:{name}"),
            PluginId::User { user, repo } => write!(f, "{user}/{repo}"),
            PluginId::Local(path) => write!(f, "./{path}"),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::Core => f.write_str("hume"),
            Owner::User => f.write_str("user"),
            Owner::Plugin(entry) => fmt::Display::fmt(&entry.plugin, f),
        }
    }
}

/// Case-insensitive equality (ASCII fold; plugin names are ASCII by design).
///
/// `Core("PLUM") == Core("plum")`, `User { "Alice", "Bar" } == User { "alice", "bar" }`.
/// Different variants are never equal.
impl PartialEq for PluginId {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PluginId::Core(a), PluginId::Core(b)) => a.eq_ignore_ascii_case(b),
            (PluginId::User { user: ua, repo: ra }, PluginId::User { user: ub, repo: rb }) => {
                ua.eq_ignore_ascii_case(ub) && ra.eq_ignore_ascii_case(rb)
            }
            (PluginId::Local(a), PluginId::Local(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        }
    }
}

impl Eq for PluginId {}

/// Hashes `s` with ASCII case folded, consistent with `eq_ignore_ascii_case`.
fn hash_folded<H: Hasher>(s: &str, state: &mut H) {
    for c in s.chars() {
        c.to_ascii_lowercase().hash(state);
    }
}

/// Hash must be consistent with `PartialEq`: equal IDs → equal hashes.
impl Hash for PluginId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Discriminant is hashed implicitly via the match: different variants
        // hash differently even if the inner strings happen to be the same.
        match self {
            PluginId::Core(name) => {
                0u8.hash(state);
                hash_folded(name, state);
            }
            PluginId::User { user, repo } => {
                1u8.hash(state);
                hash_folded(user, state);
                // Separator so ("ab","c") and ("a","bc") hash differently.
                // '/' cannot appear in a segment, so no legal id collides.
                '/'.hash(state);
                hash_folded(repo, state);
            }
            PluginId::Local(path) => {
                2u8.hash(state);
                hash_folded(path, state);
            }
        }
    }
}

/// A validated file name inside a plugin directory that a plugin entry loads:
/// one path segment ending in `.scm`. Compared exactly, case included, because
/// lookup requires the on-disk spelling on every filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryFile(String);

impl EntryFile {
    const MAIN: &'static str = "plugin.scm";

    /// The entry every plugin has: `plugin.scm`.
    pub fn main() -> Self {
        Self(Self::MAIN.to_string())
    }

    /// Validate `name` as an entry file: a safe path segment with a non-empty
    /// stem and a `.scm` extension.
    pub fn parse(name: &str) -> Result<Self, String> {
        let stem_ok = name
            .strip_suffix(".scm")
            .is_some_and(|stem| !stem.is_empty());
        if !stem_ok || !hume_platform::path::is_safe_segment(name) {
            return Err(format!(
                "invalid plugin entry '{name}': expected a file name ending in .scm \
                 with no path separators"
            ));
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_main(&self) -> bool {
        self.0 == Self::MAIN
    }
}

/// One loadable entry of a plugin: the unit that has its own activation
/// triggers, lifecycle state, and rollback scope.
///
/// Every plugin has a [`EntryFile::main`] entry; `#:entry` on
/// `declare-plugin!` adds others that share the plugin's directory, config
/// and identity. A local plugin has only its main entry, and that entry is
/// the file the id names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryId {
    pub plugin: PluginId,
    pub file: EntryFile,
}

impl EntryId {
    pub fn new(plugin: PluginId, file: EntryFile) -> Self {
        Self { plugin, file }
    }

    /// The `plugin.scm` entry of `plugin`.
    pub fn main(plugin: PluginId) -> Self {
        Self::new(plugin, EntryFile::main())
    }
}

/// `core:p` for the main entry, `core:p (x.scm)` for a secondary one.
impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file.is_main() {
            fmt::Display::fmt(&self.plugin, f)
        } else {
            write!(f, "{} ({})", self.plugin, self.file.as_str())
        }
    }
}

/// Whose code a Steel evaluation is running: what a command, a hook
/// handler or a callback is credited to, and what a registration from it
/// ranks as.
///
/// - [`Owner::Core`]: HUME's own runtime files, and built-in Rust commands
///   (`(command-plugin …)`'s answer for a command no script defined).
/// - [`Owner::User`]: `init.scm`, and the procs it defines.
/// - [`Owner::Plugin`]: a plugin's entry file, and the procs it defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    Core,
    User,
    Plugin(EntryId),
}

/// Who a piece of Steel code is credited to: `owner`, its author, and
/// `behalf`, whoever it runs for, which a registration it makes ranks as.
/// They differ for a command the user runs by hand, which runs on the
/// user's behalf whoever wrote it, and for what that command schedules.
#[derive(Debug, Clone)]
pub(crate) struct Provenance {
    pub(crate) owner: Rc<Owner>,
    pub(crate) behalf: hume_core::ConfigOrigin,
}

impl Provenance {
    /// `owner`'s own code, run on its own behalf: a file it wrote, or
    /// something it set up to run without anyone asking.
    pub(crate) fn own(owner: Owner) -> Self {
        let behalf = match owner {
            Owner::Core => hume_core::ConfigOrigin::Bundled,
            Owner::User => hume_core::ConfigOrigin::User,
            Owner::Plugin(_) => hume_core::ConfigOrigin::Plugin,
        };
        Self {
            owner: Rc::new(owner),
            behalf,
        }
    }

    /// The code of the command `owner` wrote, run because the user asked:
    /// a key, a `:` command, or a replay of one.
    pub(crate) fn invoked_by_user(owner: Rc<Owner>) -> Self {
        Self {
            owner,
            behalf: hume_core::ConfigOrigin::User,
        }
    }
}

/// One running piece of attributed code, innermost last.
#[derive(Debug)]
enum Frame {
    /// A plugin body, from `%begin-lazy-activation!` to
    /// `%finish-lazy-activation!`. `owner` is `entry` as an
    /// [`Owner::Plugin`], built once so reading it costs no allocation.
    Body {
        entry: Rc<EntryId>,
        owner: Rc<Owner>,
    },
    /// A command `call!` applies inside the VM, running as its definer.
    Call(Rc<Owner>),
}

/// The frames of attributed code currently running, persistent on
/// `ScriptingHost` and empty between sessions. A session's own owner, the
/// owner of whatever Rust started it, is `SteelCtx`'s and sits below every
/// frame the session pushes.
#[derive(Debug, Default)]
pub(crate) struct AttributionStack {
    frames: Vec<Frame>,
}

impl AttributionStack {
    /// Enter `id`'s body (via `begin_lazy_activation`).
    pub(crate) fn push_body(&mut self, id: EntryId) {
        let owner = Rc::new(Owner::Plugin(id.clone()));
        self.frames.push(Frame::Body {
            entry: Rc::new(id),
            owner,
        });
    }

    /// Leave the innermost frame when it is a plugin body, returning the
    /// entry whose body just ended; `None` when the innermost frame is not
    /// a body.
    pub(crate) fn pop_body(&mut self) -> Option<EntryId> {
        let Some(Frame::Body { entry, .. }) = self
            .frames
            .pop_if(|frame| matches!(frame, Frame::Body { .. }))
        else {
            return None;
        };
        Some(Rc::unwrap_or_clone(entry))
    }

    /// Enter a command `call!` applies, run as `owner`, returning the depth
    /// [`Self::drain_above`] restores.
    pub(crate) fn enter_call(&mut self, owner: Rc<Owner>) -> usize {
        let depth = self.frames.len();
        self.frames.push(Frame::Call(owner));
        depth
    }

    /// Leave every frame above `depth`, returning the plugin bodies among
    /// them innermost first. For a session's end, which drops what its
    /// Scheme never closed, and for leaving the `call!` frame
    /// [`Self::enter_call`] returned `depth` for, with any frame inside it
    /// a raise left open.
    pub(crate) fn drain_above(&mut self, depth: usize) -> Vec<EntryId> {
        self.frames
            .drain(depth.min(self.frames.len())..)
            .rev()
            .filter_map(|frame| match frame {
                Frame::Body { entry, .. } => Some(Rc::unwrap_or_clone(entry)),
                Frame::Call(_) => None,
            })
            .collect()
    }

    /// Whether a plugin body is executing, at any depth.
    pub(crate) fn in_body(&self) -> bool {
        self.innermost_body().is_some()
    }

    /// The entry of the innermost plugin body executing, below any `call!`
    /// frame inside it: the activation a registration made now belongs to.
    pub(crate) fn innermost_body(&self) -> Option<&Rc<EntryId>> {
        self.frames.iter().rev().find_map(|frame| match frame {
            Frame::Body { entry, .. } => Some(entry),
            Frame::Call(_) => None,
        })
    }

    /// How many plugin bodies are executing, nested.
    pub(crate) fn body_depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| matches!(frame, Frame::Body { .. }))
            .count()
    }

    /// Number of frames.
    pub(crate) fn len(&self) -> usize {
        self.frames.len()
    }

    /// The owner of the innermost frame, if any.
    pub(crate) fn top_owner(&self) -> Option<&Rc<Owner>> {
        self.frames.last().map(|frame| match frame {
            Frame::Body { owner, .. } | Frame::Call(owner) => owner,
        })
    }
}

#[cfg(test)]
mod tests;
