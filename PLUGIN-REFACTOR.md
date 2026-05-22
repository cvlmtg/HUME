# PLUGIN-REFACTOR — Architecture Reference

**Audience:** a future Claude session picking up this refactor.

**Purpose:** Faithful as-is map of HUME's plugin/scripting system, split into two
parts: the baseline the refactor starts from (`main@203e0e7`) and the delta already
done on `treesitter@f690856`. Together they form the "what exists" brief before
planning any restructuring.

**Scope:** the Steel/plugin layer — `ScriptingHost`, `SteelCtx`, the builtin API,
the ledger/attribution system, hooks, command registry, keymap, plugin loading, fs
sandbox, watchdog, and the grammar/language Steel API. Engine-side tree-sitter
internals (`TreeSitterHighlighter`, `ScopeRegistry`, `LoadedGrammar`/libloading,
reparse pipeline) are mentioned only at the Steel API boundary.

**Not in scope yet:** refactor opinions, pain points, or decision points. Those
get added in a separate step. §3 contains the first forward-design section: the
lazy loading roadmap, decisions locked with the author on `2026-05-20`.

**Line-number caveat:** anchors in §1 are `main@203e0e7`; anchors in §2 are
`treesitter@f690856`. Verify with `git show main:<path>` / `git show
treesitter:<path>`.

---

## §1 — Current plugin architecture (`main`)

### 1.1 Layering and invariants

HUME embeds [Steel](https://github.com/mattwparas/steel) (a Scheme dialect,
`steel-core 0.8.2`) as its scripting and configuration language. Steel's `Engine`
is `!Send` and must run on the main thread. One `ScriptingHost` owns the `Engine`
and all persistent scripting state for the editor's process lifetime.

Key crate boundaries:
- **`editor/src/scripting/`** — the scripting host, `SteelCtx`, watchdog, ledger,
  hooks, and all built-in Steel functions.
- **`editor/src/editor/registry.rs`** — `CommandRegistry`, `MappableCommand`,
  `TypedCommand`.
- **`editor/src/editor/keymap.rs`** — key trie, per-mode keymaps.
- **`runtime/`** — Scheme files shipped with the binary: `init.scm.example`,
  core plugins (`plum`, `helix-surround`), themes.

### 1.2 `ScriptingHost` — `editor/src/scripting/mod.rs:538`

The sole owner of persistent scripting state.

```rust
pub(crate) struct ScriptingHost {
    engine: Engine,                                      // private
    pub(crate) plugin_stack: PluginStack,
    pub(crate) ledger_stack: LedgerStack,
    pub(crate) cmd_owners: HashMap<String, String>,      // command-name → owner display
    pub(crate) hooks: HookRegistry,
    pub(crate) pending_messages: Vec<(Severity, String)>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) runtime_dir: Option<PathBuf>,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    hook_program_cache: HashMap<(usize, usize), String>, // private; cached composite hook programs
}
```

Public methods: `new` (591), `eval_init` (630), `teardown_plugin` (659),
`reload_plugin` (695), `call_steel_cmd` (871), `fire_hook` (928).
Private: `eval_plugin_with_attribution` (728), `eval_source_raw` (759),
`process_pending_cmds` (823).
Test-only: `eval_source` (576).

### 1.3 `SteelCtx<'a>` — the borrowed per-eval view — `mod.rs:181`

`SteelCtx<'a>` is a short-lived struct assembled at each eval/call/hook site. It
borrows persistent fields from `ScriptingHost` (and from the caller's `Editor`
refs) for the duration of one Steel evaluation, then those borrows are released.

**Borrowed `&'a mut` fields** (from `ScriptingHost` or caller):
```
settings, keymap, plugin_stack, ledger_stack, cmd_owners,
hooks, pending_messages,
data_dir / runtime_dir: Option<&'a Path>,
buffers, engine_view, pane_state, pane_jumps: Option<&'a mut ...>,
```

**Owned transient fields** (fresh each eval, discarded on return):
```
declared_plugins, loaded_plugins,
builtin_cmd_names: HashSet<String>,
pending_steel_cmds: Vec<PendingSteelCmd>,
interrupt_flag: Arc<AtomicBool>,
cmd_queue: Vec<String>,
wait_char_request: Option<String>,
pending_char: Option<char>,
cmd_arg: Option<String>,
is_init: bool,                        // init path vs command path
focused_pane_id, focused_buffer_id, live_focused_buffer_id,
```

`CustomReference` + `custom_reference!(SteelCtx<'a>)` at :243–244 — these are
required by Steel's `with_mut_reference` API.

Two helper structs pack the ref-bundles: `EditorSteelRefs<'a>` (412) and
`HostBundle<'a>` (429). `SteelCtxTestHarness` (333) is a `#[cfg(test)]` struct
with owned backing storage; its `.ctx()` method produces a `SteelCtx` for unit
tests without a live editor.

### 1.4 Eval flow — `run_steel` and `eval_source_raw`

**`run_steel`** (`mod.rs:449`) is the central ceremony for every evaluation:

```rust
// 1. Arm watchdog (spawns a thread with park_timeout)
let watchdog = EvalWatchdog::arm(interrupt_flag, budget);
// 2. Park &mut SteelCtx into the engine via with_mut_reference
engine
    .with_mut_reference::<SteelCtx<'a>, SteelCtx<'static>>(&mut steel_ctx)
    .consume_once(|engine, args| {
        let ctx_val = args.into_iter().next().expect("with_mut_reference yields one arg");
        engine.update_value(HUME_CTX, ctx_val);        // park as *hume.ctx*
        let res = engine.compile_and_run_raw_program(source);
        engine.update_value(HUME_CTX, SteelVal::Void); // clear, release borrow
        res
    });
// 3. Cancel watchdog, reset interrupt flag
watchdog.cancel();
interrupt_flag.store(false, Ordering::Relaxed);
```

`HUME_CTX` = `"*hume.ctx*"` (const at :50). Every builtin registered via
`register_fn_with_ctx(HUME_CTX, name, fn)` receives `&mut SteelCtx` as its
injected first argument, giving it access to all editor state the ctx borrows.

**`eval_source_raw`** (`mod.rs:759`) handles the borrow-scoping dance: it
destructures `&mut *self` into a `HostBundle` so that `self`'s fields (borrowed
into `SteelCtx`) and `self` (needed for `EvalSnapshot::restore` on error) don't
conflict.

**`EvalSnapshot`** (`mod.rs:490`):
```rust
struct EvalSnapshot {
    settings, keymap, plugin_stack, ledger_stack, cmd_owners, hooks,
    hooks_version_at_capture: u32,
}
```
`capture` clones all six fields. `restore` overwrites them — but **skips the
hooks write-back unless `hooks.version` changed** (avoids clobbering registrations
made during eval on success). `pending_messages` is **deliberately NOT reverted**
(488–489) so failure messages survive a rollback.

`EvalSnapshot` is used exclusively by `eval_source_raw` (init/plugin-load path).
**`call_steel_cmd` and `fire_hook` perform no rollback** — config mutation builtins
(`set-option!`, `bind-key!`) are blocked in command mode via `is_init` guards.

### 1.5 Engine init and builtin registration — `builtins/mod.rs:75`

`ScriptingHost::new` (591) must call `fs::init_dirs` **before** `Engine::new()` so
sandbox paths are available when the BOOTSTRAP is compiled (597–599).

Invariant: `engine.register_value(HUME_CTX, SteelVal::Void)` at :79 **must be
the very first call** before any `register_fn_with_ctx`. The
`supply_context_arg` codegen references this global at registration time; absence
raises `FreeIdentifier` at engine init.

Context-aware builtins: `register_fn_with_ctx(HUME_CTX, "name", fn)`.  
Context-free builtins: `register_fn` / `register_value`.

The `BOOTSTRAP` const (57) is a Scheme snippet compiled and run via
`compile_and_run_raw_program` at the end of `new`. It defines `load-plugin`:

```scheme
(define (load-plugin name)
  (push-declared-plugin! name)
  (let ((path (resolve-plugin-path name)))
    (when path
      (push-loaded-plugin! name)
      (dynamic-wind
        (lambda () (push-current-plugin! name))
        (lambda () (load path))        ; loads plugin file inline
        (lambda () (pop-current-plugin!))))))
```

The `dynamic-wind` guarantees `pop-current-plugin!` runs even if the plugin's
code errors.

### 1.6 Watchdog and cooperative interrupt — `mod.rs:104`

`EvalWatchdog::arm(flag, budget)` spawns a thread that calls
`std::thread::park_timeout(budget)`. If not cancelled in time it sets
`flag.store(true)`. `cancel(self)` sets a cancel flag, calls `unpark()`, and
joins the thread (immediate defuse).

Budgets: `steel_init_budget_ms` (used at :766), `steel_command_budget_ms` (at
:878, :962).

