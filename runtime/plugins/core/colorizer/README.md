# core:colorizer

Paints color literals with the color they name: `#rgb`, `#rgba`, `#rrggbb` and `#rrggbbaa`,
and `rgb()`, `rgba()`, `hsl()`, `hsla()` and `oklch()` calls, in every language, and the 148
CSS named colors in the languages listed in `'named-languages`. A literal's alpha is
dropped.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:colorizer" #:config (hash 'mode 'background))
```

- **Depends on:** `core:stdlib`, for config validation.
- **Activates on:** `on-buffer-open`, or either toggle command (`manifest.scm`).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-colorizer).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'enabled` | bool | `#t` | Whether painting starts on for each buffer |
| `'mode` | `'background` or `'foreground` | `'background` | `'background` paints the literal's background with the color and its text black or white, whichever reads; `'foreground` paints its text |
| `'named-languages` | list of strings | `'("css" "scss" "less")` | Languages whose buffers also paint color names, and paint a 3- or 4-digit literal with no `a`-`f` digit |
| `'max-render-chars` | integer ≥ 1 | `7100` | The chars one render may walk; a line that does not fit what is left is not painted |

## Commands

| Command | Effect |
|---|---|
| `:toggle-colorizer` | Turn painting off for the current buffer, clearing its spans, or back on, painting at once |
| `colorizer/toggle` | The same procedure, bindable |

## How it works

### Files

`plugin.scm` reads the config and registers the hooks and commands. `lib/render.scm` keeps
the per-buffer state and the scopes already painted, and turns what the scanner finds into
styles and spans. `lib/scan.scm` finds the literals and reads a call's
arguments. `lib/convert.scm` turns a call's values into the bytes of an sRGB color,
remembers each `oklch()` result, and holds `colorizer/box-insert!`, which the session
tables and each buffer's line table grow through.
`lib/names.scm` is the names table.

### Scanning

A render reads `(buffer-viewport-ranges pane)`, the lines every pane on the buffer shows,
already sorted and with overlapping or touching ranges joined. Each range is read with one
`buffer-lines #:range`. The char offset of each line is the previous line's plus its
`string-length` plus one, starting at `line->char-offset` of the range's first line.

Each line is scanned on its own: a literal, a name and a call all start and end on one line.
`string-matches` finds the candidates on a line in one native pass, with a pattern
`scan.scm` compiles once at load: a `#`, a function name directly followed by `(`, and, for a
`'named-languages` buffer, one more with every key of the names table. A line with no
candidate is done. Otherwise the line becomes one `string->list`, and each candidate is read
from `list-tail` at its offset, the char before it included, by the same rules a walk from the
line start applies (below). A candidate that starts inside the last painted piece is skipped,
so the next candidate read is the next char a walk from the line start would reach.
`string-ref` is never used: it walks the string from its start on each call.

A candidate never hides another literal. A `#` candidate is one char, and every char of the
other candidates after their first is a letter or their final `(`, and a literal never starts
at a letter after a letter. So a candidate the rules refuse leaves every later start to the
next candidates.

### Render budget

One render walks at most `'max-render-chars` chars, one budget for all its ranges. Before any
line is walked, a claim pass measures each line once (a line in the table below gives its
length from there) and marks it, in buffer order. A line claims its `string-length` when that
is at most what remains and is walked; a longer one is skipped and gives no piece, and a
later line that fits still claims. A line the table already holds claims nothing.

Each buffer keeps, for one `buffer-generation` and one `'named-languages` answer, the length
and the pieces of every line it walked. A later render of the same text, such as one after a
scroll, takes such a line from the table instead of walking it.

A word char is an ASCII letter, an ASCII digit, `_` or `-`; every other char, non-ASCII
ones included, ends a word.

- **Hex literal:** a `#` whose previous char is neither a word char nor `&`, then 3, 4, 6
  or 8 hex digits, then a char that is not a word char, or the line end. A 3- or 4-digit
  literal with no `a`-`f` digit matches only in a `'named-languages` buffer.
- **Name:** in a `'named-languages` buffer, a run of ASCII letters bounded by non-word chars
  whose lowercase is a key of the names table.
