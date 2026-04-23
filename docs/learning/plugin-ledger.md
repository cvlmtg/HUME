# The Plugin Ledger: Attribution and Clean Unload

Plugins in HUME are first-class — they can bind keys, change settings, and
register commands. They can also be unloaded and reloaded at runtime. For
that to work correctly, the editor needs to know *who* changed what, *what
the previous value was*, and *how to get back to it* when a plugin leaves.
The **ledger** is the bookkeeping system that makes this possible.

## The problem

When two plugins both rebind the same key, whichever ran second is the
live owner. If the first one is later unloaded, the key should still belong
to the second — the first plugin's value is simply gone, but the second's
remains. If the second one is then unloaded too, the key should return to
whatever it was before either plugin ran.

Without an explicit record of this chain, unloading becomes destructive: the
simplest approach (store the previous value, restore it on unload) fails as
soon as two plugins overlap. Unloading the earlier one would clobber the
later one's value.

The ledger solves this through a combination of **attribution** (who owns
what, right now) and **prior-chaining** (what each plugin displaced, so
the chain can be reconstructed when any plugin leaves).

## Attribution: the plugin stack

Every mutation that flows through the scripting layer is attributed to a
current **owner** at the moment the mutation happens. A stack tracks the
current execution context:

- When `init.scm` runs at startup and no plugin body is executing, the
  stack is empty — attributions go to the **user** owner.
- When `(load-plugin …)` loads a plugin, the plugin's identity is pushed
  onto the stack. Any mutations inside that body are attributed to that
  plugin. Nested `(load-plugin …)` calls push further, so the innermost
  plugin gets credit.
- When the plugin body finishes, the identity is popped.

This means attribution is automatic — plugins don't declare ownership
explicitly, they simply run, and whatever they mutate gets their name
attached to it.

## Owners

Three types of owner exist:

- **Core** — the built-in default. Core is never the active attribution
  (HUME's initial state isn't loaded via the scripting layer), but it
  appears as the *prior* owner when a plugin mutates something that was
  previously at its factory default.
- **User** — `init.scm` running outside any plugin body. These are the
  user's personal customisations.
- **Plugin(id)** — a specific plugin, identified by a case-insensitive
  `user/repo` or `core:name` string. The casing is preserved for display
  and file paths; equality and hashing are case-insensitive, so
  `ALICE/TOOL` and `alice/tool` are the same plugin.

## The ledger entry

When a mutation is attributed and recorded, one entry is written:

- **Key** — a stable string that identifies what changed. A space in the
  key indicates a keymap binding (e.g. `"normal f"` for the `f` key in
  Normal mode); no space indicates a setting (e.g. `"tab-width"`).
- **Prior value** — the serialised form of the value that was live
  *before* this mutation. The new value is not stored here — it lives in
  the real registry, keymap, or settings, so there's no duplication.
- **Prior owner** — who owned the binding before this mutation.
- **Prior extend flag** — for keymap entries only: whether the old binding
  had extend semantics. Settings entries always carry `false` here.

The key insight: the entry stores only what's needed to *undo* the change,
not the change itself.

## Deduplication within a plugin

If a plugin mutates the same key more than once, only the **first** mutation
is recorded. The first entry already captures "what existed before this plugin
touched this key" — that's the only information needed to restore the state
when the plugin unloads. Subsequent mutations by the same plugin to the same
key are silently ignored by the ledger (the live state is still updated; only
the ledger record is skipped).

## The ledger stack

One ledger exists per plugin, and ledgers are ordered by activation time —
oldest first. The ledger stack is the full list of these ledgers.

To find out who currently owns a key, the stack is scanned newest-to-oldest
(right-to-left): the first ledger that contains an entry for that key belongs
to the current live owner. If no ledger mentions the key, ownership falls
back to Core.

## Unload: the rewrite-prior algorithm

When a plugin is unloaded, its ledger is removed from the stack and each
entry is processed:

**Case 1 — a later plugin also touched this key.**
The unloading plugin is in the middle of the chain: some earlier state →
this plugin's value → a later plugin's current value. The later plugin is
the live owner; its value must stay. But its ledger entry currently says
"the value before me was this plugin's value" — after the unloading plugin
disappears, that's no longer accurate. The fix: rewrite the later plugin's
entry so its prior fields point directly to what existed *before the
unloading plugin* — effectively splicing the unloading plugin out of the
chain. The live value is untouched.

**Case 2 — no later plugin touched this key.**
The unloading plugin is the live owner. Its prior value and prior owner are
returned to the caller, which restores them to the live state.

After this operation, the chain is consistent: every remaining ledger entry
has accurate prior fields, and any future unload will reconstruct the correct
baseline.

### An example

Three plugins load in order: X, Y, Z — all bind the same key `f`.

| After loading | Live value | Ledger chain (oldest → newest) |
|---------------|------------|-------------------------------|
| X loads | X's value | X: prior = Core's value |
| Y loads | Y's value | X: prior = Core's value → Y: prior = X's value |
| Z loads | Z's value | X: prior = Core → Y: prior = X → Z: prior = Y |

Unload X (middle of chain):
- `f` is still live under Z. Y's entry currently says "prior = X's value".
  Rewrite Y's entry: prior = Core's value.
- Result: Y: prior = Core → Z: prior = Y's value. Live value = Z's value. ✓

Now unload Y (still in the middle):
- `f` is still live under Z. Z's entry currently says "prior = Y's value".
  Rewrite Z's entry: prior = Core's value.
- Result: Z: prior = Core's value. Live value = Z's value. ✓

Now unload Z (live owner):
- No later plugin. Z's prior (Core's value) is returned and restored.
- Result: `f` is back to Core's original value. ✓

The order didn't matter. The chain always reconstructs cleanly.

## Why this matters

The ledger makes three guarantees:

1. **Order independence.** Plugins can be unloaded in any order. The rewrite
   algorithm keeps the chain consistent regardless of which plugin leaves first.
2. **Reload correctness.** Reloading a plugin is just unload followed by load.
   Because unload always produces a consistent chain, the reload starts from
   the right baseline.
3. **Observability.** At any point, you can ask who currently owns a key.
   The `(command-plugin name)` builtin surfaces this; the editor uses it
   internally for conflict detection.

## The contract for mutating builtins

Any Steel builtin that modifies shared editor state must participate in the
ledger. The required pattern is:

1. Read the current live value and the current owner.
2. Record them in the ledger under a stable key before writing anything.
3. Write the new value to the live state.

If a builtin skips step 2 — writing directly to live state without a ledger
record — the mutation is invisible to the attribution system. The owner-of
query returns `Core` even though a plugin changed it. On unload, there's no
entry to return, so the prior value is never restored. The plugin leaves a
permanent mark on the editor even after it's gone.

This contract applies to every piece of editor state that plugins can
meaningfully customise: settings, keymaps, and command registrations.
