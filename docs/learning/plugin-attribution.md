# Plugin Attribution: Who Owns What

When multiple plugins and the user's own `init.scm` all register commands, HUME needs to
answer "who owns this command?" — both for conflict detection at declaration time and for
the `(command-plugin name)` query at runtime.

## The plugin stack

Every Steel eval runs in the context of a **plugin stack**. When `init.scm` runs at startup
and no plugin body is executing, the stack is empty — operations are attributed to the
**user** owner. When a plugin body is activated, the plugin's identity is pushed onto the
stack; any commands defined inside that body are credited to it. Nested activations (a
plugin body that triggers loading of a second plugin) push further onto the stack; the
innermost plugin gets credit. When the body finishes, its identity is popped.

Attribution is automatic — plugins don't declare ownership explicitly, they simply run.

## Owners

Three types of owner exist:

- **User** — `init.scm` running outside any plugin body. These are the user's personal
  customisations.
- **Plugin(id)** — a specific plugin, identified by a case-insensitive `user/repo` or
  `core:name` string. The casing is preserved for display and file paths; equality and
  hashing are case-insensitive, so `ALICE/TOOL` and `alice/tool` are the same plugin.
- **Core** — the built-in fallback for Rust commands that were never registered through
  Steel. The core owner is never active during scripting; it surfaces only as the
  string `"hume"` when `(command-plugin name)` is called for a built-in Rust command.

## Plugin names

Valid plugin name forms:
- `core:<name>` — a bundled core plugin (`core:plum`, `core:helix-surround`, …)
- `<user>/<repo>` — a third-party plugin with exactly one `/`

Name segments must be non-empty, must not be `.` or `..`, and must not contain `/`,
`\`, or NUL. Equality and hashing are case-insensitive; display and on-disk paths
use the original casing.

## Owner attribution and `(command-plugin name)`

Command registrations are attributed at the time `(define-command! …)` is called: the
current stack top is stored in an internal owner map (command name → owner string).

For lazy plugins that declare commands in their manifest with `(declare-plugin … #:commands …)`,
the owner is pre-seeded immediately at declaration time — before the plugin body runs —
so `(command-plugin "cmd")` resolves correctly even when queried before the first
activation.

```scheme
(command-plugin "move-right")   ; => "hume"  (built-in Rust command)
(command-plugin "my-cmd")       ; => "alice/my-plugin" (if defined inside that plugin)
```

## Conflict detection

Conflicts fall into two categories with different severity:

**Soft conflict — manifest vs manifest.** If two plugins both list the same command name
in their `#:commands`, the first declarant wins. The duplicate is silently dropped (a
non-fatal error is logged to `:messages`) and `init.scm` continues loading.

**Hard conflict — body vs existing.** When a plugin body calls `define-command!` for a
name that is already registered — whether by a built-in, an earlier plugin body, or
another lazy plugin's `#:commands` manifest — it raises a Steel error that aborts the
plugin body. The plugin is marked as failed, any commands it partially registered are
rolled back, and the originally registered command stays live. There is no shadowing; the
first registration wins unconditionally.

The reverse is also caught: if `declare-plugin` lists a `#:commands` entry that an eager
command already occupies, that entry is dropped with a non-fatal error. If it was the
sole activation entry, the declaration itself hard-errors (a plugin with no remaining
activation entries can never load).

## Load-once model

HUME does not support unloading or hot-reloading individual plugins. Once a plugin body
has been evaluated, its commands, bindings, and hooks remain until `:reload-config`
rebuilds everything from scratch by re-running `init.scm`. There is no prior-value
tracking and no rollback on plugin removal.