Interruption is **cooperative only** — Steel 0.8.2 has no involuntary
op-callback. The sole check point is `(hume/yield!)` (`builtins/interrupt.rs:43`),
which reads `ctx.interrupt_flag` and calls `steel::stop!` if set.

### 1.7 Filesystem sandbox — `builtins/fs.rs`

`ScriptDirs` holds canonicalized sandbox roots, stored in a
`thread_local! SCRIPT_DIRS: RefCell<Option<ScriptDirs>>` (:51).

- `init_dirs` (90): eager `canonicalize`, Windows `\\?\` prefix strip (71–86).
- `is_under_write_sandbox` (149): `<data>/plugins/` only.
- `is_under_read_sandbox` (157): `<data>/plugins/` + `<runtime>/plugins/`.
- Path validation helpers: `has_dotdot` (174), `normalize_lexical` (184),
  `canonical_ancestor_join` (206).
- TOCTOU-safe: direct `canonicalize` on the read/stat operations, never
  `.exists()` before reading.

`with_data_plugins` (137) exposes the write root to `shell.rs` for the git
builtins.

### 1.8 Command system — `registry.rs`, `mappings.rs`

**`MappableCommand`** (registry.rs:74) — five variants, all keyed by name string:

| Variant | Signature |
|---------|-----------|
| `Motion` | `fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet` |
| `Selection` | `fn(&Text, SelectionSet, MotionMode) -> SelectionSet` |
| `Edit` | `fn(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet)` |
| `EditorCmd` | `fn(&mut Editor, usize, MotionMode) -> Result<(), CommandError>` |
| `SteelBacked` | `{ steel_proc: String, extendable: bool }` |

`steel_proc` is the mangled lambda name stored in the Steel namespace (e.g.
`%hume-cmd-my-command`), written there by `process_pending_cmds`.

**`TypedCommand`** (256): `{ name, doc, aliases: &'static [&'static str], fun:
fn(&mut Editor, Option<&str>, bool/*force*/) -> Result<(), CommandError> }`.

**`CommandRegistry`** (290): single `HashMap<Cow<str>, Command>` (mappable and
typed share one namespace) + `alias_map`. `register_defaults` wires ~137
built-in commands. `unregister_all_steel_backed` (347) is used by `:reload-config`
to clear Steel-backed commands before rebuilding the engine.

**`:` dispatch** — `execute_command` (mappings.rs:1299): `parse_typed_command`
(force = single trailing `!`, stripped; name = `[A-Za-z_-]+`) → look up typed →
fallback to mappable (count=1, arg) → "Unknown command".

**Keypress → Steel command** — `execute_keymap_command` (601): on `SteelBacked`
branch (674) calls `host.call_steel_cmd(steel_proc, pending_char, cmd_arg,
EditorSteelRefs{...})`. The returned `(cmd_queue, wait_char_request)` are
processed by recursively calling `execute_keymap_command` for each name in
`cmd_queue` and entering `WaitChar` mode if requested. `call_steel_cmd` returns
`Result<(Vec<String>, Option<String>), String>` with no rollback.

### 1.9 Keymap — `keymap.rs`, `scripting/keys.rs`

`KeymapCommand { name: String, force_extend: bool }` (keymap.rs:64). The
`force_extend` flag is an **internal engine concern** — it is NOT settable via the
`bind-key!` builtin; only `bind-key-extend!` (from `keymap_bind.rs`) produces it.

`WalkResult` (77) = `Leaf(KeymapCommand)` | `Interior { name }` | `WaitChar` |
`NoMatch`. `KeyTrie` (103) wraps a tree of `KeyTrieNode = Leaf | Node | WaitChar`.

`Keymap { normal, extend, insert }` (286) — three per-mode tries. Methods:
`bind_user_with_extend(mode, keys, command, force_extend)` (346),
`unbind_user`, `lookup_command`.

`parse_key_sequence` (keys.rs:42): whitespace-separated tokens; modifiers
`ctrl-`/`shift-`/`alt-` (any order, case-insensitive); named keys (`esc`,
`tab`, `enter`, `space`, arrows, `f1`–`f12`, etc.); single Unicode char
preserves case; `shift-tab` → `BackTab`.

**Steel keymap builtins** (`builtins/keymap_bind.rs`): all init-only, all record
ledger entries via `bind_inner`:
- `bind-key! mode key command` (89)
- `bind-key-extend! mode key command` (117) — sets `force_extend = true`
- `unbind-key! mode key` (142)
- `bind-wait-char! cmd wait-char-cmd` (179) — enters WaitChar mode

### 1.10 Plugin loading and lifecycle

**Startup** — `Editor::init_scripting` (`editor/mod.rs:971`):
1. Resolve `config_dir` via `os::dirs::config_dir()` (XDG_CONFIG_HOME/hume or
   %APPDATA%).  If unresolved, log warning and skip scripting.
2. `init_path = config_dir/init.scm`.
3. `ScriptingHost::new()` — calls `fs::init_dirs` + builds engine + `register_all`
   + eval `BOOTSTRAP`.
4. `host.eval_init(&init_path, settings, keymap, &builtin_names)`. Missing file →
   `Ok(Vec::new())` (normal — users copy `init.scm.example`).
5. Register returned `SteelCmdDef`s into `CommandRegistry`.

**Plugin resolution** (`builtins/plugins.rs:80`):
- `Core(name)` → `<runtime>/plugins/core/<name>/plugin.scm`
- `User { user, repo }` → `<data>/plugins/<user>/<repo>/plugin.scm`
- Absent path → `None` → silently skipped (still declared, not loaded).

**Runtime dir** (`os/dirs.rs:116`): `HUME_RUNTIME` env → `../share/hume` relative
to exe → `./runtime` cwd fallback.

**`declared-plugins` vs `loaded-plugins`**: declared = every `(load-plugin …)`
call; loaded = subset that resolved + evaluated. plum's `:plum-install` uses
declared to know what to install.

### 1.11 Ledger attribution — `scripting/ledger.rs`

The ledger is the undo stack for plugin mutations to settings, keybindings, and
commands. It enables correct teardown and reload.

```rust
pub enum PluginId { Core(String), User { user: String, repo: String } }
// case-insensitive Eq/Hash; case-preserving Display.

pub enum Owner { Core, User, Plugin(PluginId) }
// Core = "hume" builtins; User = top-level init.scm; Plugin = a named plugin.

pub struct LedgerEntry {
    pub key: String,
    pub prior_value: String,
    pub prior_force_extend: bool,
    pub prior_owner: Owner,
}
// Records only the *prior* state; the live value stays in settings/keymap.

pub struct Ledger { plugin: PluginId, entries: Vec<LedgerEntry> }
// Per plugin. ≤1 entry per key: first mutation wins (subsequent mutations by the
// same plugin are no-ops on the ledger — the prior was already captured).

pub struct LedgerStack { ledgers: Vec<Ledger> } // oldest-first
// record(234): dedup per-key. owner_of(269): newest→oldest, last-writer-wins.
// Default owner: Owner::Core.
```

`PluginStack`: tracks the currently-executing plugin. `current_owner()`: `last()`
→ `Plugin(id)`, empty → `User`. Top-level `init.scm` code runs with `User` owner
→ no ledger entry recorded.

`cmd_owners: HashMap<String, String>` (`ScriptingHost`, mod.rs:548): command-name →
display string. Populated by `process_pending_cmds` (842). `(command-plugin name)`
returns from here, defaulting to `"hume"`.

**Key disambiguation** in `restore_ledger_entry` (mod.rs:1026):
- `BindMode::from_ledger_prefix` match → keymap entry
- Space-containing but no valid mode prefix → hard error (corrupt prefix guard)
- Otherwise → settings entry

Prefixes written by builtins:
- Settings: bare key (e.g. `"tab-width"`)
- Keymap: mode-prefixed (e.g. `"normal f"`, via `mode.ledger_prefix()`,
  `keymap_bind.rs:52`)
- Commands: `"cmd:<name>"`

### 1.12 Teardown and reload

**`teardown_plugin`** (`mod.rs:659`):
1. `hooks.purge_plugin(&id)` — removes all `(Owner::Plugin(id), _)` handlers.
2. `ledger_stack.unload(&id)` (`ledger.rs:294`) — splice rewrite algorithm:
   for each entry being removed, if a *newer* plugin also touched the same key,
   rewrite the successor's `prior_value`/`prior_owner`/`prior_force_extend` to
   what the removed plugin saw (live value stays unchanged); otherwise return the
   entry for restoration.
3. Returned entries: `cmd:` prefix → push name to `cmds_to_remove`; otherwise
   call `restore_ledger_entry`.

**`restore_ledger_entry`** (mod.rs:1021):
- Settings: `apply_setting(key, prior_value)` — must mirror `serialize_setting`;
  both arms must be added in the same commit (enforced by round-trip test).
- Keybinds: `bind_user_with_extend(prior_force_extend)` or `unbind_user` when
  `prior_value.is_empty()`.

**`reload_plugin`** (695): teardown → re-resolve path → `eval_plugin_with_attribution`.

**`:reload-plugin`** (`commands.rs:1636`): unregisters old cmds from `CommandRegistry`,
registers new ones from reload.

**`:reload-config`** (1668): calls `unregister_all_steel_backed()` first (clears
all Steel-backed commands so their names pass the builtin-conflict check on
re-eval), drops the host, calls `init_scripting` from scratch.

**Conflict check in `define_command_inner`** (`builtins/commands.rs:103`): rejects
names in `builtin_cmd_names` ("conflicts with a built-in") and intra-session
duplicates.

### 1.13 Hooks — `scripting/hooks.rs`

```rust
pub enum HookId { OnBufferOpen, OnBufferClose, OnBufferSave, OnEdit, OnModeChange }