- **Function call:** `rgb`, `rgba`, `hsl`, `hsla` or `oklch`, in any case, after a non-word
  char and directly followed by `(`. A letter run that is a function name followed by `(`
  is read as a call first.

### Function calls

A call's arguments follow CSS Color 4. Whitespace and `/* … */` comments may sit between
any two tokens. A value is a CSS `<number>` (an optional sign, digits with an optional fraction or a leading
`.`, an optional exponent), with `%` or, for a hue, `deg`, `grad`, `rad` or `turn`; or
`none`. The number's chars go to `string->number`. A number too large for a float leaves the
call unpainted, since the chroma search below would never end on an infinite chroma.

After the first value, the next char picks the syntax. A `,` reads the legacy syntax: three
values with a `,` between each, then an optional `, alpha`; `rgb` takes three numbers or
three percentages, `hsl` a hue then two percentages, `oklch` none at all. Anything else reads
the modern syntax: three values, then an optional `/ alpha`; `none` stands for 0. A `,` met
in the modern syntax, a value with no `,` before it in the legacy one, a nested `(`, a word
other than `none` or a unit, a unit the position does not take, or the end of the line
leaves the call unpainted, and the walk resumes after its `(`.

The scanner resolves each value's unit into the function's own scale (an `rgb` channel, an
`hsl` saturation or lightness, and an `oklch` lightness in 0-1, a chroma where 100% is 0.4,
a hue in degrees), then hands the three to `lib/convert.scm`. The alpha is checked, then
dropped.

A call is painted as one piece, from its name through the `)`.

A `/*` with no `*/` before the line ends fails every call that reaches it. The walk of a
line remembers the first such `/*`, and a later comment on that line that opens at or after
it fails at once, so a line with many calls inside one unclosed comment is still walked
once.

### Conversion

`lib/convert.scm` clamps each value as CSS does (channels, saturation and lightness to 0-1,
the chroma to at least 0, the hue modulo 360) and gives each channel the byte
`floor(v × 255 + 0.5)`, rounding half up as browsers do, where Scheme's `round` would round
half to even.

- **`hsl`:** CSS Color 4's `hslToRgb`.
- **`oklch`:** to OKLab (`a = C cos h`, `b = C sin h`), to linear sRGB through Björn
  Ottosson's matrices, then the sRGB transfer function. A color outside sRGB is mapped by
  CSS Color 4's "CSS gamut mapping to an RGB destination": `L ≥ 1` gives white, `L ≤ 0`
  black, and a color inside sRGB is kept. Otherwise a color within `JND = 0.02` of its clip
  (ΔEOK, the distance in OKLab) is the clip, and any other color searches its chroma by
  halving until the interval is under `ε = 0.0001`, keeping lightness and hue, and answers
  the last clipped candidate.
- **ΔEOK** takes the clipped color back to OKLab: the inverse transfer, the inverse matrices,
  and a cube root as `(expt x (/ 1.0 3))`, Steel having no `cbrt`. A clipped channel is never
  negative, so the cube root needs no sign handling.

The search measures each candidate in one pass that builds no list: `colorizer/let-srgb`, the
one forward conversion, binds the three channels as locals, and `colorizer/clip-error`
answers -1 for a candidate inside sRGB, or else the ΔEOK of its clip. A candidate inside sRGB
is its own clip, at a ΔEOK of 0 up to float error, which the search treats as below JND.
The search keeps the chroma of the last clipped candidate and converts it once more for its
bytes. Each `oklch()` result is kept for the session, keyed by the call's three values, so a
render that shows a call again does not search again.

A chroma too large for the matrices gives NaN channels. The clamp maps NaN to 0, and a NaN
channel is never in gamut, so such a color still yields a byte.

The background mode's luminance uses the same inverse transfer.

### Styles and spans

Each color has one scope per mode, `colorizer.bg.rrggbb` or `colorizer.fg.rrggbb`, so two
literals of one color share it. The plugin remembers, for the session, each color it has
painted with its scope, and every span of a color reuses that one scope string. A render
sends the styles of the colors new to that table in one `set-scope-styles!` call, and none
when every color is known, since each call restyles the whole theme. Then one
`set-extra-highlights!` call replaces the buffer's spans, and the new colors join the table.

