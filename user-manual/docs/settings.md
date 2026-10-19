# Settings

Every option HUME has, how text wrap works, the statusline elements, and how a file's language is detected. See [Setting options](configuration.md#setting-options) for how to change them.

## Global options

Set with `:set global <option>=<value>` or `(set-option! "option" value)`. All of these are global-only.

For a `bool` option, `:set` accepts `true`/`false`, `on`/`off`, `yes`/`no`, or `1`/`0`; from Scheme, pass `#t`/`#f`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `theme` | string | `sand` | Active color [theme](themes.md) name. Empty selects the built-in fallback theme |
| `cursor-shape-insert` | `block`/`bar`/`underline` | `bar` | Cursor shape while in Insert mode, applied to every cursor when multiple are active |
| `scroll-margin` | integer | `3` | Minimum lines kept above/below cursor |
| `object-jump-align` | `top`/`center`/`off` | `center` | Where the view lands after jumping forward to a paragraph or structural object (`}`, `g f`, …); `top` is still subject to `scroll-margin` |
| `mouse` | bool | `#t` | Enable mouse support |
| `mouse-scroll-lines` | integer | `3` | Lines per mouse scroll tick |
| `mouse-select` | bool | `#f` | Mouse drag creates selections |
| `jump-list-capacity` | integer ≥ 1 | `100` | Max jump list entries |
| `jump-line-threshold` | integer | `5` | Line distance to record a jump |
| `history-capacity` | integer ≥ 1 | `100` | Max entries per `:`/`/`/`?` prompt history |
| `undo-levels` | integer | `0` | Max undo states kept per buffer; `0` means unlimited. Once the limit is reached, the oldest states (including whole abandoned branches) are dropped as new edits are made |
| `steel-init-budget-ms` | integer ≥ 1 | `10000` | Max evaluation time (ms) for `init.scm` and each plugin activation. Setting it *from* `init.scm` has no effect on that same run: the budget is read before each file/plugin evaluation starts, so a change only takes effect for evaluations after it, i.e. the next plugin activation or the next session |
| `steel-command-budget-ms` | integer ≥ 1 | `1000` | Max Steel command evaluation time (ms) |
| `command-completion` | `common-prefix` \| `first-candidate` | `common-prefix` | What the first `Tab` on the `:` line does when several candidates match: `common-prefix` completes the longest text they share and picks nothing, so the next `Tab` picks the first candidate; `first-candidate` fills in the first candidate at once |
| `popup-border` | bool | `#t` | Show borders on popups, menus, pickers and the bottom drawer |
| `completion-docs` | bool | `#t` | Show the highlighted completion's details and documentation in a box beside the completion menu. When off, or when there is no room for the box beside the menu, the details show in the menu instead. When a language server's completion arrives without them, HUME asks the server for them as you highlight it |
| `syntax-highlight-max-bytes` | integer ≥ 1 | `1048576` | Max bytes for syntax highlighting |
| `pane-dividers` | bool | `#t` | Draw a 1-cell divider between sibling panes |
| `statusline` | `left` \| `center` \| `right` | see [Statusline](#statusline) | Three `\|`-separated sections of comma-separated element names. Set it with `configure-statusline!` |
| `statusline.mode-colors` | bool | `#t` | Tint the whole statusline with the current mode's color; off shows the [theme](themes.md)'s base `ui.statusline` color in every mode |
| `tabline` | `always`/`never`/`dynamic` | `dynamic` | When to show the tab bar: always, never, or only once more than one tab is open |
| `viewport-debounce-ms` | integer ≥ 1 | `150` | Delay after scrolling stops before plugins are told which lines are visible, e.g. to request inlay hints for them |
| `before-save-timeout-ms` | integer ≥ 1 | `1000` | How long a save waits for `on-before-buffer-save` handlers before writing the file anyway |

The `lsp.*` options below configure HUME's built-in language-server support, which `core:lsp` and any other plugin that talks to a language server build on. `core:lsp`'s own options are passed when you load it. See [Language Servers](lsp.md#settings) for those, and for setup and commands.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `lsp.diagnostics-severity-floor` | `error` \| `warning` \| `info` \| `hint` | `hint` | Lowest diagnostic severity to display |
| `lsp.diagnostics-on-insert-line` | bool | `#f` | Keep the diagnostic underline and end-of-line summary on the line you are typing on in Insert mode. Off hides them there |
| `lsp.request-timeout-ms` | integer ≥ 1 | `10000` | How long to wait for a language-server reply |
| `lsp.diagnostics-pull-debounce-ms` | integer ≥ 1 | `250` | Delay after you stop typing before asking a server that only reports on request for its diagnostics |

## Buffer options

These options have a global default that every buffer without its own override resolves to (including buffers already open when you change it, not just ones opened afterward) and a per-buffer override that takes precedence when present. Set the global default with `:set global <option>=<value>` or `(set-option! "option" value)`; override the current buffer with `:set buffer <option>=<value>`, or from a script with `(set-buffer-option! pane "option" value)`. See [From `init.scm`](configuration.md#from-init-scm) for setting per-language overrides from the `on-language-set` hook.

`language` and `line-ending` are exceptions, they have no global default. `language` is auto-detected per buffer, and setting it overrides the detection for that buffer. `:set buffer language=` with nothing after the `=` (or `""` from a script) clears it. `line-ending` is read from the file when it opens (new buffers use `lf`) and can only be set per buffer.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `wrap-mode` | `none` \| `soft[:N]` \| `word[:N]` \| `indent[:N]` | `indent` | Line wrapping. `N` is the wrap column (`0` or omitted = pane content width). Also accepts `:set pane wrap-mode=<value>` to pin one pane above the buffer/global setting; see [Text wrap](#text-wrap) |
| `tab-width` | integer, 1–255 | `4` | Spaces per indent level |
| `indent-guides` | bool | `#t` | Draw vertical guides at each indentation level |
| `tab-style` | `hard` \| `soft` | `hard` | What `Tab` inserts: `hard` = literal `\t`; `soft` = spaces to next tab stop |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display in the gutter |
| `auto-pairs` | bool | `#t` | Enable auto-pair insertion |
| `smart-indent` | bool | `#t` | Enter, `o` and `O` indent a new line from the language's syntax, or copy the current line's indent when the language has no grammar, and a closing token such as `}` typed on that new line moves it back to its block's indent. `#f`: new lines start at the left margin, and nothing is moved back as you type |
| `select-inserted-text` | bool | `#t` | Leaving Insert mode keeps the text you typed selected, instead of leaving a plain cursor |
| `prefer-line-comments` | bool | `#f` | `G G` comments a selection spanning several lines with a line comment on each line, even when the language also has block comments |
| `word-selects-whitespace` | `off` \| `before` \| `after` | `before` | Which whitespace `w`/`W`/`b`/`B`, `mm`/`MM` and `maw`/`maW` add to the word they select. `before`: the whitespace before the word, or the whitespace after it when none precedes it (the first word of a line, or a word directly after punctuation). `after`: the whitespace after the word, up to the end of the line, or the whitespace before it when none follows (the last word of a line, or a word directly before punctuation). Either way a line's indentation is never selected. `off`: the bare word, and `maw`/`maW` select the same as `miw`/`miW` |
| `word-chars` | string | `""` | Extra characters counted as part of a word, e.g. `-` makes `foo-bar` one word instead of three. Set it per language from an `on-language-set` hook (see [From `init.scm`](configuration.md#from-init-scm)). Everything it affects is listed under this table |
| `signcolumn` | `always[:N]` \| `auto[:N]` | `always` | Gutter column for plugin-supplied signs (diagnostics, git changes, etc). Bare `always`/`auto` sizes the column to one column per registered sign source: a source claims its column the moment the plugin registers it, so the width doesn't change as individual signs come and go; `:N` pins it to exactly N columns (1–127) instead, hiding whichever lower-priority sources don't fit. `auto` additionally collapses to zero width when no signs are visible |
| `auto-read` | bool | `#t` | Prompt to reload when the current buffer's file changes on disk. `#f` only warns; reload manually with `:e!` |
| `whitespace-space` | `none` \| `all` \| `trailing` | `none` | When to render space indicators. Also reveals invisible Unicode spaces (non-breaking and ideographic) with a distinct `⍽` marker |
| `whitespace-tab` | `none` \| `all` \| `trailing` | `none` | When to render tab indicators |
| `whitespace-newline` | `none` \| `all` | `none` | When to render newline indicators |
| `language` | string | *(auto-detected)* | The buffer's language: picks its syntax highlighting, language server, and language-specific plugins |
| `line-ending` | `lf` \| `crlf` | *(from the file)* | The line ending a save writes. Changing it marks the buffer modified but is not an undo step, so `u` does not revert it; set it back to the file's own ending and the buffer reads unmodified again |

::: details What `word-chars` affects
The characters count as part of a word for `w`/`b`, `mm`, `miw`/`maw`, `select-word-nearest-on-line`, `Ctrl-w`, and `*`. They also affect quote auto-pairing (`'`, `"`, `` ` ``, not bracket pairs), the identifier under the cursor used by plugin commands (e.g. rename), where a completion without a server-supplied replace range starts, whether a closing word typed on a freshly indented line is finished, and whether `Ctrl-a`/`Ctrl-s` read a `-` as a minus sign. They don't affect `W`/`B`/`MM`, which already treat punctuation as part of a WORD. Whitespace and newline characters are rejected.
:::

Characters the terminal cannot show (control characters, and invisible ones such as a zero-width space or a bidirectional override) are always displayed as their codepoint (`<200b>`), styled with the [theme](themes.md#scopes-hume-adds)'s `ui.virtual.invisible` scope, whatever the options above are set to. They are not whitespace you can choose to hide: left invisible they misalign the rest of the line, and an unseen bidirectional override can make code read differently from how it runs.

## Text wrap

Text wrap is controlled by three layers: a global default, a per-buffer override, and a per-pane pin, plus a per-pane toggle command.

- `wrap-mode` is a **buffer option** (see [Buffer options](#buffer-options)): set a global default with `:set global wrap-mode=<value>` or `set-option!`, or override one buffer with `:set buffer wrap-mode=<value>` or `(set-buffer-option! pane "wrap-mode" value)`. To wrap by file type (markdown but not source code, say), set it per language from an `on-language-set` hook (see [From `init.scm`](configuration.md#from-init-scm)).
- `:set pane wrap-mode=<value>` pins the style for the pane you're currently in and the buffer it's currently showing, above both the buffer and global setting, without affecting other panes on the same buffer.
- `:wrap` (alias of `:toggle-soft-wrap`) **toggles** wrapping on or off for the current pane and buffer.

::: details How pins and `:wrap` work together
Switching a pinned pane to a different buffer resolves the new buffer's own setting instead; switching back returns the pin. There's no command to clear a pin back to following the buffer/global setting. Pin it to a different value, or close the buffer, to move on from it.

Turning wrapping off with `:wrap` pins the pane to no wrap; turning it back on restores whatever it was doing before: the buffer/global setting, if the pane wasn't pinned, or the exact style you pinned it to with `:set pane wrap-mode=…`. If that restores a setting that doesn't actually wrap, it pins the configured global style instead (or `indent`, if the global itself is `none`), so `:wrap` always visibly wraps.

Two panes showing the same buffer can still wrap independently once one of them is pinned with `:set pane` or `:wrap`; that's what the per-pane layer is for.
:::

Accepted values:

- `none`: no wrapping; long lines scroll horizontally.
- `soft`: break at the pane width, splitting at any character (may split a word in the middle).
- `word`: break at the pane width but prefer whitespace, so words aren't split.
- `indent`: like `word`, but wrapped continuation lines are indented to match the line's leading whitespace, so nested code stays visually nested (this is the default).
- `:N` suffix: wrap at column `N` instead of the pane's content width (e.g. `word:80`). `0` or omitted means content width.

## Statusline

The statusline is fully configurable from Scheme:

```scheme
(configure-statusline! '("Mode" "Separator" "FileName") '() '("Position"))
```

Each argument is a list of element name strings: left, center, right.

Available elements:

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode label (`NOR`/`INS`/`EXT`/`CMD`/`SRC`/`SIF`) |
| `"Separator"` | Divider between sections |
| `"FileName"` | Current buffer filename (basename) |
| `"FilePath"` | Full path of current buffer |
| `"Cwd"` | Working directory |
| `"Position"` | Line and column position; column counts graphemes (`h`/`l` presses), matching `:diagnostics` and goto/references lists |
| `"KittyProtocol"` | Kitty keyboard protocol indicator |
| `"DirtyIndicator"` | `[+]` when buffer has unsaved changes |
| `"LineEnding"` | Line ending type (LF/CRLF) |
| `"SearchMatches"` | Current search match count |
| `"MiniBuf"` | Pending key sequence hint |
| `"MacroRecording"` | Macro recording indicator |
| `"Language"` | Buffer language |
| `"ReadOnly"` | `[RO]` indicator |
| `"Diagnostics"` | Error and warning counts from the language server |

The default is equivalent to:

```scheme
(configure-statusline!
  '("Position" "FilePath" "Language" "ReadOnly" "DirtyIndicator")
  '()
  '("MacroRecording" "SearchMatches" "Diagnostics" "KittyProtocol" "Separator" "Mode"))
```

### Custom elements

Place `"steel:<name>"` for any `<name>` of your choosing to add your own element. `<name>` must be non-empty and must *not* contain `,` or `|`. Push its text with `set-statusline-text!`, driven by whatever should trigger an update (a hook, a timer, a command):

```scheme
(configure-statusline! '("steel:line-count" "FilePath") '() '("Position" "Mode"))

(define (refresh-line-count! pane)
  (set-statusline-text! "line-count" pane
    (string-append (number->string (buffer-line-count pane)) "L")))

(register-hook! 'on-text-changed refresh-line-count!)
(register-hook! 'on-buffer-enter refresh-line-count!)
```

`core:git-diff` ships a `"steel:git-branch"` element using this same mechanism (see [Core Plugins → core:git-diff](core-plugins.md#core-git-diff)); just add it to your own `configure-statusline!` call.

`set-statusline-text!` takes the element name, a pane, and the text to show; an empty string clears it. Each buffer keeps its own value per name, and a placed element shows only the focused buffer's: switching to a buffer with nothing pushed yet shows nothing, same as any other element with no content. Placing the element and pushing its text are independent: either can happen first, and neither errors if the other hasn't happened yet.

## Language detection

HUME detects file languages from extension, glob pattern, or shebang line. See [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language) for defining custom languages with `define-language!`, setting their project roots and language-server identifier with `configure-language!` and, for grammars HUME doesn't ship, `register-grammar!`.