const HOOKS: &[(HookId, &str)] = &[
    (HookId::OnBufferOpen,  "on-buffer-open"),
    (HookId::OnBufferClose, "on-buffer-close"),
    (HookId::OnBufferSave,  "on-buffer-save"),
    (HookId::OnEdit,        "on-edit"),
    (HookId::OnModeChange,  "on-mode-change"),
];

pub struct HookRegistry {
    handlers: HashMap<HookId, Vec<(Owner, SteelVal)>>,
    pub version: u32,  // bumped on every register call
}
```

`register(65)`: appends `(owner, proc)`. `handlers_for(74)`: order preserved.
`is_empty_for(82)`: fast-path. `purge_plugin(87)`: retain-out by owner match.

`(register-hook! 'symbol proc)` (`builtins/hooks.rs:17`): init-only (`is_init`
guard). Owner from `plugin_stack.current_owner()`.

**`fire_hook`** (`mod.rs:928`):
1. Early-return `Ok(vec![])` if no handlers (941–943).
2. Pre-bind args as `hook-arg-N` and handler procs as `hook-proc-N` via
   `engine.register_value`.
3. Build or fetch cached composite Steel program keyed by `(arg_count,
   handler_count)` from `hook_program_cache` (`build_hook_program` at :69).
4. Run all handlers in **one** `with_mut_reference` session under the command
   budget.
5. Extract `cmd_queue` from the ctx.
6. Null `hook-arg-N` and `hook-proc-N` globals back to `Void` (998–1005) to
   release any Arc refs pointing at closed buffers.
7. Return `Result<Vec<String>, String>`.

**Firing call sites** (`editor/mod.rs`): `fire_hook_silent` (926, extracts and
dispatches queued commands), `fire_hook_buffer_save` (915), OnModeChange (:1253,
args = old/new mode strings), OnBufferOpen (:1488, arg = `SteelBufferId(bid)`),
OnBufferClose (:1508, arg = `SteelBufferId(bid)`), OnBufferSave from `:w`
(`commands.rs:1452`, :1466).

### 1.14 Builtin API catalog

All functions receive `&mut SteelCtx` as their first arg (registered via
`register_fn_with_ctx`), unless noted as context-free.

| File | Steel functions | Capability |
|------|----------------|-----------|
| `builtins/commands.rs` | `define-command!`, `define-command-extend!`, `call!`/`call-command!`, `request-wait-char!`, `cmd-arg`, `command-plugin`, `pending-char` | Define Steel-backed commands; queue command execution; read single-char arg; look up command owner |
| `builtins/keymap_bind.rs` | `bind-key!`, `bind-key-extend!`, `unbind-key!`, `bind-wait-char!` | Bind/unbind keys in normal/extend/insert mode; set up wait-char sequences |
| `builtins/buffers.rs` | `current-buffer`, `current-pane`, `buffers`, `panes`, `buffer-path`, `buffer-name`, `buffer-dirty?`, `open-buffer!`, `close-buffer!`, `switch-to-buffer!`, pane stubs | Read/write multi-buffer and pane state (read ops any context; write ops command-mode only) |
| `builtins/fs.rs` | `data-dir`, `runtime-dir`, `path-join`, `path-exists?`, `list-dir`, `make-dir`, `delete-dir`, `log!` | Sandboxed filesystem access (writes to `<data>/plugins/`, reads also `<runtime>/plugins/`) |
| `builtins/shell.rs` | `git-clone url dest`, `git-pull dir` | Git operations sandboxed to `<data>/plugins/` |
| `builtins/plugins.rs` | `push-declared-plugin!`, `push-loaded-plugin!`, `push-current-plugin!`, `pop-current-plugin!`, `resolve-plugin-path`, `loaded-plugins`, `declared-plugins` | Drive the Scheme `load-plugin` wrapper; Core vs User path resolution |
| `builtins/hooks.rs` | `register-hook! 'name proc` | Register a hook handler (init-only) |
| `builtins/settings.rs` | `set-option! key value` | Mutate editor settings (init-only, ledger-aware) |
| `builtins/statusline.rs` | `configure-statusline! left center right` | Set statusline element lists (init-only, ledger-aware) |
| `builtins/interrupt.rs` | `hume/yield!` | Cooperative interruption check |
| `builtins/ids.rs` | `buffer-id?`, `pane-id?`, `buffer-id=?`, `pane-id=?` | Predicate + value-equality for opaque ID types (context-free) |
| `builtins/keymap_bind.rs` | *(see above)* | *(see above)* |

### 1.15 Opaque IDs — `builtins/ids.rs`

`SteelBufferId(pub BufferId)` and `SteelPaneId(pub PaneId)` implement Steel's
`Custom` trait. `fmt` renders as `#<buffer-id N>` / `#<pane-id N>` using slotmap's
`as_ffi`. Predicates `buffer-id?` / `pane-id?` are context-free.

**Critical caveat (ids.rs:74–78):** Steel `equal?` on `Custom` values uses
Arc-pointer equality. Two separate `SteelBufferId` wrappers around the same
underlying `BufferId` are **not** `equal?` unless they share the Arc allocation.
Always use `buffer-id=?` / `pane-id=?` (ids.rs:106–113) for value equality in
Scheme code.

### 1.16 Runtime plugins

**`core:plum`** (`runtime/plugins/core/plum/plugin.scm`): HUME's package manager.
- `:plum-install`, `:plum-cleanup`, `:plum-update`, `:plum-list`
- Uses `declared-plugins` (from init.scm) and an `installed-plugins` disk walk of
  `<data>/plugins/` to determine what to install/remove.
- Pre-treesitter plum: `git-clone` + `git-pull`; no grammar management yet.

**`core:helix-surround`** (`runtime/plugins/core/helix-surround/plugin.scm`):
- Binds `m s` / `m d` / `m r` as surround shortcuts via `bind-wait-char!`.
- Calls `(unbind-key! "normal" "m w")` on load.

---

## §2 — Changes introduced in the `treesitter` branch

Commit range: `main@203e0e7..treesitter@f690856` (110 commits). Each subsection
describes a self-contained change-set as a portable unit. The subsections are
roughly ordered from most-structural to most-additive.

### 2.1 Module reorganization (pure code-movement)

On `main`, `editor/src/scripting/mod.rs` is a ~1040-line monolith.
On `treesitter` it is split into:

| New file | Contents moved from `mod.rs` |
|----------|------------------------------|
| `host.rs` | `ScriptingHost` struct + all its methods |
| `eval.rs` | `eval_source_raw`, `run_steel`, `process_pending_cmds` |
| `steel_ctx.rs` | `SteelCtx`, `EvalSnapshot`, result types, `SteelCtxTestHarness` |
| `refs.rs` | `EditorSteelRefs`, `HostBundle` |
| `watchdog.rs` | `EvalWatchdog` |
| `mod.rs` | re-exports only |

`builtins/buffers.rs` (single 1107-line file on `main`) split into a subdirectory:
```
builtins/buffers/{mod,enumerate,focus,language,mutate,pane_stubs,properties,tests}.rs
```
Two new builtin files added: `builtins/grammar.rs`, `builtins/syntax.rs`.

Editor-side splits (`editor/` directory):
- `registry.rs` → `registry/{mod,builtins_editor_cmd,builtins_motion,
  builtins_selection,builtins_typed,tests}.rs`
- `commands.rs` → `commands/` subdirectory
- `mappings.rs` → `mappings/` subdirectory
- `mod.rs` → sub-modules
- New files: `scripting_glue.rs`, `syntax.rs`, `syntax_glue.rs`, `render_coord.rs`

Engine splits (out-of-scope mechanics, noted for orientation):
- `engine/src/format.rs` → `format/` subdirectory
- `engine/src/style.rs` → `style/` subdirectory

These are purely structural moves with no semantic changes. When porting, apply
the new file layout first.

### 2.2 Four-phase startup orchestration

On `main`, startup evaluates only `config_dir/init.scm` (one `eval_init` call).
Plugins are loaded inline from within `init.scm` via `(load-plugin ...)`.

