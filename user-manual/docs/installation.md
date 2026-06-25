# Installation

## Nightly builds

Pre-built archives are published on every push to `main` at the [**nightly release**](https://github.com/cvlmtg/HUME/releases/tag/nightly) page.

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

## Grammar installation (syntax highlighting)

HUME uses tree-sitter for syntax highlighting. PLUM handles grammar installation automatically:

```
:plum-install-grammar       # install grammar for current buffer's language
:plum-list-grammars
:plum-ensure-grammars
```

Grammars are compiled native libraries stored under your HUME data directory. Most grammars install without any additional tools beyond a C compiler.
