# Configuration

Options, key bindings, the statusline, plugins, and language servers are all configured in one language, in one file. There's no separate config format to learn: `init.scm` is a Scheme program, so anything you can compute you can configure.

HUME can be configured two ways: the `:set` command for runtime changes during a session, or an `init.scm` file for persistent configuration loaded at startup.

HUME reads persistent configuration from:

- **macOS / Linux:** `$XDG_CONFIG_HOME/hume/init.scm` (defaults to `~/.config/hume/init.scm`)
- **Windows:** `%APPDATA%\hume\init.scm`

Pass `--config <FILE>` (see [Command-line Flags](cli.md)) to load a different file instead. [Themes](themes.md) and the data directory still resolve from the standard directories above. `:reload-config` re-runs whichever file the session started from.

## Example init.scm

HUME ships a starter config as `init.scm.example` in its runtime directory: `share/hume/init.scm.example` in the macOS/Linux release archive, `runtime/init.scm.example` on Windows or in a source checkout (see [File locations](#file-locations)). Copy it to `~/.config/hume/init.scm` (`%APPDATA%\hume\init.scm` on Windows) and edit it:

<<< @/../../runtime/init.scm.example{scheme}

What the plugins it loads give you:

- `core:stdlib`: helpers the other plugins need. Keep it first.
- `core:pickers`: fuzzy pickers for files (`z f`), open buffers (`z b`) and changed files (`z m`). See [Fuzzy Finder](pickers.md).
- `core:lsp`: language-server features such as hover, go-to-definition, diagnostics and completion. See [Language Servers](lsp.md).
- `core:lsp-install`: `:lsp-install` downloads a language server for you.
- `core:plum`: installs the grammars syntax highlighting needs, and third-party plugins and themes. See [Syntax Highlighting](syntax-highlighting.md).
- `core:git-diff`: marks changed lines in the gutter as you edit.
- `core:undotree`: `:undotree` shows the undo history as a graph.

The commented-out lines are optional; [Core Plugins](core-plugins.md) describes each one.

Before your `init.scm` runs, HUME loads its own prelude (which defines `bind-keys!`, `define-language!`, `configure-language!` and friends) and its built-in language definitions, so those are always available to you.

## Errors and reloading

If the file does not exist, HUME starts with defaults. If it fails partway through, the error is reported in `:messages` and everything up to that point stays applied, so a broken line late in the file leaves you half-configured rather than back at defaults. Fix it and run `:reload-config` to re-run the file without restarting.

A plugin that fails to load doesn't stop the rest: HUME reports the error, naming the plugin and pointing at the file and line the problem is in, and keeps going with the rest of `init.scm`. See [Plugins](plugins.md) for how to recover.

::: details What `:reload-config` resets
`:reload-config` starts from a clean slate: every option, key binding, hook, command, and plugin goes back to its default first, then the file runs again, so removing a line from `init.scm` and reloading does undo what it did. Any `:set global`/`:set buffer`/[`:theme`](themes.md) change you made during the session is discarded too, not just what `init.scm` set, with two exceptions: a pane-scoped `:set pane` override, which stays as you left it (panes are editing state, not config), and an explicit `:set buffer language=<name>`, which is restored after the reload rather than discarded: detection can't reconstruct it on its own (that's exactly why you had to set it explicitly), so losing it on every reload would be more surprising than keeping it. If the file fails partway through this time, you're left with defaults plus whatever ran before the error, same as at startup.

Buffers stay open and language servers stay attached across a reload: it behaves as if every open file were closed and reopened. Completion triggers, inline diagnostics, and any per-language setup your config applies (e.g. from `on-language-set`) come back too, without restarting the language server or losing your place in the file.

A file named with `--config` must exist: if it is missing at startup or when you reload, HUME reports an error instead of falling back to defaults.
:::

## Setting options

There are two ways to set an option: the `:set` command for runtime changes, or `set-option!` in `init.scm` for persistent defaults. [Settings](settings.md) lists every option.

### `:set` command

The `:set` command takes a scope and a `key=value` pair. The scope is required:

```
:set global <option>=<value>     set the global default
:set buffer <option>=<value>     override for the current buffer only (takes precedence over global)
:set pane <option>=<value>       override for the current pane only (view-scoped settings)
```

For a buffer option (see [Buffer options](settings.md#buffer-options)), `:set global` takes effect immediately in every buffer that has no override of its own, not just newly opened ones. `wrap-mode` additionally accepts `:set pane`, which pins one pane's wrap style above both the buffer and global setting (see [Text wrap](settings.md#text-wrap)).

Changes apply to the current session and are not persisted. For persistent configuration, use `init.scm` (below).

### From `init.scm`

```scheme
(set-option! "option-name" value)
```

Sets the global default. The value is a string, symbol, boolean, or integer, or for a word-list option such as `shell`, a list of strings. An option that takes one of a fixed set of names also accepts, and `get-option` returns, the name as a symbol: `'soft`. A word-list option's `get-option` value is a list of strings: `'("sh" "-c")`. Callable from `init.scm`, a plugin body, or a command/hook body: anywhere Scheme code runs.

```scheme
(set-option! "gutter-line-numbers" 'absolute)
(set-option! "indent-width" 2)
```

`init.scm` is a real Scheme program, not a flat list of settings, so you can react to what's being opened rather than only set fixed defaults. The most common case is configuring an option per file type: register an `on-language-set` handler and call `(set-buffer-option! pane "option" value)` to override just that buffer (see [Hooks](writing-plugins.md#hooks) for the full hook API):

```scheme
; 2-space indentation for Markdown buffers
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "markdown")
      (set-buffer-option! pane "indent-width" 2))))