On `treesitter`, startup uses four ordered phases (`editor/src/editor/
scripting_glue.rs:83`, new file):

1. **`prelude.scm`** (`runtime_dir/scheme/prelude.scm`) → `eval_init`. Must not
   define commands (assertion enforced). Defines Scheme macros available to all
   subsequent files.
2. **`languages.scm`** (`runtime_dir/scheme/languages.scm`) → `eval_init`. Must
   not define commands. After eval, `apply_pending_language_regs` is called to
   flush the queued language identity registrations so plugins can use language
   names in step 3.
3. **`plugins.scm`** (`config_dir/plugins.scm`) → `eval_plugins_scm` (new method).
   See §2.3.
4. **`init.scm`** (`config_dir/init.scm`) → `eval_init`. Builtin-names set is
   re-captured first to include commands plugins registered in step 3.

After step 4: `apply_pending_language_regs` again (for `define-language!` calls
in `init.scm`), pick up `set-option! "history-capacity"`, flush `log!` messages.

### 2.3 `plugins.scm` pre-init phase — `host.rs:163`

`eval_plugins_scm` is a new `ScriptingHost` method:

1. Eval `plugins.scm` manifest with `eval_source_raw`. The new `load-plugin`
   Scheme wrapper (see §2.4) no longer calls `(load path)` inline; it only
   calls `push-declared-plugin!` + `%queue-plugin-load!`. After the manifest
   finishes, `self.pending_plugin_loads: Vec<String>` (new host field) holds
   every queued name in declaration order.
2. Drain `pending_plugin_loads` and for each name:
   - Parse `PluginId`, resolve path.
   - Call `eval_plugin_with_attribution` (same as the current `load-plugin`
     mechanism but now as a separate `eval_source_raw` call).
   - **Drain `process_pending_cmds` between iterations.** This is the key
     change: plugin A's `%hume-cmd-*` Steel globals are registered into the
     engine before plugin B compiles, enabling cross-plugin `(call! "name")`.
3. Per-plugin errors are soft-failure (push `Severity::Error` to
   `pending_messages`); one bad plugin does not abort the rest.
4. Returns `Ok([])` if `plugins.scm` is not on disk.

`runtime/plugins.scm.example` — new file, a template for users to copy to
`~/.config/hume/plugins.scm` listing their `(load-plugin ...)` calls.

### 2.4 Prelude and macro layer — `runtime/scheme/prelude.scm`

New file on `treesitter`; also now on `plugins` (see **Plugins branch** note below).
Loaded before `init.scm`. Defines Scheme macros that improve ergonomics over the
raw Rust-registered builtins (which carry a `%` prefix to mark them as internal).

**On `treesitter`:** contains `define-language!` (wraps `%define-language!`) and
`bind-keys!`; also documents `call!` (defined in BOOTSTRAP). Loaded before
`languages.scm` and `init.scm` via `scripting_glue.rs:123`.

**On `plugins` branch** (`runtime/scheme/prelude.scm`, loaded in `init_scripting`
before `init.scm`): `define-language!` is **not** portable (no `%define-language!`
builtin here). Importable subset: three batch keymap macros + room to grow.

```scheme
;; bind-keys! — batch bind-key!
(define-syntax bind-keys!
  (syntax-rules ()
    ((_ mode (key cmd) ...) (begin (bind-key! mode key cmd) ...))))

;; bind-keys-extend! — batch bind-key-extend!
(define-syntax bind-keys-extend!
  (syntax-rules ()
    ((_ mode (key cmd) ...) (begin (bind-key-extend! mode key cmd) ...))))

;; unbind-keys! — batch unbind-key!
(define-syntax unbind-keys!
  (syntax-rules ()
    ((_ mode key ...) (begin (unbind-key! mode key) ...))))
```

Verified: global `define-syntax` macros defined before `(require)`ing a plugin
module ARE visible inside the module body (Steel 0.8.2, test
`global_define_syntax_is_visible_inside_required_module`). Missing prelude is a
silent no-op; a prelude that fails to parse/eval is reported as `Severity::Error`
and `init.scm` still runs.

The `%`-prefix convention: `%call!`, `%in-init-mode?`, `%queue-plugin-load!` (all
treesitter-only) are Rust-registered primitives; user code calls the unprefixed
macro. `call!` on the `plugins` branch is defined in BOOTSTRAP (not the prelude).

### 2.5 Variadic `call!` and honest arity — commit `ff6a17f`

**On `main`:**
- `cmd_queue: Vec<String>` — command names only, no arguments.
- `cmd-arg` side-channel: set by the caller before invoking a Steel command,
  read inside the command lambda via `(cmd-arg)`.
- `call_steel_cmd` returns `Result<(Vec<String>, Option<String>), String>`.

**On `treesitter` — commit `ff6a17f`:**
- `cmd_queue: Vec<(String, Vec<SteelVal>)>` — name + positional args.
- `cmd-arg` side-channel **retired**. `call-command!` alias **removed**.
- `%call!` Rust primitive (`builtins/commands.rs`): `(%call! name args-list)` →
  `ctx.cmd_queue.push((name, args_vec))`.
- `call!` is a **plain** variadic Scheme macro defined in BOOTSTRAP (not only in
  `prelude.scm`, so it is available in all engine contexts including test harnesses):
  ```scheme
  (define-syntax call!
    (syntax-rules () ((_ name args ...) (%call! name (list args ...)))))
  ```
- `SteelCmdDef` gains `arity: u16` (from `ByteCodeLambda::arity()`) and
  `is_variadic: bool` (from `is_multi_arity()`), captured before `register_value`
  takes ownership of the closure.
- Minibuffer arity rule: arity-0 → drop the typed arg silently; arity-1 or
  variadic → pass as `StringV` or `BoolV(false)` when no arg given; arity ≥ 2
  non-variadic → report user error (minibuffer can only supply one string).

**Mode-aware `call!` — commit `f690856` (separate; out of scope for `plugins`):**
`f690856` ("feat: enable init-mode call! via drain-between-plugins") adds
`%in-init-mode?` and a mode-branching `call!` that resolves commands synchronously
during init (enabling cross-plugin calls from `plugins.scm`). This depends on
four-phase startup and `plugins.scm`, neither of which exist on the `plugins`
branch. `ff6a17f`'s plain macro is the right subset to import; `f690856` is not.
`%queue-plugin-load!` is also from `f690856`, not `ff6a17f`.

**Adjacent tree-sitter work (separate from `ff6a17f`):** `define-command-inline-output!`
— a new command variant that streams output inline rather than to the message bar.
Scheduled as a §3 prerequisite (see the inline-output item in §3.9).

### 2.6 `SteelCtx` / result-type changes — `steel_ctx.rs`

On `treesitter`, the return types of `call_steel_cmd` and `fire_hook` are named
structs:

```rust
pub(crate) struct SteelCmdResult {
    pub(crate) cmd_queue: Vec<(String, Vec<SteelVal>)>,
    pub(crate) wait_char_request: Option<String>,
    pub(crate) pending_language_sets: PendingLanguageSets,  // Vec<(BufferId, Option<String>)>
    pub(crate) grammar_sweeps: Vec<String>,
}

pub(crate) struct HookResult {
    pub(crate) cmd_queue: Vec<(String, Vec<SteelVal>)>,
    pub(crate) pending_language_sets: PendingLanguageSets,
    pub(crate) grammar_sweeps: Vec<String>,
}

pub(crate) struct TeardownResult {
    pub(crate) cmds_to_remove: Vec<String>,
    pub(crate) langs_to_remove: Vec<String>,
}
```

**New `SteelCtx` fields vs `main`:**

| Field | Type | Owner | Purpose |
|-------|------|-------|---------|
| `pending_language_regs` | `&'a mut Vec<PendingLanguageReg>` | borrowed (host-owned) | queued `define-language!` / `register-grammar!` registrations |
| `pending_plugin_loads` | `&'a mut Vec<String>` | borrowed (host-owned) | plugin names queued by `%queue-plugin-load!` |
| `pending_language_sets` | `PendingLanguageSets` | transient (owned) | `set-buffer-language!` deferred effects |
| `pending_grammar_sweeps` | `Vec<String>` | transient (owned) | grammar names needing open-buffer reattach |
| `languages` | `Option<&'a mut LanguageRegistry>` | borrowed (caller) | language/grammar registry; `None` during init |
| `builtin_cmd_names` | `Option<&'a HashSet<String>>` | was owned `HashSet` on `main` | now optional reference; `None` during some evals |

`PendingSteelCmd` and `SteelCmdDef` gain `inline_output: bool`, `arity: u16`,
`is_variadic: bool`.

### 2.7 Language identity decoupling — Steel API

(Engine internals are out of scope; this section covers the Steel-facing API.)

