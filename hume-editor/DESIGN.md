> **STATUS: IMPLEMENTED** — The architecture described here shipped with the sync-dispatch
> + editor-state redesign (complete 2026-06-10, 2067 tests). Preserved as a design reference.

# HUME Editor — Command Dispatch & State Architecture

## Why this exists

Dispatching a command from Steel requires two things to be true simultaneously:
the Steel VM must be running (borrowing `&mut steel`), and the editor must be
reachable for the handler to mutate it. Two structural blockers made this
impossible without deferral:

1. **Re-entrancy.** While a script runs, the Steel `Engine` is borrowed `&mut`
   as the executor. Anything that must call Steel again (SteelBacked bodies,
   Lazy plugin activation) cannot — the engine is busy and unreachable from
   the script's context.

2. **Borrow aliasing.** The `Engine` used to live *inside* `Editor`
   (`Editor → scripting → engine`). Running the VM required carving
   `&mut engine` out of the editor, which meant the editor could only be lent
   to the script as disjoint slices — never as a whole `&mut Editor`.
   `EditorCmd` handlers take `&mut Editor`, so they could not run inside an
   eval.

Blocker #1 is intrinsic: work that needs Steel *must* run after the current
eval. **Blocker #2 was an accident of structure** — the engine being a
descendant of the editor — and this design removes it.

## The core move

Move editor state out of the `Editor` god-struct into a sibling subtree, so
the Steel VM and the editor data become **cousins that never alias**:

- **Before**: `Editor → scripting → engine`, with buffers/panes/etc. as flat
  fields on `Editor` next to `scripting`. Running the VM required
  field-splitting the whole `Editor`.
- **After**: `Editor → { scripting → steel ; state: EditorState ; view: EngineView }`.
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
│   └── registries: ScriptingRegistries  // bundles cmd_owners, hooks,
│                                        //   lazy_registry, declared_plugins
│                                        //   (disjoint from steel — NLL split)
├── state: EditorState         // document/command state — everything a command
│                              //   can mutate (see D2 for full field partition)
└── view: EngineView           // render/view state: full-fat panes, layout tree,
                               //   theme (engine/ crate type, see D3)
```

### `EditorState` — complete field partition

Everything a command can read or mutate lives here. No command-reachable state
stays on the outer `Editor` (see D2).

| Field | Notes |
|---|---|
| `buffers` | `BufferStore` |
| `mode` | `Mode` |
| `pending_keys` | `Vec<KeyEvent>` |
| `count` | `Option<usize>` |
| `wait_char` | `Option<WaitCharPending>` |
| `pending_char` | `Option<char>` |
| `registers` | `RegisterSet` |
| `kill_ring` | `KillRing` |
| `clipboard` | `SystemClipboard` — `!Send`; never passed to Steel |
| `register_prefix` | `Option<RegisterPrefix>` |
| `last_command` | `Option<Cow<'static, str>>` |
| `last_paste` | `Option<Vec<String>>` |
| `should_quit` | app-control flag |
| `minibuf` | `Option<MiniBuffer>` |
| `completion` | `Option<CompletionState>` |
| `status_msg` | `Option<String>` |
| `message_log` | `MessageLog` |
| `settings` | `EditorSettings` |
| `registry` | `CommandRegistry` — read-only at dispatch; `run_command_sync` clones the command out before running, ending the borrow before any `&mut state` access |
| `keymap` | modified by `bind-key!` |
| `last_find` | `Option<FindChar>` |
| `search` | `SearchState` |
| `focused_pane_id` | `PaneId` |
| `history` | `HistoryStore` |
| `languages` | `LanguageRegistry` — reset at `:reload-config` |
| `cwd` | `PathBuf` — updated by `:cd` |
| `force_full_redraw` | set by inline-output dispatch |
| `motion_format_scratch` | per-command scratch; reused each keypress |
| `visual_move_target_cols` | per-command scratch |
| `last_repeatable_action` | `Option<RepeatableAction>` |
| `pending_repeat` | `Option<PendingRepeat>` — per-command; set by `cmd_repeat`, consumed by `drain_pending_repeat` at tail of `handle_key` |
| `insert_session` | `Option<InsertSession>` |
| `explicit_count` | `bool` |
| `macro_recording` | `Option<(char, Vec<KeyEvent>)>` |
| `macro_pending` | `Option<MacroPending>` |
| `replay_queue` | `VecDeque<KeyEvent>` |
| `skip_macro_record` | `bool` |
| `is_replaying` | `bool` |
| `mouse_drag_anchor` | `Option<usize>` |
| `panes` | `PaneView` — consolidates the three per-pane editor maps: `SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>`, `SecondaryMap<PaneId, PaneTransient>`, `SecondaryMap<PaneId, JumpList>` |
| `pending_hooks` | `Vec<(HookId, Vec<SteelVal>)>` — unified hook channel (see D5/D7) |

