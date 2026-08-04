# HUME Editor — Command Dispatch & State Architecture

## Why this exists

Dispatching a command from Steel requires two things to be true simultaneously:
the Steel VM must be running (borrowing `&mut steel`), and the editor must be
reachable for the handler to mutate it. Two structural constraints shape the
dispatch model:

1. **Re-entrancy.** While a script runs, the Steel `Engine` is borrowed `&mut`
   as the executor. A fresh eval (`compile_and_run_raw_program`) cannot start
   while that borrow is held — so Lazy plugin activation defers. Applying an
   already-registered engine-global closure is a funcall on the running call
   stack, not a fresh eval; activated plugin commands are not blocked.

2. **Borrow aliasing.** The `Engine` sits as a sibling of `EditorState` and
   `EngineView` under the outer `Editor` shell
   (`Editor → { scripting → steel ; state: EditorState ; view: EngineView }`).
   This makes `&mut steel`, `&mut state`, and `&mut view` provably disjoint,
   so any sync handler receiving `(&mut EditorState, &mut EngineView)` can run
   safely inside an eval without aliasing the running VM.

## The core move

Editor state lives in a sibling subtree so the Steel VM and editor data are
**cousins that never alias**:

`Editor → { scripting → steel ; state: EditorState ; view: EngineView }`.
The Steel engine sits under `scripting`; command-mutable document/mode data
sits under `state`; render/view state (full-fat panes, layout, theme) sits
under `view`. `&mut state`, `&mut view`, and `&mut steel` are all provably
disjoint.

## Ownership tree

```
Editor                         // thin app shell: lifecycle + three subtrees
├── scripting: Option<ScriptingHost>  // None until init_scripting
│   ├── steel: Engine          // the Scheme VM handle — always "steel", never
│   │                          //   bare "engine" (see D1)
│   ├── registries: ScriptingRegistries  // persistent scripting registries:
│   │                                    //   command ownership, hooks,
│   │                                    //   lazy/declared plugins, command
│   │                                    //   table; disjoint from steel — NLL
│   │                                    //   split
│   └── …                      // infra fields: plugin_stack, pending_messages,
│                               //   data_dir, interrupt_flag, etc.
├── state: EditorState         // document/command state — everything a command
│                              //   can mutate (see D2)
└── view: EngineView           // render/view state: full-fat panes, layout tree,
                               //   theme (hume-engine/ crate type, see D3)
```

### `EditorState` — the command-mutable boundary

Everything a command can read or mutate lives on `EditorState`: buffers, mode,
pending input, registers, kill ring, settings, command registry, keymap, search
state, per-pane bookkeeping (`panes: PaneView`), the deferred-hook channel
(`pending_events`), and so on. The struct definition
(`hume-editor/src/editor/mod.rs`) is the authoritative field list.

Two kinds of state deliberately stay on the outer `Editor` instead:

- **Arc-wrapped render comms** (`bracket_hl_data`, `search_hl_data`,
  `completion_view`) — shared with the render thread as `Arc` clones, so no
  `&mut` borrow of `EditorState` is needed to reach them.
- **Lifecycle shell fields** (`scripting`, `parse_worker`, `kitty_enabled`, …) —
  app plumbing no command touches.

## Borrow story at eval time

To run the VM, four disjoint `&mut` borrows are taken simultaneously (post-init,
once `editor.scripting` is `Some`):

```
&mut editor.scripting.as_mut().unwrap().steel       // the executor
&mut editor.scripting.as_mut().unwrap().registries  // NLL field-split from steel
&mut editor.state                                   // sibling of scripting
&mut editor.view                                    // sibling of scripting and state
```

None aliases another. `state` and `view` back an `EditorHostImpl` passed to the
eval context as `&mut dyn EditorHost`:

```rust
pub(crate) struct EditorHostImpl<'a> {
    pub(crate) state: &'a mut EditorState,
    pub(crate) view:  &'a mut EngineView,
}
```

Inside `ScriptingHost`, the `steel_and_bundle` helper performs the NLL field-split
of `steel` vs. everything else, returning `(&mut Engine, HostBundle<'_>)`. `HostBundle`
carries `registries: &mut ScriptingRegistries` alongside the infrastructure borrows
(`plugin_stack`, `pending_messages`, `pending_language_regs`, `data_dir`,
`runtime_dir`, `interrupt_flag`) — all disjoint from `steel`, so the VM can run
while the bundle is live.