The session refuses a scope name it has not seen once it holds as many as it allows, and
`set-extra-highlights!` then stores nothing. The plugin catches that one refusal, logs a
warning once for the session, and paints again with only the colors in its table, each
already in the session. From then on a render drops every other color and sends no
styles. Every other error from the call is raised as it is.

The session tables, the painted colors and the `oklch()` results, and each buffer's line
table grow through `colorizer/box-insert!`, which empties the box before its `hash-insert`. Steel changes a hash
that nothing else holds in place. A hash still held by its box is copied instead, and
dropping the version the box held then walks every entry, so each insert would cost the size
of the table.

In `'background` mode the text is black when the color's WCAG relative luminance is above
0.179, and white otherwise.

### Refresh and liveness

`on-viewport-change` and `on-language-set` render at once. `on-text-changed` renders
through a 50 ms `debounce-by` keyed by `buffer-key`, so a burst of edits renders once.
A render whose `buffer-generation`, ranges and `'named-languages` answer are those of the
buffer's last paint returns at once, calling no setter: an edit that also moves the view
fires both hooks, and every pane on the buffer fires its own `on-viewport-change`. A toggle
forgets the last paint.
`on-buffer-close` drops the buffer's state. A debounced render can fire after its buffer
closed, so a render checks `buffer-live?` before it reads or tracks the buffer.

## Performance

`hume-editor/benches/colorizer.rs` times one render, alone and followed by a frame, on prose
with no literal, one hex literal per line, one out-of-gamut `oklch()` per line, one minified
CSS line of 500 to 16,000 chars, a minified line of the default budget on every line
(`long_lines`, at the default budget), and prose lines of 80 to 200 chars (`screen_<n>`), at
80x25 and 200x60. Each timed render follows a deleted char and its undo, so it paints a text
it has not walked. Its `scroll_render_then_frame` rows page down before each render instead,
so every render styles colors new to the session and the next frame re-bakes the theme.

The default of `'max-render-chars`, 7,100, bounds what dense text costs. Prose costs little
per char, since only its candidates are read in Steel: every `screen_<n>` render at 200x60
takes about 1 ms on the efficiency cores
(`taskpolicy -c background cargo bench -p hume-editor --bench colorizer`). Minified CSS has
a candidate every few chars, so it costs the most per char: one minified line takes 10.7 ms
at 4,000 chars and 21.5 ms at 8,000, and the `long_lines` render, which spends the whole
budget on it, takes 19 ms, just over one 16 ms frame. At the default, a 200x60 screen of lines
up to 120 chars (7,080) paints whole, and a screen of longer lines loses its last lines.

A render's cost grows with the candidates it reads in Steel. A call costs more than a literal,
and an out-of-gamut `oklch()` the most, for its chroma search.

## Design decisions

- **Id selectors are painted:** `#bad {` in a stylesheet paints like a color, since nothing
  tells the two apart in the text. `div#bad` does not, by the leading-char rule.
- **`#123` outside stylesheets:** a 3- or 4-digit literal with only decimal digits, such as
  an issue number, is painted only in a `'named-languages` buffer.
- **One line at a time:** a call must open and close on one line. Seeing a call that starts
  above the view or ends below it means reading lines around every view on every render,
  and color calls are written on one line in practice.
- **Names table, not palette names:** names map to hex through `lib/names.scm`, so a
  theme's own palette entry named `red` never changes what `red` paints.

## Known limitations

- `hwb()`, `lab()`, `lch()`, `oklab()` and `color()` are not painted, nor a call whose
  arguments need `var()`, `env()`, `attr()`, `calc()` or a relative color (`rgb(from …)`).
- A call written across several lines is not painted.
- A line that does not fit what remains of the render budget is not painted, so a 200x60
  screen of lines longer than 120 chars, minified CSS among them, loses its last lines.
- Each color is a scope name, and the session holds a fixed number of them, about 65,000,
  shared with themes, grammars and other plugins. Once it is full, a color the plugin has
  not painted before stays unpainted.