Arc-wrapped render comms (stay on outer `Editor` as `Arc` clones — no `&mut` borrow needed):

| Field | Notes |
|---|---|
| `bracket_hl_data` | `Arc<RwLock<Vec<…>>>` |
| `search_hl_data` | `Arc<RwLock<Vec<…>>>` |
| `completion_view` | `Arc<RwLock<Option<CompletionView>>>` |

Shell (`Editor` outer — lifecycle only; no command access):

| Field | Notes |
|---|---|
| `scripting` | `Option<ScriptingHost>` |
| `builtin_cmd_names` | `HashSet<String>` — set at init, immutable |
| `kitty_enabled` | set at startup; read only by event loop |
| `parse_worker` | `Box<dyn ParseBackend>` — lifecycle plumbing |
| `parse_worker_disconnect_logged` | one-shot UI dedup flag |

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
carries `registries: &mut ScriptingRegistries` alongside six infrastructure borrows
(`plugin_stack`, `pending_messages`, `pending_language_regs`, `data_dir`,
`runtime_dir`, `interrupt_flag`) — all disjoint from `steel`, so the VM can run
while the bundle is live.

## Dispatch invariant

> **Defer a command iff it needs the Steel engine.**

| Command kind | Needs Steel? | Dispatch |
|---|---|---|
| Motion / Selection / Edit | no | **sync** |
| EditorCmd | no | **sync** |
| EditorCmd that fires a hook | command: no | command **sync**; handler pushes to `EditorState::pending_hooks`; `drain_hooks` fires after command body (semantic defer — see D5) |
| TypedCommand (`:` commands) | no | **sync** — but stays on `fn(&mut Editor, …)`, not Steel-dispatchable (see D7) |
| SteelBacked | yes | **defer** |
| Lazy | yes (loads Steel) | **defer** |

### `call!` — the sole dispatch primitive

`call!` (`%call!`) is the sole dispatch primitive. It runs sync when the command
kind does not need Steel, defers when it does.

**Sync path conditions (both must hold):**
1. `command_is_native` returns `Ok(true)` for the name.
2. `cmd_queue.is_empty()` — no Steel-defined command is already queued. When a
   Steel command is ahead in the queue, native commands are deferred too, to
   preserve source order.

**Arg → count/extend mapping for native commands:**

| Args | count | extend |
|---|---|---|
| `(call! "name")` | 1 | `false` |
| `(call! "name" n)` | `n` (clamped to ≥ 1) | `false` |
| `(call! "name" n #t)` | `n` (clamped to ≥ 1) | `true` |
| any other shape | `steel::stop!` | — |

For SteelBacked/Lazy (deferred), args flow through the queue unchanged.
`register_command_names` emits `(define name (lambda () (call! "name")))` wrappers —
bare `(move-left)` resolves to `(call! "move-left")` with no args (count=1, Normal).

## Decisions

### D1 — Naming hygiene: "engine" is overloaded, disambiguate everywhere

Two unrelated things are called "engine" in HUME:

- the **`engine/` workspace crate** (rendering pipeline, pane geometry), and
- the **Steel `Engine`** struct (the Scheme VM).