## Dispatch invariant

> **Defer a command iff it needs to re-enter the engine with a fresh eval.**

Activated plugin commands apply their closure inline on the running call stack — no
fresh eval, no deferral. The routing lives in Steel (`%dispatch-command`).

| Command kind | Dispatch |
|---|---|
| Motion / Selection / Edit | **sync** |
| EditorCmd | **sync** |
| EditorCmd that fires a hook | command **sync**; handler pushes to `ConfigState::pending_events`; `drain_events` fires after the input event completes (semantic defer — see D5) |
| TypedCommand (`:` commands) | **sync** — but stays on `fn(&mut Editor, …)`, not Steel-dispatchable (see D7) |
| SteelBacked (activated plugin) | **sync** — `%dispatch-command` applies the closure inline via `(apply proc args)` |
| Lazy | **defer** — needs a fresh eval to activate |

### `call!` → `%dispatch-command` routing

`call!` is the sole dispatch primitive. It desugars to
`(%dispatch-command name (list args…))`, which routes by command kind:

- **`%lookup-plugin-proc` returns a closure** (activated plugin command): `(apply proc args)` —
  the closure runs inline on the Steel call stack, synchronous with the caller. Buffer state
  read after the call reflects its effects immediately.
- **`%lookup-plugin-proc` returns `#f`** (native, lazy, or unknown): `%call-native!` —
  native commands run synchronously via `run_command_sync`; lazy/unknown commands are queued.

`%call-native!` is the Rust leaf; the `call!` macro always desugars to `%dispatch-command` which falls back to `%call-native!` for native/unknown commands.

**Arg → count/extend mapping for native commands:**

| Args | count | extend |
|---|---|---|
| `(call! "name")` | 1 | `false` |
| `(call! "name" n)` | `n` (clamped to ≥ 1) | `false` |
| `(call! "name" n #t)` | `n` (clamped to ≥ 1) | `true` |
| any other shape | error raised to Steel | — |

For lazy/unknown (queued), args flow through unchanged.
`register_command_names` emits `(define name (lambda () (call! "name")))` wrappers —
bare `(move-left)` resolves to `(call! "move-left")` with no args (count=1, Normal).

## Decisions

### D1 — Naming hygiene: "engine" is overloaded, disambiguate everywhere

Two unrelated things are called "engine" in HUME:

- the **`hume-engine/` workspace crate** (rendering pipeline, pane geometry), and
- the **Steel `Engine`** struct (the Scheme VM).

Rule:
- **Bare "engine" = the `hume-engine/` crate. Always.**
- The Scheme VM is always **"Steel engine"** in prose and **`steel`** (or
  `SteelEngine`) as the field/type — never a bare `engine` field.
- Code comments and docs must not use bare "engine" for the VM.

### D2 — `EditorState` boundary: everything a command can mutate lives here

If any command-reachable state is left on the outer `Editor`, that command needs
`&mut Editor` again and re-creates blocker #2 in miniature. The `EditorState`
struct definition is the authoritative statement of this boundary. `EngineView`
(full-fat panes, layout tree, theme) is the `view` sibling — not inside
`EditorState`, because it is also render state, but still a disjoint field of
the outer shell and reachable from sync `EditorCmd` handlers.

The outer `Editor` becomes a thin shell: `{ scripting, state, view }` plus pure
app-lifecycle plumbing that no command touches.

### D3 — Pane / view ownership: three-way split

Pane data is split across three locations, each with a clear owner:

- The **`hume-engine/` crate defines the full-fat `Pane` type** (`hume-engine/src/pane.rs`):
  `buffer_id`, `viewport`, `saved_scrolls`, `selections`, `primary_idx`, `providers`.
  Instances live in `Editor.view: EngineView` via `EngineView.panes: SlotMap<PaneId, Pane>`.
  Layout geometry lives on `EngineView.layout: LayoutTree`, not on `Pane`.
