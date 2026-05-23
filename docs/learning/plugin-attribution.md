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
  Steel. The `Core` owner is never active during scripting; it surfaces only as the
  string `"hume"` when `(command-plugin name)` is called for a built-in Rust command.

## Plugin names

Valid plugin name forms:
- `core:<name>` — a bundled core plugin (`core:plum`, `core:helix-surround`, …)
- `<user>/<repo>` — a third-party plugin with exactly one `/`

Name segments must be non-empty, must not be `.` or `..`, and must not contain `/`,
`\`, or NUL. Equality and hashing are case-insensitive; display and on-disk paths
use the original casing.

## `cmd_owners` and `(command-plugin name)`

Command registrations are attributed at the time `(define-command! …)` is called: the
current stack top is stored in an internal `cmd_owners` map (command name → owner string).

For lazy plugins that declare command triggers with `(declare-plugin … #:on-command …)`,
the owner is pre-seeded in `cmd_owners` immediately at declaration time — before the plugin
body runs — so `(command-plugin "cmd")` resolves correctly even when queried before the
first activation.

```scheme
(command-plugin "move-right")   ; => "hume"  (built-in Rust command)
(command-plugin "my-cmd")       ; => "alice/my-plugin" (if defined inside that plugin)
```

## Conflict detection

If two plugins try to claim the same command trigger (via `#:on-command`), the **first
declarant wins**. The duplicate is dropped with a non-fatal `Severity::Error` logged to
`:messages`; init continues.

The same first-wins rule applies to `define-command!` inside a plugin body: if a name is
already registered (by a built-in or an earlier plugin), the later registration is rejected
with `Severity::Error` and the **first** definition stays live. There is no shadowing.

## Load-once model

HUME does not support unloading or hot-reloading individual plugins. Once a plugin body
has been evaluated, its commands, bindings, and hooks remain until `:reload-config`
rebuilds everything from scratch by re-running `init.scm`. There is no prior-value
tracking and no rollback on plugin removal.
