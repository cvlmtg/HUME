# Installation

HUME is a single binary with no runtime dependencies. Download it, run it, and you have an editor — syntax highlighting and language servers are added later, on demand, only for the languages you actually use.

## Download

Grab the latest release from the [**releases page**](https://github.com/cvlmtg/HUME/releases). The newest tagged version is the one most people want; a [`nightly`](https://github.com/cvlmtg/HUME/releases/tag/nightly) build is also published if you want the newest changes. However keep in mind that the nightly release might be unstable or contain half-finished features.

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

Or copy it into `/usr/local/` for a system-wide install:

```sh
cp -R hume-*/* /usr/local/
```

### Windows

Extract the `.zip`, then run `hume.exe` from inside the extracted folder, or add that folder to your `PATH`.

Keep the folder intact either way: the binary looks for its runtime files (themes, plugins, language definitions) alongside itself, so moving `hume` out on its own leaves it without them.

### Check it works

```sh
hume --version
```

prints something like `hume 0.9.0-f460770`. The same string is available inside the editor with `:version`.

## Building from source

**Prerequisites:** a Rust toolchain — install one from [rustup.rs](https://rustup.rs/).

```sh
git clone https://github.com/cvlmtg/HUME
cd HUME
cargo build --release
./target/release/hume
```

Run it from the repository root, as above — HUME picks up the `runtime/` directory sitting there.

::: warning `cargo install` needs one extra step
`cargo install --git https://github.com/cvlmtg/HUME` installs the binary but **not** the runtime files, so themes, plugins, `:tutor`, and syntax highlighting won't work. Point HUME at a copy of the `runtime/` directory to fix it:

```sh
export HUME_RUNTIME=/path/to/HUME/runtime
```
:::

## Syntax highlighting

HUME highlights code with tree-sitter, and installs grammars on demand rather than shipping them. That first install needs `git`, `curl`, the `tree-sitter` CLI, and a C compiler on your `PATH`. See [Syntax Highlighting](syntax-highlighting.md) for the walkthrough.

## Terminal compatibility

HUME targets modern terminals and degrades quietly rather than refusing to start — there is no capability check at launch. You'll get the best results from a terminal that supports:

- **24-bit true color.** Colors are always emitted as RGB. Terminals without true color will show approximate or wrong colors, but HUME still runs.
- **Synchronized output.** Sent once per frame to avoid tearing; terminals that don't recognise it ignore it harmlessly.
- **The kitty keyboard protocol.** Detected automatically on WezTerm, kitty, ghostty, and foot. It unlocks a handful of extra key combinations — everything else works without it, and the manual marks those keys "kitty only".