; word-wrap Markdown buffers, leave source code unwrapped
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "markdown")
      (set-buffer-option! pane "wrap-mode" "word"))))

; treat '-' as a word character in CSS-family buffers, so `w`/`b`/`mm`/`*`
; see "foo-bar" as one word instead of three
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (member lang '("css" "scss" "less"))
      (set-buffer-option! pane "word-chars" "-"))))

; keys for Markdown buffers only (see "Keys for one buffer" below)
(bind-key! 'normal "G H" "markdown-heading" #:keymap 'markdown-keys)
(register-hook! 'on-language-set
  (lambda (pane lang)
    (if (equal? lang "markdown")
        (attach-keymap! pane 'markdown-keys)
        (detach-keymap! pane 'markdown-keys))))
```

`markdown-heading` is the command from [Example: Markdown headings](writing-plugins.md#example-markdown-headings).

## Key bindings

```scheme
(bind-key! 'normal "ctrl-j" "move-down")
(bind-key! 'normal "space b" "goto-alternate-buffer")
(unbind-key! 'normal "ctrl-j")
```

`bind-key!` takes an editor command's name (the same names in [Builtin Commands](builtin-commands.md)), never a typed command; there's no way to bind one of those to a key.

`bind-key!`: binds a key in the given mode (`'normal`, `'insert`, `'extend`, `'command`). An `'insert` or `'command` binding is a single key.
`unbind-key!`: removes a binding.
`bind-key-extend!`: binds a `'normal` key so it always extends the selection, as the one-shot `Ctrl-` motions do.

To set several bindings at once, use the plural forms:

```scheme
(bind-keys! 'normal
  ("ctrl-h" "select-prev-word")
  ("ctrl-l" "select-next-word"))

(bind-keys-extend! 'normal
  ("ctrl-n" "select-line")
  ("ctrl-y" "select-line-backward"))

(unbind-keys! 'normal "ctrl-j" "ctrl-k")
```

`bind-keys!` batches `bind-key!`, `bind-keys-extend!` batches `bind-key-extend!`, and `unbind-keys!` batches `unbind-key!`. Each takes one or more `(key cmd)` pairs (or, for `unbind-keys!`, one or more bare keys) instead of a single one.

### Keys for one buffer

Add `#:keymap` and a name to put a binding in a keymap of its own instead of the global bindings. The first binding into a name creates that keymap. A keymap does nothing until you attach it to a buffer with `attach-keymap!`, and `detach-keymap!` turns it off again. This makes `Ctrl-j` and `Ctrl-k` jump between paragraphs in Markdown buffers only:

```scheme
(bind-keys! 'normal #:keymap 'markdown-keys
  ("ctrl-j" "goto-next-paragraph")
  ("ctrl-k" "goto-prev-paragraph"))

(register-hook! 'on-language-set
  (lambda (pane lang)
    (if (equal? lang "markdown")
        (attach-keymap! pane 'markdown-keys)
        (detach-keymap! pane 'markdown-keys))))
```

`bind-key!`, `bind-key-extend!` and `unbind-key!` take `#:keymap` after their other arguments, as in `(unbind-key! 'normal "ctrl-k" #:keymap 'markdown-keys)`, and the plural forms take it right after the mode. The hook detaches the keymap when a buffer's language changes away from Markdown. Detaching removes only that keymap, so the other keymaps attached to the buffer, a plugin's for instance, keep working.

In a buffer, keys are looked up in its attached keymaps, the most recently attached first, then in the global bindings. Attaching a keymap that is already attached keeps its place. The first keymap that has the keys, as a binding or as the start of a longer one, decides. So a key bound in a keymap hides the global bindings that start with it (binding `g` hides the global `g g`), and the start of a longer binding hides a global binding of the same keys. Bindings that only share their first keys combine: with `G H` in a keymap and the global `G G`, both work. In Extend mode, each keymap's `'extend` bindings are tried before its `'normal` ones, and both before the next keymap, so a keymap's `'normal` binding wins over a global `'extend` binding of the same keys.

Attaching or detaching a name nothing has been bound into yet does nothing and logs a warning in `:messages`. Bind first: attaching before the first binding has no effect, even once the keymap exists.

Global bindings belong in `init.scm` or a plugin body, but bindings with `#:keymap` and attaching work from commands and hooks too, so you can build and attach a keymap while you edit. Keymap names are shared by every plugin, so a plugin's keymap names start with the plugin's own, like `'my-plugin/keys`. `:reload-config` clears every keymap along with the other bindings, and a hook like the one above attaches its keymap again when it runs.

### Falling back to a key's normal Insert-mode behaviour

Binding an Insert-mode key replaces its behaviour outright. `insert-key!` (see [Plugin API](plugin-api.md#editing-navigation), which also has a worked example) lets a command decide, per keypress, whether to override the key or fall back to what it would otherwise do.

`.` re-runs an Insert-mode key's binding, so it decides again at the new cursor, and a side effect such as saving the file happens again too. If the binding's command is repeatable (a built-in like `replace`, or a plugin command marked `#:repeatable`) and asks you for something (a picker, a `prompt!`, a character), `.` reuses your answer; if it isn't, the change ends at that key. A completion is replayed as the text it left at the cursor, and a binding that reaches completion only on replay reports an error. See [Dot-repeat](writing-plugins.md#dot-repeat).

### Keys in the command line

A `'command` binding applies in the command mode prompt and in prompts plugins open, not in search or sift. It replaces the prompt's own use of that key, except that an open candidate list keeps the keys it picks candidates with. Running a command this way doesn't change what `.` repeats. `Ctrl-r`, which inserts a register or the current file's path, is bound this way by default (see [Inserting a register while typing](copy-and-paste.md#inserting-a-register-while-typing)). For example, `Ctrl-g` inserting the working directory:

```scheme
(define-command! "insert-cwd" "Insert the working directory."
  (lambda (pane) (paste-text! pane (cwd))))
(bind-key! 'command "ctrl-g" "insert-cwd")
```

### Binding a command that reads a character

A command that needs a character typed after its key (find/till motions, `replace`, `surround-add`) waits for it on its own, so it is bound like any other command:

```scheme
(bind-key! 'normal "m s" "surround-add")
```

See [Reading characters](writing-plugins.md#reading-characters) for writing a command of your own that reads characters.

### Key-string grammar

A key string is a **whitespace-separated** list of tokens. Each token is `[modifier-]*key` where the modifier separator is a **dash** `-` (not `+`):

| Component | Values |
|-----------|--------|
| Modifiers | `ctrl-`/`c-`, `shift-`/`s-`, `alt-`/`a-` (case-insensitive, repeatable, any order, short and long forms may be mixed) |
| Named keys | `space`, `tab`, `enter` / `return` / `cr` / `ret`, `esc` / `escape`, `lt` (`<`), `backspace` / `bs`, `delete` / `del`, `insert` / `ins`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `f1`–`f12` |
| Single char | Any single Unicode character; case is preserved (`"G"` and `"g"` are distinct); an uppercase letter and an explicit `shift-` are equivalent (`"ctrl-X"` and `"ctrl-shift-x"` bind the same key) |

Multi-key sequences are space-separated: `"g e"`, `"m i w"`, `"ctrl-p h"`. Examples: `"ctrl-j"`, `"shift-tab"` (becomes `BackTab`), `"ctrl-shift-left"`, `"g e"`.

::: tip Binding the backslash key
In Scheme string literals `\` is the escape character, so to bind the `\` key write it escaped: `"\\"`, not `"\"`:

```scheme
(bind-key! 'normal "\\" "my-command")
```

The same applies to the double quote: bind `"` as `"\""`.
:::

## File locations

HUME resolves its directories per OS:

| Path | macOS / Linux | Windows |
|------|---------------|---------|
| Config dir (`init.scm`, hand-authored `themes/`) | `$XDG_CONFIG_HOME/hume/` (default `~/.config/hume/`) | `%APPDATA%\hume\` |
| Data dir (plugin clones, tree-sitter grammars, installed `themes/`) | `$XDG_DATA_HOME/hume/` (default `~/.local/share/hume/`) | `%LOCALAPPDATA%\hume\` (fallback `%APPDATA%\hume\`) |
| Runtime dir (bundled `runtime/`: `tutor.rst`, `themes/`, `scheme/`, `init.scm.example`, core plugins) | see below | see below |

`--config <FILE>` overrides only which file HUME evaluates as `init.scm`; user `themes/` and the data dir still resolve from the config dir above regardless.

HUME looks for its runtime directory in this order, taking the first that exists:

1. `$HUME_RUNTIME`, if set
2. `../share/hume/` relative to the binary (macOS and Linux only; this is the layout you get from the release archive)
3. `runtime/` next to the binary (the Windows archive layout)
4. `runtime/` in the current working directory (handy when running from a source checkout)

A relative value in `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`, `$HUME_RUNTIME` or `$TMPDIR` is resolved against the directory HUME was started in, once, at startup. A later `:cd` does not change where these point.

Notable subpaths inside the data dir: `data/plugins/` (PLUM-managed plugin clones), `data/grammars/` and `data/grammars/sources/` (compiled and source tree-sitter grammars), `data/themes/` (installed third-party themes).

::: warning Plugins are trusted code
Plugins run with the same privileges as HUME itself: they can read and write any file your user account can, and run other programs. There is no sandbox. Install third-party plugins only from sources you trust.
:::

::: info
On macOS HUME follows the XDG convention (`~/.config/hume/`, `~/.local/share/hume/`) rather than `~/Library/Application Support/`.
:::