**New data model at the boundary:**
- `Buffer.language: Option<String>` — SSOT for language identity, independent of
  grammar. Set via `Editor::set_buffer_language(bid, opt_name)`.
- `Editor.languages: LanguageRegistry` — maps language names to identity configs
  (exts, globs, shebangs) and optional attached grammar bundles.

**`%define-language!` / `define-language!` macro** (`builtins/syntax.rs`):
- Init-only.
- Queues `PendingLanguageReg::Identity { name, extensions, globs, shebangs,
  owner }` onto `ctx.pending_language_regs` (owned by `ScriptingHost`).
- Applied to `Editor.languages` after init finishes via
  `apply_pending_language_regs`.

**`PendingLanguageReg` enum:**
- `Identity { name, extensions, globs, shebangs, owner }`
- `Grammar { name, grammar_path: PathBuf, symbol: String, highlights_path: PathBuf }`

**`buffer-language` / `set-buffer-language!`** (`builtins/buffers/language.rs`):
- Command-mode only (`require_cmd_ctx!` guard).
- `set-buffer-language!`: sets `Buffer.language` immediately (so a subsequent
  `buffer-language` call in the same eval sees the new value), then defers
  `setup_buffer_syntax` + `OnLanguageSet` hook firing via `ctx.pending_language_sets`.
- After `call_steel_cmd` or `fire_hook` returns, `scripting_glue.rs` drains
  `pending_language_sets` before executing `cmd_queue`.

**`OnLanguageSet` hook** (new `HookId` variant, `hooks.rs`):
- Symbol: `on-language-set`.
- Fires `(bid, name-or-#f)` on every language transition (`SteelBufferId` + either
  `SteelVal::StringV(name)` or `SteelVal::BoolV(false)` for `None`).
- Fired from `Editor::set_buffer_language` on every call (including auto-detect on
  file open).

**`:set buffer language=…`** — intercepted in `typed_set` before reaching
`apply_setting`. `:set global language=…` → error (language is buffer-only).

**Detect and registry** (boundary mention): `detect_language(path, first_line,
registry)` returns `Option<String>` using glob > ext > shebang priority.
`LanguageRegistry` lives on `Editor.languages`; methods: `by_name`, `attach_grammar`,
`has_grammar`. Ledger entries keyed `lang:<name>`.

### 2.8 Grammar Steel API — compile and install

**`register-grammar!`** (`builtins/syntax.rs`):

- **Init path** (`is_init = true`): queues `PendingLanguageReg::Grammar { name,
  grammar_path, symbol, highlights_path }`. Fail-soft (deferred).
- **Command path** (`is_init = false`): calls `languages.attach_grammar(...)` on
  `ctx.languages` immediately, then `engine_view.theme.bake(...)`, then pushes
  `name` to `ctx.pending_grammar_sweeps`. Fail-hard (`steel::stop!`).

Grammar attach (engine internals, boundary mention): `attach_grammar` dlopen's
the `.so`/`.dylib`/`.dll` via libloading. This is the **only `unsafe` block in
HUME** — field order in `LoadedGrammar` is load-bearing (`language` before
`_library` so the language pointer drops before the library closes).
`pending_grammar_sweeps` drives `Editor::reattach_grammars`, which re-runs
`setup_buffer_syntax` for all open buffers using the named language.

**`compile-grammar!`** (`builtins/grammar.rs`):
- `(compile-grammar! src-dir out-path)` — shells out to `tree-sitter build -o
  <out-path> <src-dir>`.
- Init: fail-soft (logs Warning, returns `#<void>`). Command: fail-hard.
- On spawn failure: surfaces "install tree-sitter CLI" hint.
- Output sandbox: `<data>/grammars/` (hard-fail canonicalize, `..` rejected).

**`grammar-output-path`** (`builtins/grammar.rs`): `(grammar-output-path name)`
→ `"<data>/grammars/<name>.<ext>"` where ext = `dylib` (macOS), `dll` (Windows),
`so` (Linux).

**`language-has-grammar?`** (`builtins/syntax.rs`): command-mode only, checks
`LanguageRegistry::has_grammar(name)` → `#t` / `#f`.

**New shell builtins** (`builtins/shell.rs`):
- `(git-clone-rev url dest rev)` — sandboxed to `<data>/plugins/` + `<data>/grammars/`.
- `(curl-fetch url dest)` — same sandbox.

**`delete-file`** — registered as `SteelVal::FuncV(fs::delete_file)` (not via
`register_fn_with_ctx`); sandbox = `<data>/grammars/` only.

**fs sandbox extension**: `is_under_write_sandbox` and `is_under_read_sandbox`
both extended to also accept `<data>/grammars/`.

### 2.9 Ledger attribution extensions

**`record_lang_ledger_entries`** (host.rs:309 on treesitter): called after
`apply_pending_language_regs`; writes `lang:<name>` ledger entries for each
language registered by the current plugin. Enables teardown to cleanly remove a
plugin's language registrations.

**`TeardownResult`** (new struct): `teardown_plugin` now returns `{ cmds_to_remove,
langs_to_remove }` instead of just `cmds_to_remove`. The caller passes
`langs_to_remove` to `Editor::remove_languages`.

**Grammar-source ownership moved to plum Scheme** (commit `9f00e69`): earlier
treesitter commits had a `PendingLanguageReg::GrammarSource` variant and a
`ScriptingHost.grammar_sources: HashMap<String, GrammarSource>` field. These were
removed and replaced by plum's `grammars.scm` pure-Scheme declarations. On
`treesitter@f690856`, there is no `GrammarSource` type in the Rust core.

### 2.10 Plum plugin restructure — grammar management

On `main`, plum is a single `plugin.scm`. On `treesitter`, `plugin.scm` is a thin
loader that `require`s three sub-files:

```scheme
(require "lib.scm")     ; shared helpers: valid-dir-entry?, batch-run
(require "plugins.scm") ; :plum-install, :plum-update, :plum-cleanup, :plum-list
(require "grammars.scm"); grammar download, compile, register commands
```

Then declares grammar sources and runs `plum/register-installed-grammars!`.

**`grammars.scm`** — new grammar management commands:
- `:plum-install-grammar name` — git-clone source + compile + `register-grammar!`
- `:plum-update-grammar name` — re-fetch rev + recompile + re-register
- `:plum-list-grammars` — list declared / installed / grammar-loaded status
- `:plum-cleanup-grammars` — remove `.so`/`.dylib` files for undeclared grammars

Helper Steel procedures:
- `plum/declare-grammar-source! name url rev [symbol]` — pure data: records
  source URL/rev/symbol for a grammar name; symbol defaults to
  `"tree_sitter_" + name`.
- `plum/register-installed-grammars!` — scans `<data>/grammars/` for
  platform-appropriate shared library files and calls `register-grammar!` for
  each found (using the corresponding highlights path if available).
- `plum/ensure-grammars! names` — for each name: install if missing, otherwise
  attach; useful in `init.scm`.

**New Scheme data files** (`runtime/scheme/`):
- `grammar-sources.scm` (~358 lines): list of `(plum/declare-grammar-source! …)`
  calls for Helix-pinned grammars. Pure data; no commands.
- `helix-pin.scm`: `(define helix-pin "<sha>")` — the Helix repo commit that the
  grammar sources in `grammar-sources.scm` were ported from. Used for
  reproducibility.
- `languages.scm` (~320 entries): `(define-language! ...)` calls ported from
  Helix's `languages.toml`. Provides default language identities for common
  languages (rust, python, javascript, …). Loaded as step 2 of startup.

### 2.11 Miscellaneous smaller deltas

- **`:redraw`** typed command (`builtins_typed.rs`): force a full terminal redraw.
- **`first_line` 64-byte cap**: shebang detection reads only the first 64 bytes of
  a file to determine the interpreter name, capping the scope of the read.
- **Theme syntax scopes**: `engine/src/builtins/tree_sitter_hl.rs` extended with
  syntax token scope mappings (grammar capture names → theme style keys). The
  `bake` method on `Theme` integrates these with the `ScopeRegistry`.
- **`ui.cursor.insert`** theme fix: themes that were missing `ui.cursor.insert = {}`
  now have it (absence caused Insert-mode head cell to inherit the Normal-mode
  block background).

---

## §3 — Lazy plugin loading: design and roadmap

**Status:** design, not yet implemented. Targets the `plugins` branch.

### 3.1 Why, and the one hard constraint

Eager loading runs every `plugin.scm` at startup: Steel compile per plugin, plus
(once grammars land) `dlopen` per grammar. Lazy loading defers a plugin's body
until something actually needs it, so you pay only for what you use.

The hard constraint: **to defer a body you must know its triggers without running
it** — running the body *is* loading. So triggers are declared in a manifest that
is always evaluated. On this branch the manifest is `init.scm` itself (the user's
`load-plugin` calls); the plugin body is the deferred payload.

