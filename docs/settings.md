# Settings

Settings can be changed in three ways:

1. **Steel config** (`~/.config/hume/init.scm`): `(set-option! "key" "value")` — applies as a global setting at startup.
2. **Command prompt** (`:` key): `:set global key=value` or `:set buffer key=value` — takes effect immediately.
3. **Buffer-local overrides** only affect the current buffer; global settings affect all buffers that don't have an override.

---

## Global-Only Settings

These settings can only be set globally (not overridden per-buffer).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `scrolloff` | integer | `3` | Number of lines to keep visible above and below the cursor |
| `mouse-scroll-lines` | integer | `3` | Number of lines to scroll per mouse wheel tick |
| `mouse-enabled` | bool | `true` | Enable mouse support |
| `mouse-select` | bool | `false` | Allow click-to-move and click-drag selection with the mouse |
| `jump-list-capacity` | integer ≥ 1 | `100` | Maximum number of entries in the jump list |
| `jump-line-threshold` | integer | `5` | Minimum line distance for a motion to be recorded as a jump-list entry |

---

## Per-Buffer Settings

These settings can be set globally (affecting all buffers) or overridden for a single buffer.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tab-width` | integer 1–255 | `4` | Width of a tab character in columns, and number of spaces inserted when pressing Tab |
| `wrap-mode` | `none` \| `indent:N` | `indent:76` | Soft line wrapping. `none` disables wrapping; `indent:N` wraps at column N with continuation lines indented to match the wrapped line's indent level |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display. `absolute`: plain line numbers. `relative`: distance from cursor. `hybrid`: absolute on the cursor line, relative elsewhere |
| `auto-pairs-enabled` | bool | `true` | Enable auto-pairs: automatically insert closing delimiters and skip over them on close |

---

## Whitespace Rendering

Controls how invisible characters are displayed. Set sub-fields independently via `:set` or `(set-option!)`.

| Key | Scope | Values | Default | Description |
|-----|-------|--------|---------|-------------|
| `whitespace-space` | global / buffer | `none` \| `all` \| `trailing` | `none` | Render space characters |
| `whitespace-tab` | global / buffer | `none` \| `all` \| `trailing` | `none` | Render tab characters |
| `whitespace-newline` | global / buffer | `none` \| `all` \| `trailing` | `none` | Render newline characters |

- `none`: never shown
- `all`: always shown
- `trailing`: shown only on trailing whitespace (useful for catching accidental trailing spaces)

---

## Statusline Configuration (Steel only)

The statusline is configured via `(configure-statusline! left center right)` in `init.scm`. Each argument is a quoted list of element name strings.

```scheme
(configure-statusline!
  '("Mode" "Separator" "FileName" "DirtyIndicator")
  '()
  '("MacroRecording" "Selections" "Position"))
```

**Available elements (PascalCase strings):**

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode (`NORMAL`, `INSERT`, `EXTEND`) |
| `"Separator"` | Visual divider between element groups |
| `"FileName"` | Name of the current file (basename only) |
| `"DirtyIndicator"` | Shows `[+]` when the buffer has unsaved changes |
| `"Position"` | Cursor line and column |
| `"Selections"` | Number of active selections (hidden when just one) |
| `"SearchMatches"` | Current match index and total when a search is active |
| `"MiniBuf"` | Contents of the mini-buffer (search prompt, command prompt) |
| `"MacroRecording"` | Recording indicator when a macro is being captured |
| `"KittyProtocol"` | Shows `[kitty]` when the kitty keyboard protocol is active |
| `"Language"` | Detected language identifier (e.g. `rust`, `json`); empty for scratch/unknown buffers |

---

## Steel Scripting API

All settings and keymap changes available from `init.scm`:

### `(set-option! key value)`

Apply a global setting. Equivalent to `:set global key=value`. Invalid values are rejected.

```scheme
(set-option! "tab-width" "2")
(set-option! "wrap-mode" "none")
(set-option! "scrolloff" "5")
```

### `(bind-key! mode key-sequence command)`

Bind a key sequence to a named command in the given mode.

- `mode`: `"normal"`, `"extend"`, or `"insert"`
- `key-sequence`: space-separated key tokens, e.g. `"g r"`, `"ctrl-k"`, `"alt-j"`
- `command`: name of any registered command

```scheme
(bind-key! "normal" "g r" "redo")
(bind-key! "normal" "ctrl-k" "move-up")
```

### `(unbind-key! mode key-sequence)`

Remove an existing binding.

```scheme
(unbind-key! "normal" "Ctrl+c")
```

### `(bind-wait-char! mode key-sequence command)`

Bind a key sequence to a wait-char node. The next keypress after the sequence is captured and made available to the command via `(pending-char)`.

```scheme
(bind-wait-char! "normal" "m r" "helix-replace-surround")
```

### `(define-command! name doc lambda [#:repeatable #t] [#:inline-output #t])`

Register a Steel lambda as a named mappable command. The command can then be bound with `bind-key!` or invoked via `(call! ...)`.

When triggered from a key binding, the lambda receives `count` and `extend` as leading arguments based on how many parameters it declares:

| Lambda signature | Receives |
|---|---|
| `(lambda ())` | nothing |
| `(lambda (count))` | the repeat count (integer ≥ 1) |
| `(lambda (count extend))` | count and extend flag (`#t`/`#f`) |

The lambda decides how to act on these; it forwards them explicitly via `(call! name count extend)`. Ctrl+key always delivers `extend = #t`, enabling one-shot extend on any key-bound command.

```scheme
(define-command! "my-command" "Description shown in command help."
  (lambda ()
    (call! "move-right")
    (call! "delete")))

;; With count and extend support:
(define-command! "step-right" "Move right N times."
  (lambda (count extend)
    (call! "move-right" count extend)))
```

**`#:repeatable #t`** — opt in to dot-repeat (`.`). Use only for self-contained buffer edits that make sense to replay at a new cursor position. The whole lambda body re-executes on replay.

```scheme
(define-command! "delete-and-remember" "Delete selection; repeatable."
  (lambda () (call! "delete"))
  #:repeatable #t)
```

**`#:inline-output #t`** — bracket dispatch with a terminal exit so subprocess output streams live to the terminal instead of the message bar. Use for shell-outs (formatters, linters, installers). The editor returns to its normal screen after a keypress.

```scheme
;; #:inline-output is typically used in plugin code that shells out.
;; See runtime/plugins/core/plum/grammars.scm for real-world examples.
(define-command! "my-build-command" "Run a build step inline."
  (lambda ()
    ; body calls plugin builtins that run subprocesses
    (my-plugin/run-build))
  #:inline-output #t)
```

`#:repeatable` and `#:inline-output` are mutually exclusive — shell-out commands must not participate in dot-repeat.

Command names must be unique; duplicate registrations are rejected.

### `(call! command-name args…)`

Queue a named command for execution with optional positional args.

```scheme
(call! "move-right")
(call! "replace-char" my-char)
(call! (string-append "surround-" suffix))
```

### `(pending-char)`

Returns the wait-char argument as a single-character string. Empty string if no wait-char is pending. Only meaningful inside a command registered with `bind-wait-char!`.

```scheme
(let ((ch (pending-char)))
  (call! (string-append "find-" ch)))
```

### `(configure-statusline! left center right)`

Configure all three statusline sections in one call. See [Statusline Configuration](#statusline-configuration-steel-only) above.
