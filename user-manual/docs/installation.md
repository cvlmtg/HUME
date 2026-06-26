# Installation

HUME is still in its early phases and ships **nightly builds only** — there is no stable release channel yet. Pre-built archives are republished on every push to `main` at the [**nightly release**](https://github.com/cvlmtg/HUME/releases/tag/nightly) page.

| Platform | Archive |
|---|---|
| macOS (Apple Silicon) | `hume-*-aarch64-apple-darwin.tar.gz` |
| Linux (x86\_64, glibc 2.39+) | `hume-*-x86_64-unknown-linux-gnu.tar.gz` |
| Windows (x86\_64) | `hume-*-x86_64-pc-windows-msvc.zip` |

### macOS / Linux

Extract and run:

```sh
tar xzf hume-*.tar.gz
./hume-*/bin/hume
```
Or copy to `/usr/local/` for a system install:

```sh
cp -R hume-*/* /usr/local/
```

### Windows

Extract the `.zip`, then run `hume.exe` from inside the folder, or add the folder to your `PATH`.

Run `hume --version` to confirm the build (e.g. `hume 0.1.0-f460770`). The same string is available inside the editor via `:version`.

## Building from source

**Prerequisites:** Rust toolchain (install via [rustup.rs](https://rustup.rs/)).

```sh
cargo install --git https://github.com/cvlmtg/HUME
```

Or clone and build manually:

```sh
git clone https://github.com/cvlmtg/HUME
cd HUME
cargo build --release
./target/release/hume
```

## Syntax highlighting

HUME highlights code with tree-sitter. Grammars are installed on demand by PLUM — see [Syntax Highlighting](syntax-highlighting.md) for the full guide, including prerequisites (`git`, `curl`, `tree-sitter`, a C compiler) and the `:plum-*` commands.

## Terminal compatibility

HUME targets modern terminals. There is no hard capability check at startup — unsupported features degrade silently rather than blocking launch — but you'll get the best experience with a terminal that supports:

- **24-bit true color.** Colors are emitted as RGB escapes unconditionally. Terminals without truecolor will render an approximation or wrong colors, but HUME will still run.
- **Synchronized output (DEC 2026).** Emitted unconditionally per frame; terminals that don't recognise it ignore the sequence (no harm done).
- **Kitty keyboard protocol.** Auto-detected at startup on supported terminals (WezTerm, kitty, ghostty, foot). When available it enables:
  - `Ctrl+;` (collapse selection to anchor)
  - `Ctrl+,` (remove primary selection)
  - `Ctrl+h`/`j`/`k`/`l`/`w`/`b` one-shot extend of the corresponding motion
  - `Ctrl+Shift+<char>` one-shot extend via `REPORT_ALTERNATE_KEYS`

  On a legacy terminal all of the above are **silent no-ops**. The keys that emit real C0 control bytes — `Ctrl+d`, `Ctrl+u`, `Ctrl+o`, `Ctrl+i` (also `Tab`), `Ctrl+r`, `Ctrl+e`, `Ctrl+x`, `Ctrl+X`, `Ctrl+6` — work everywhere.

  Known limitation: WezTerm builds before `20240203-110809-5046fc22` do not fully support `REPORT_ALTERNATE_KEYS`, so `Ctrl+Shift+<char>` one-shot extend may fail there even with kitty enabled.