- **`EditorState` owns only editor-side per-pane bookkeeping** (`PaneView`): jump
  lists, transient search cursor, per-pane selection set. Selections and search
  cursor are nested `SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>`.
  Jump lists and transient state are keyed by `PaneId` alone. `PaneView` refers
  into `view`; does not own it.
- **`Editor.view`** is the shell sibling that bridges both: it holds the pane
  `SlotMap` and is borrowed disjointly from `state` as the 4th `&mut` at eval time.

A sync `EditorCmd` handler receives `&mut EditorState` and `&mut EngineView` as
separate arguments — two disjoint borrows, both routed through `EditorHostImpl`.

### D4 — Engine-requiring work defers by design, not limitation

`Lazy` plugin activation and hook dispatch defer because they need a fresh eval —
a new `compile_and_run_raw_program` call that cannot start while `&mut steel` is
held by the current eval. `SteelBacked` commands with an activated closure dispatch
inline (see Dispatch invariant); those without one (e.g. mid-activation) queue.

### D5 — Hooks always defer (semantic decision, LOCKED)

Hooks run *after* the command that triggers them completes — this is a semantic
guarantee of the hook model ("when X happens, then do Y"), not a consequence of the
borrow architecture. Even if re-entrancy were fully solved mechanically, hooks must
not fire mid-command. **Do not optimize hooks to fire inline.** This decision is locked.

Hooks travel via the `ConfigState::pending_events` single channel. Every producer routes
here — `queue_event`, mode change, buffer open/close/save, language-set.
`drain_events` fires the queue from one interactive choke point — `handle_input`, which
all key and mouse input (including macro replay) routes through — plus a one-time
startup drain (`lib.rs`) before the event loop.

### D6 — `EditorHost` trait: kept, re-backed by two coarse borrows

The `EditorHost` trait (defined in the `hume-scripting/` crate) is kept for two reasons:

1. The crate cycle (`hume-editor → hume-scripting → {hume-engine, hume-platform}`) is a hard wall.
   Dissolving the trait would require moving `EditorState` into a crate below
   `scripting`, re-layering most of the editor.
2. The trait preserves mockable scripting tests (`NullHost`, `MockHost`) and a
   curated API boundary: builtins call trait methods rather than poking `EditorState`
   internals.

`EditorHostImpl` holds exactly two coarse borrows (`state: &mut EditorState`,
`view: &mut EngineView`). These back the trait interface — callers (builtins) see no
change.

`ScriptingRegistries` bundles the persistent scripting registries (command ownership,
hooks, the lazy/declared-plugin tables, the activated-command table) as one field of
`ScriptingHost`. The NLL field-split is clean: `steel` vs. `registries` are two
disjoint fields, borrowed separately by `steel_and_bundle`.

### D7 — EditorCmd handler shape; TypedCommand exception

**`EditorCmd`** handlers use one shape:

```rust
fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>
```

Alias: `EditorCmdFn` type alias in `hume-editor/src/editor/registry/command.rs`.

Handlers needing no viewport access bind the view parameter as `_view`. All handlers
are synchronous and Steel-eval-safe. `run_command_sync` dispatches them, reports
errors, and returns. Hook channel is `ConfigState::pending_events` (D5).

**`TypedCommand`** (`fn(&mut Editor, Option<&str>, bool) -> Result<…>`) **deliberately
stays on `fn(&mut Editor, …)`**, for three reasons:

1. TypedCommands are not Steel-dispatchable — only callers are the `:` command line
   and tests. `&mut Editor` here never runs while the Steel engine is borrowed, so
   blocker #2 cannot occur.
2. Some handlers genuinely need shell-level fields: `:e` and `:set buffer language=`
   reach `activate_lazy_language_plugins` (needs `scripting`) and `setup_buffer_syntax`
   (needs `parse_worker`). Neither is reachable from `(&mut EditorState, &mut EngineView)`.
3. TypedCommand is the Editor-orchestration layer: it drives whole-app ops (`:w`, `:e`,
   `:bd`, `:split`, `:set language`) that legitimately span state + view + parse_worker
   + Steel together.

The correctness gate (`rg 'fn cmd_.*&mut Editor[^S]'`) is scoped to `cmd_*`
handlers. TypedCommand handlers (e.g. `write_file`) retaining `&mut Editor` are
the ratified exception.