Two recent changes make this *simpler*, not harder:
- **Unload was removed** → lazy loading is **load-once, no restore**. No ledger, no
  prior-state capture.
- **Attribution was kept** (`PluginStack`/`Owner`) → lazy activation is the same
  push/eval/pop as eager load. Attribution "just works."

### 3.2 Locked decisions

| Decision | Choice |
|----------|--------|
| Opt-in | Eager by default; any declared trigger ⇒ lazy; `#:lazy #t` forces lazy |
| Where triggers live | User's `init.scm`, as `load-plugin` keyword args |
| Trigger types | Command, Event/hook, Language/filetype |
| Keys | Bound via existing `bind-key!`; binding to an `#:on-command` name makes the key a trigger. No `#:keys` keyword, no key-trigger machinery |
| Failure | Fail-fast: mark `Failed`, report once, no per-trigger retry |
| Unload | None — load-once |

### 3.3 Manifest API — `init.scm`

`load-plugin` gains keyword arguments. No keywords ⇒ eager (today's behavior).

```scheme
(load-plugin "user/foo")                       ; eager — unchanged

(load-plugin "user/bar"
  #:on-command '("bar" "bar-baz")              ; load when :bar / (call! "bar") / a key→"bar"
  #:on-event   '(on-buffer-save)               ; load when hook fires
  #:on-language '("rust" "toml")               ; load when buffer language set (Phase 3b)
  #:lazy #t)                                   ; optional explicit override

;; Keys use the existing bind-key!. Binding to a declared #:on-command name
;; turns the key into a load trigger — no #:keys keyword needed:
(bind-key! "normal" "<leader>b" "bar")
```

- Presence of any `#:on-*` ⇒ lazy.
- `#:lazy #t` with no trigger ⇒ lazy until an explicit `(require-plugin "name")`
  (or an idle event, Phase 4).
- Keys are bound with the existing `bind-key!` (and `bind-key-extend!` /
  `bind-wait-char!`). A key bound to a name listed in `#:on-command` resolves to
  that command's lazy stub, so the key becomes a load trigger automatically — no
  `#:keys` keyword. `bind-key!` does **not** validate the name (keymap_bind.rs:66),
  so binding a key to a command absent from every `#:on-command` just warns
  "unknown command" on press (mappings.rs:608/1335); declare it in `#:on-command`
  to make it trigger.

### 3.4 Plugin lifecycle

```
Declared { path, triggers } ──► Loading ──► Loaded
                                     └──────► Failed
```

- **Eager**: `Declared → Loaded` during init.
- **Lazy**: stays `Declared` until a trigger fires, then `activate`.
- `activate_plugin(id)`:
  1. `Loaded`/`Failed` ⇒ no-op (idempotent; `Failed` does not retry).
  2. Set `Loading` (re-entrancy guard for trigger cycles A→B→A).
  3. Push `PluginStack`, eval body (`eval_plugin_with_attribution`),
     drain `process_pending_cmds` (real registrations overwrite stubs), pop.
  4. Set `Loaded`; on error set `Failed` + push a `Severity::Error` message.
  5. Drop this plugin's entries from the trigger maps.

### 3.5 Activation registry (Rust)

New state on `ScriptingHost` (trigger maps) coordinated with `Editor` (keymap /
command registry / future language registry):

```rust
struct LazyRegistry {
    plugins:           HashMap<PluginId, PluginState>,
    command_triggers:  HashMap<String, PluginId>,        // cmd name → owner
    event_triggers:    HashMap<HookId, Vec<PluginId>>,
    language_triggers: HashMap<String, Vec<PluginId>>,   // Phase 3b
}

enum PluginState { Declared { path: PathBuf }, Loading, Loaded, Failed }
```

Keys are **not** stored here — they are ordinary `bind-key!` keymap leaves whose
target command name is a key in `command_triggers`.

### 3.6 Dispatch interception — one mechanism per chokepoint

- **Command — registry stub.** For each `#:on-command` name, register
  `MappableCommand::Lazy { plugin }` (new variant) in `CommandRegistry` under that
  name. **Collision check at registration:** if the name is already known (a
  built-in, an already-loaded plugin's command, or another lazy stub), the
  colliding trigger is **dropped** — a `Severity::Error` is logged to `:messages`
  and init continues (never silently shadows; see §3.10). The existing name lookup finds
  the stub; dispatch matches `Lazy` → `activate_plugin` → re-dispatch. The body's
  `define-command!` overwrites the stub with the real `SteelBacked`. **Loop guard:**
  if the entry is still `Lazy` after activation (author never defined it), report
  error + remove stub + treat as unknown — never re-enter. Chokepoints:
  `execute_command` (mappings.rs:1299), `execute_keymap_command`
  (mappings.rs:601), `call_steel_cmd` (scripting/mod.rs:676). *Post-prereq-1:
  `call_steel_cmd` is **not** a direct chokepoint — it only ever receives a
  concrete Steel proc name, never a registry command name. `(call! "name")`
  queues to `cmd_queue` and drains through `execute_keymap_command`
  (mappings.rs:787); the two implemented arms cover it transitively.* **`:reload-config`
  note:** `unregister_all_steel_backed` (registry.rs:338) clears only `SteelBacked`;
  reload must also drop `Lazy` stubs + trigger maps so a fresh init rebuilds them
  (a previously lazily-`Loaded` plugin reverts to `Declared`).

- **Event — pre-fire activation.** In `fire_hook` (scripting/mod.rs:731), before
  firing `HookId H`, drain `event_triggers[H]` and `activate` each pending plugin so
  its real handlers are registered, then fire including the new handlers.

- **Language — pre-set activation (Phase 3b).** In `set_buffer_language` (imported
  from treesitter in Phase 3a), after resolving language `X` and before firing
  `OnLanguageSet`, `activate` each plugin in `language_triggers[X]`. Because the
  import is **identity-only** (no `setup_buffer_syntax` call), the activation point
  is simply "language field set" — a plugin that later adds grammars hooks
  `OnLanguageSet` itself. This chokepoint does not exist on `plugins` yet; Phase 3a
  imports it.

### 3.7 Why keys need no machinery

HUME invariant: keys bind to **named commands** (select-then-act; no key-to-key
remapping). So `(bind-key! "normal" "m s" "foo-surround")` is just an ordinary
trie leaf pointing at the command name `foo-surround`. If `foo-surround` is listed
in a lazy plugin's `#:on-command`, that name resolves to a `MappableCommand::Lazy`
stub; pressing the sequence walks to the leaf, dispatch finds the stub, activates
the plugin, and re-dispatches to the real command. There is **no key-specific lazy
machinery**: the keymap is unchanged and the only lazy artifact is the command stub,
shared with `:foo-surround` / `(call! "foo-surround")`. This holds for multi-key
sequences (the whole sequence is one leaf — prefix/`Interior` nodes fire nothing)
and for `bind-wait-char!` commands (the real command runs on re-dispatch and
requests WaitChar as usual).

### 3.8 Interactions

- **Attribution**: activation push/pop is identical to eager; `cmd_owners` is
  populated on load. `(command-plugin name)` answers correctly pre-load by
  pre-seeding `cmd_owners` from the manifest at trigger-registration time (manifest
  is SSOT — resolved §3.10).
- **Unload**: none. Load-once is fully compatible with the ledger removal.
- **BOOTSTRAP rewrite**: `load-plugin` (builtins/mod.rs:57) changes from inline
  `(load path)` to: parse keywords → register triggers + command stubs → if eager,
  `activate` now; if lazy, return (deferred). New Rust primitives back the Scheme
  wrapper: `%declare-plugin!`, `%register-command-trigger!`,
  `%register-event-trigger!`, `%register-language-trigger!`.

### 3.9 Roadmap

- **Prerequisite — variadic `call!` + drop `cmd-arg` (port `ff6a17f`).**
  Orthogonal to lazy loading but worth doing first: small, self-contained, removes
  a side channel, and Phase 1 command-trigger re-dispatch benefits from honest
  arg-passing.

  *Source commit:* `ff6a17f` on `treesitter` branch. Import `ff6a17f` only — NOT
  the later `f690856` (mode-aware `call!` / `%in-init-mode?`), which depends on
  four-phase startup + `plugins.scm` absent on `plugins`.

  *Port change set (plugins-branch monolith paths):*

  1. `editor/src/scripting/builtins/mod.rs` — BOOTSTRAP: append the
     `(define-syntax call! …)` macro (`call!` stays in BOOTSTRAP, not prelude:
     test engines never load the prelude, so tests using `call!` via `eval_source`
     would break; the prelude is also optional—silent no-op if the runtime dir is
     missing—while `call!` is a core dispatch primitive that must be unconditionally
     present). Replace `call!` + `call-command!`
     registrations with `%call!` → `commands::call_command_primitive`; delete the
     `cmd-arg` registration.
  2. `editor/src/scripting/builtins/commands.rs` — rename
     `call_command(ctx, name)` → `call_command_primitive(ctx, name, args: SteelVal)`;
     add `steel_list_to_vec` helper (`steel::stop!(TypeMismatch …)` on non-list).
     Delete the `cmd_arg` builtin and its two unit tests; update the queue tests to
     the `(name, Vec<SteelVal>)` shape.
  3. `editor/src/scripting/mod.rs` —
     - add `cmd_arg_global_name(i) -> String` → `"*hume.ca{i}*"` (mirrors `hook_arg_name`);
     - `SteelCtx.cmd_queue: Vec<(String, Vec<SteelVal>)>`; delete `cmd_arg` field +
       both constructor params/inits (`new_init`, `new_command`, test harness);
     - `SteelCmdDef`: add `arity: u16` + `is_variadic: bool`;
     - `process_pending_cmds`: introspect arity before `register_value` —
       `SteelVal::Closure(gc) => (gc.arity() as u16, gc.is_multi_arity())`,
       non-closure fallback `=> (0, true)`;
     - `call_steel_cmd`: param `cmd_arg: Option<String>` → `args: Vec<SteelVal>`;
       return `(Vec<(String, Vec<SteelVal>)>, Option<String>)`; build invocation —
       empty args → `(proc)`, else bind each as `*hume.ca{i}*` then
       `(proc *hume.ca0* …)`; null out globals after run (Arc release);
     - `fire_hook`: return `Vec<(String, Vec<SteelVal>)>`; drop the `None` `cmd_arg`
       ctor arg.
  4. `editor/src/editor/registry.rs` — `MappableCommand::SteelBacked`: add
     `arity: u16` + `is_variadic: bool`.
  5. `editor/src/editor/mappings.rs` —
     - `execute_keymap_command`: param `cmd_arg: Option<String>` → `steel_args: Vec<SteelVal>`;
       pass into `call_steel_cmd`; post-Steel drain loop →
       `for (cmd_name, cmd_args) in queue`;
     - keymap call sites (WaitChar, normal leaf, insert leaf) → `vec![]`;
     - minibuffer dispatch: apply arity rule on `SteelBacked` (0 → `vec![]`,
       1/variadic → `StringV`-or-`BoolV(false)`, ≥2 non-variadic → user error);
       remove the `(cmd-arg)` comment.
  6. `editor/src/editor/mod.rs` — hook drain → `for (cmd_name, cmd_args) in cmd_queue`.
  7. `editor/src/editor/commands.rs` — `cmd_repeat`: `None` → `vec![]`.
  8. `SteelBacked` registry-construction site (locate via `rg "SteelBacked \{"`) —
     thread `arity` + `is_variadic` from `SteelCmdDef`.
  9. Tests — `tests/visual_move.rs` call sites → `vec![]`; `scripting/tests.rs`
     `call_steel_cmd(…, None, None, …)` → `…, vec![], …`; delete `call-command!`
     alias test.
  10. `runtime/init.scm.example` — remove `call-command!` mention.

  *Verification:* `cargo test` green; `rg "cmd-arg|cmd_arg|call-command!"` returns
  nothing in `editor/` + `runtime/`. New unit tests for arity rule (independent-oracle,
  flip arity to confirm test catches wrong branch).

  *Optional — named result structs:* the port widens `call_steel_cmd`/`fire_hook`
  tuple returns. As an optional readability step (do it in the same port or skip),
  replace the growing tuples with named structs `SteelCmdResult { cmd_queue,
  wait_char_request }` / `HookResult { cmd_queue }` following treesitter §2.6—minus
  the grammar fields.

- **Prerequisite — `define-command-inline-output!`.**
  Self-contained, non-lazy-loading import. Adds a Steel command variant that brackets
  dispatch with an alt-screen exit so shelling-out commands (plum install, formatters,
  linters) stream live output to the terminal rather than dumping to the message bar.
  Flow: exit alt-screen → print `--- running {name} ---` banner → run command → print
  "press any key to return" → wait for keypress → re-enter alt-screen → force full
  redraw. Source: treesitter (adjacent to `ff6a17f`; see §2.5).

  **Do this AFTER the variadic-`call!` port:** both touch `SteelCmdDef`,
  `PendingSteelCmd`, `define_command_inner`, `MappableCommand::SteelBacked`, the
  `mappings.rs:674` dispatch arm, and registry tests. Avoid double-editing those.

  *Port change set (plugins-branch monolith paths):*

  1. `editor/src/os/terminal.rs` — port `enter_inline_output(kitty, mouse)` +
     `leave_inline_output(kitty, mouse, mouse_select)` verbatim from
     `treesitter:editor/src/os/terminal.rs` (~60 lines). Deps: alt-screen,
     `Push/PopKeyboardEnhancementFlags`, raw mode, mouse `\x1b[?…` toggles — all
     already used in this branch's terminal init.
  2. `editor/src/scripting/mod.rs` — add `inline_output: bool` to `PendingSteelCmd`
     and `SteelCmdDef`.
  3. `editor/src/scripting/builtins/commands.rs` — thread `inline_output` param
     through `define_command_inner` (currently ends at `extendable`, :91); set on
     pushed `PendingSteelCmd` (:113); add `define_command_inline_output` calling
     `define_command_inner(…, extendable=false, inline_output=true)`.
  4. `editor/src/scripting/builtins/mod.rs` — register `"define-command-inline-output!"`
     → `commands::define_command_inline_output`.
  5. `editor/src/editor/registry.rs` — `MappableCommand::SteelBacked` gains
     `inline_output: bool` and `name: String` (for the banner; treesitter §2.6:
     SteelBacked already carried `name` by `f690856`).
  6. `editor/src/editor/mod.rs` — add `force_full_redraw: bool` field (ABSENT on this
     branch); honor at the top of the render loop:
     `if std::mem::take(&mut self.force_full_redraw) { let _ = term.clear(); }`
     (treesitter `mod.rs:546` pattern — clears ratatui's diff cache after alt-screen
     toggle invalidates the terminal's previous contents).
  7. `editor/src/editor/mappings.rs:674` — in the `SteelBacked` dispatch arm, when
     `inline_output`: `enter_inline_output` → banner → `call_steel_cmd` → (unconditionally,
     success or error) "press any key" raw read → `leave_inline_output` → set
     `force_full_redraw` (treesitter `key_dispatch.rs:669-721`).
  8. Registry-construction site that builds `SteelBacked` from a `SteelCmdDef`
     (`editor/mod.rs`) — thread `inline_output` + `name`.
  9. Tests — registry-test `SteelBacked` literals gain `inline_output: false, name:
     …`; scripting test: `define-command-inline-output!` sets `inline_output=true`
     (oracle: plain `define-command!` gives `false`).
  10. `runtime/init.scm.example` — document `define-command-inline-output!`.

  *Verification:* `cargo test` green; a `define-command-inline-output!` call in the
  test harness produces a `SteelCmdDef` with `inline_output = true`.

- **Phase 0 — Manifest plumbing.** Extend `load-plugin` (BOOTSTRAP + new
  primitives), add `LazyRegistry`/`PluginState`/`activate_plugin`. Eager path
  unchanged: configs with no keywords behave exactly as today. No user-visible
  change yet.

- **Phase 1 — Command triggers (+ keys).** `MappableCommand::Lazy`, stub
  registration, re-dispatch at the three command chokepoints, `#:on-command`
  (keys via existing `bind-key!`), loop guard. Self-contained, no external deps. Fully testable (dispatch
  tests: stub present → invoke → body evaluated → real command runs → stub gone).

- **Phase 2 — Event triggers + explicit require.** `#:on-event` over the existing
  five `HookId` variants; `event_triggers` consulted in `fire_hook`. Add
  `(require-plugin "name")` builtin for programmatic / `#:lazy`-no-trigger
  activation. No external deps. — **COMPLETE (2026-05-22)**

- **Phase 3a — Import language identity from the `treesitter` branch.** Straight
  file copy (commit history irrelevant), grammars left behind. This is the
  filetype-identity layer the language trigger needs; it has standalone value
  (filetype detection, `define-language!`) independent of lazy loading.
  **Copy (identity load-bearing):**
  - Identity half of `editor/src/editor/syntax.rs`: `LanguageRegistry` (fields
    `by_name`/`by_ext`/`compiled_globs`/`glob_lang_names`/`shebang_to_name`/
    `lang_order`), `LanguageConfig` minus its `grammar` field, `detect_language`,
    `detect_shebang`, `RegisterError`.
  - `Buffer.language: Option<String>` field (`editor/src/editor/buffer.rs:80`).
  - `HookId::OnLanguageSet`.
  - `set_buffer_language` **minus the `setup_buffer_syntax` call** +
    `detect_and_set_language` (from `syntax_glue.rs`); call
    `detect_and_set_language(bid)` in `open_buffer` before the `OnBufferOpen` fire
    (main: `editor/src/editor/mod.rs:1479`/`:1488`).
  - Steel side: `define-language!` builtin (`builtins/syntax.rs`, grammar-free body)
    + prelude macro (`runtime/scheme/prelude.scm:31`); `set-buffer-language!` /
    `buffer-language` builtins + `effective_language` helper; identity language
    registrations in `runtime/scheme/languages.scm` (pure data).
  - SteelCtx state: `PendingLanguageReg::Identity` variant, `pending_language_regs`
    field, `pending_language_sets` field; post-init flush of `pending_language_regs`.
  - **Plumb minimally:** widen `call_steel_cmd` / `fire_hook` tuple returns by one
    `pending_language_sets` element and drain in the command path (`mappings`) and
    hook path. *Non-obvious:* `OnLanguageSet` re-enters Steel, so
    `set-buffer-language!` cannot fire it inline — `pending_language_sets`
    defer-then-drain is genuinely required even for identity-only.
    **Do NOT** port the treesitter branch's named result structs, variadic
    cmd_queue, four-phase startup, or module split — those are grammar-era
    scaffolding, not identity requirements.
  - **Skip (grammar):** `GrammarBundle`/`BufferParser`, `attach_grammar`/
    `has_grammar`, `register-grammar!` / `language-has-grammar!`,
    `setup_buffer_syntax` / syntax sweep / reparse, `PendingLanguageReg::Grammar`,
    grammar sweep queues, `languages` field on SteelCtx, `Buffer.parser`.

- **Phase 3b — Language/filetype triggers.** `#:on-language`; `language_triggers`
  consulted inside the imported `set_buffer_language` (before `OnLanguageSet`).
  Self-contained once 3a lands. Highest payoff comes later when grammars arrive
  (grammar-on-demand), but identity-trigger plugins (linters, formatters keyed to a
  filetype) are useful immediately.

- **Phase 4 — Polish.** `:plugin-status` typed command (Declared/Loaded/Failed +
  trigger list), load-time reporting, idle/very-lazy event, `init.scm.example` docs.
  Optional post-init lint: warn if a keymap leaf targets a name that is neither a
  registered command nor a lazy stub (catches typos / a key bound to a command the
  user forgot to list in `#:on-command`).

### 3.10 Resolved decisions and deferred questions

**Resolved (2026-05-20):**

- **Command-name collisions** → **non-fatal Error, trigger dropped, init
  continues.** All three collision cases are uniform: (a) trigger == Rust built-in —
  filtered in `declare_plugin` via `ctx.log`; (c) trigger == another lazy plugin's
  trigger — filtered in `declare_plugin`, first-writer-wins; (b) trigger == an
  eager plugin's already-registered command — caught in `register_lazy_command_stubs`
  after the eager drain. In every case the colliding trigger is dropped (no shadow),
  a `Severity::Error` is logged to `:messages`, and init.scm continues. A plugin
  whose entire `#:on-command` list collides stays **dead-lazy** (declared, never
  fires). Language triggers are 1:many (`Vec<PluginId>`) so they never conflict —
  only the 1:1 command map can.
- **`Failed` recovery** → **stays `Failed` until `:reload-config`.** No
  per-trigger / per-keystroke retry; one error message on first failure.
- **Bare `#:lazy #t`** → **loads only via explicit `(require-plugin "name")`;**
  idle/very-lazy auto-load deferred to Phase 4. *In practice this case is rare:*
  pairing `#:lazy #t` with `#:on-command` / `#:on-event` (or a key bound via
  `bind-key!` to an `#:on-command` name) makes that the trigger, so the plugin
  loads on first key press / invocation and `#:lazy #t` is redundant.
- **Pre-load `(command-plugin)`** → **yes, pre-seed `cmd_owners` from the manifest**
  so a lazy command reports its owner before the body loads. Manifest is SSOT.

**Deferred to a post-refactor brainstorm:**

- **Plugin-defined languages × laziness.** Language *identity/detection* is owned by
  HUME (`runtime/scheme/languages.scm`, eager). A plugin keying `#:on-language` off
  an existing language is fine. But a plugin that defines its **own** language must
  register that identity **eagerly** — a lazy body cannot be the sole provider of
  its own trigger language (chicken-and-egg: the language must be detectable for the
  trigger to fire). How plugin-provided languages should interact with lazy loading
  is left for a dedicated brainstorm once the core refactor lands.

---

## Key files index

### `main` branch

```
editor/src/scripting/mod.rs           ScriptingHost, SteelCtx, eval, watchdog (monolith)
editor/src/scripting/ledger.rs        PluginId, LedgerEntry, LedgerStack, PluginStack
editor/src/scripting/hooks.rs         HookId, HookRegistry
editor/src/scripting/keys.rs          parse_key_sequence
editor/src/scripting/tests.rs         unit tests
editor/src/scripting/builtins/mod.rs  register_all, BOOTSTRAP, HUME_CTX
editor/src/scripting/builtins/commands.rs    define-command!, call!, cmd-arg
editor/src/scripting/builtins/keymap_bind.rs bind-key!, bind-key-extend!, unbind-key!
editor/src/scripting/builtins/buffers.rs     buffer access builtins (monolith)
editor/src/scripting/builtins/fs.rs          fs sandbox + path ops
editor/src/scripting/builtins/shell.rs       git-clone, git-pull
editor/src/scripting/builtins/plugins.rs     load-plugin primitives
editor/src/scripting/builtins/hooks.rs       register-hook!
editor/src/scripting/builtins/settings.rs    set-option!
editor/src/scripting/builtins/statusline.rs  configure-statusline!
editor/src/scripting/builtins/interrupt.rs   hume/yield!
editor/src/scripting/builtins/ids.rs         SteelBufferId, SteelPaneId, =? builtins
editor/src/editor/registry.rs         CommandRegistry, MappableCommand, TypedCommand
editor/src/editor/keymap.rs           KeyTrie, Keymap, KeymapCommand
editor/src/editor/mappings.rs         execute_command, execute_keymap_command
editor/src/editor/mod.rs              Editor::init_scripting, fire_hook_silent, hook firing
editor/src/os/dirs.rs                 config_dir, data_dir, runtime_dir
runtime/init.scm.example             user init template
runtime/plugins/core/plum/plugin.scm  plum package manager
runtime/plugins/core/helix-surround/plugin.scm  surround plugin
```

### `treesitter` additions / replacements

```
editor/src/scripting/host.rs          ScriptingHost (split from mod.rs)
editor/src/scripting/eval.rs          eval_source_raw, run_steel
editor/src/scripting/steel_ctx.rs     SteelCtx, EvalSnapshot, result types
editor/src/scripting/refs.rs          EditorSteelRefs, HostBundle
editor/src/scripting/watchdog.rs      EvalWatchdog
editor/src/scripting/builtins/grammar.rs    compile-grammar!, grammar-output-path
editor/src/scripting/builtins/syntax.rs     %define-language!, register-grammar!, language-has-grammar?
editor/src/scripting/builtins/buffers/language.rs  buffer-language, set-buffer-language!
editor/src/editor/scripting_glue.rs   Editor::init_scripting (4-phase orchestration)
editor/src/editor/syntax.rs           editor-side syntax integration (apply_pending_language_regs, etc.)
runtime/plugins.scm.example          user plugins.scm template
runtime/scheme/prelude.scm           define-language! macro, bind-keys!, call! docs
runtime/scheme/languages.scm         ~320 language identity definitions
runtime/scheme/grammar-sources.scm   ~358 grammar source declarations (pure data)
runtime/scheme/helix-pin.scm         helix-pin sha constant
runtime/plugins/core/plum/grammars.scm   grammar install/update/list commands
runtime/plugins/core/plum/plugins.scm    plugin install commands (split from plugin.scm)
runtime/plugins/core/plum/lib.scm        shared helpers
```

---

## Verification

Facts in §1 were cross-checked against `main@203e0e7` via:
- Three parallel Explore agents reading all files under `editor/src/scripting/`,
  `editor/src/editor/`, and `runtime/`.
- Direct `git show` reads of key files.

Facts in §2 were cross-checked against `treesitter@f690856` via `git show
treesitter:<path>` and `git diff main treesitter -- <path>`. Memory files were
**not** used as the authoritative source — several memory claims were stale (e.g.
`grammar_sweeps` not `grammar_reattaches`; `SteelCmdResult` is a named struct not
a tuple; `cmd_queue` now carries args; `GrammarSource` was moved out of Rust core
into plum Scheme). Memory was used only for initial orientation.

Re-verify any specific anchor:
```bash
# Read a file at a specific branch:
git show main:editor/src/scripting/mod.rs | head -100
git show treesitter:editor/src/scripting/host.rs

# See what changed between branches for a file:
git diff main treesitter -- editor/src/scripting/hooks.rs

# Full diff stat:
git diff --stat main treesitter
```