Rule:
- **Bare "engine" = the `engine/` crate. Always.**
- The Scheme VM is always **"Steel engine"** in prose and **`steel`** (or
  `SteelEngine`) as the field/type — never a bare `engine` field.
- Code comments and docs must not use bare "engine" for the VM.

### D2 — `EditorState` boundary: everything a command can mutate lives here

If any command-reachable state is left on the outer `Editor`, that command needs
`&mut Editor` again and re-creates blocker #2 in miniature. The field partition
table above is the authoritative statement of this boundary. `EngineView` (full-fat
panes, layout tree, theme) is the `view` sibling — not inside `EditorState`, because
it is also render state, but still a disjoint field of the outer shell and reachable
from sync `EditorCmd` handlers.

The outer `Editor` becomes a thin shell: `{ scripting, state, view }` plus pure
app-lifecycle plumbing that no command touches.

### D3 — Pane / view ownership: three-way split

Pane data is split across three locations, each with a clear owner:

- The **`engine/` crate defines the full-fat `Pane` type** (`engine/src/pane.rs`):
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

`SteelBacked` and `Lazy` commands remain queued and run after the eval. This is
correct: they need to re-enter Steel, which is only safe once the current eval
releases the `&mut steel` borrow.

### D5 — Hooks always defer (semantic decision, LOCKED)

Hooks run *after* the command that triggers them completes — this is a semantic
guarantee of the hook model ("when X happens, then do Y"), not a consequence of the
borrow architecture. Even if re-entrancy were fully solved mechanically, hooks must
not fire mid-command. **Do not optimize hooks to fire inline.** This decision is locked.

Hooks travel via the `EditorState::pending_hooks` single channel. Every producer routes
here — `fire_hook_silent`, `enqueue_mode_change`, buffer open/close/save, language-set.
`drain_hooks` fires the queue at every dispatch entry point: `execute_keymap_command`,
the `handle_key` tail, startup, and `handle_mouse`.

### D6 — `EditorHost` trait: kept, re-backed by two coarse borrows

The `EditorHost` trait (defined in the `scripting/` crate) is kept for two reasons:

1. The crate cycle (`editor → scripting → {engine, platform}`) is a hard wall.
   Dissolving the trait would require moving `EditorState` into a crate below
   `scripting`, re-layering most of the editor.
2. The trait preserves mockable scripting tests (`NullHost`, `MockHost`) and a
   curated API boundary: builtins call trait methods rather than poking `EditorState`
   internals.

`EditorHostImpl` holds exactly two coarse borrows (`state: &mut EditorState`,
`view: &mut EngineView`) instead of the previous 9 heterogeneous ad-hoc field slices.
These back the same trait interface — callers (builtins) see no change.

`ScriptingRegistries` bundles the four persistent registry fields (`cmd_owners`,
`hooks`, `lazy_registry`, `declared_plugins`) that were previously flat on
`ScriptingHost`. This makes the NLL field-split clean: `steel` vs. `registries`
are two fields of `ScriptingHost`, borrowed disjointly by `steel_and_bundle`.

### D7 — EditorCmd handler shape; TypedCommand exception

**`EditorCmd`** handlers use one shape:

```rust
fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>
```

Alias: `EditorCmdFn` type alias in `editor/src/editor/registry/command.rs`.

Handlers needing no viewport access bind the view parameter as `_view`. All handlers
are synchronous and Steel-eval-safe. `run_command_sync` dispatches them, reports
errors, and returns. Hook channel is `EditorState::pending_hooks` (D5).

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

The Phase-8 correctness gate (`rg 'fn cmd_.*&mut Editor[^S]'`) is scoped to `cmd_*`
handlers. `typed_*` retaining `&mut Editor` is the ratified exception.

## Helix convergence

`EditorState` ≈ `helix-view::Editor`; outer `Editor` ≈ Helix `Application`; Steel
dispatch via `with_mut_reference` is structurally identical to Helix's scripting
approach. One delta: Helix's scripting crate sits *above* `helix-view` so it can name
`Editor` directly — no trait needed. HUME's crate cycle inverts that dependency, which
is why `EditorHost` is kept (D6).
