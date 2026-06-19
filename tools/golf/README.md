# HUME Golf

Vimgolf-style editing challenges for HUME. Each challenge transforms a starting
text (`in`) into a target text (`out`) using a sequence of keystrokes (`cmd`).
Lowest keystroke count wins.

## Setup

Build HUME first:

```sh
cargo build --release
```

## Running

```sh
cd tools/golf
./golf.sh
```

The script finds the HUME binary automatically (checks `$PATH`, then
`../../target/release/hume`, then `../../target/debug/hume`).

Score a single challenge or a directory of them:

```sh
./golf.sh challenges/4d1a34ccce8814b72600002b/  # one challenge
./golf.sh                                         # all in challenges/
```

## Output

```
CHALLENGE                      SCORE  RESULT
------------------------------------------------------------
example-add-exclamations           5  OK
example-delete-first-line          2  OK
example-replace-word              10  OK
------------------------------------------------------------
3 passed, 0 failed
```

`SCORE` is the keystroke count for HUME's solution in the `cmd` file.
`RESULT` is `OK` if the output matches `out` byte-for-byte, `FAIL` otherwise.
Challenges without a `cmd` file are listed as `SKIP`.

## Downloading real vimgolf challenges

```sh
./fetch-challenge.sh <CHALLENGE_ID>
```

This fetches the challenge from vimgolf.com (requires `curl` and `jq`), writes
`challenges/<ID>/{in,out}`, and creates a blank `cmd` for you to fill in.

```sh
./fetch-challenge.sh 4d1a34ccce8814b72600002b
# Downloaded: Simple format change
#   challenges/4d1a34ccce8814b72600002b/
#   Solve it in hume, then write your keystrokes to: challenges/4d1a34ccce8814b72600002b/cmd
#   Score it with: ./golf.sh challenges/4d1a34ccce8814b72600002b/
```

The `example-*` directories are hand-crafted challenges designed for the
[learning doc](../../docs/learning/vimgolf.md). Real vimgolf challenges are
named by their hex ID.

## Keystroke notation

The `cmd` files use a continuous stream format:

- Bare printable characters count as 1 keystroke each (space is literal).
- `<name>` tokens count as 1 keystroke each:
  - Named keys: `<esc>`, `<ret>`, `<tab>`, `<backspace>`, `<up>`, `<down>`, `<left>`, `<right>`
  - Modifier combinations: `<c-x>` (Ctrl+x), `<a-b>` (Alt+b), `<s-tab>` (Shift+Tab)
  - Long forms also work: `<ctrl-x>`, `<alt-b>`
  - `<lt>` for a literal `<`

## Adding challenges manually

Create a directory under `challenges/` with three files:

```
challenges/my-challenge/
  in    # starting text (must end with newline)
  out   # target text (must end with newline)
  cmd   # HUME keystrokes in the notation above
```

Verify it passes by running `./golf.sh challenges/my-challenge/`.

## Cross-editor scores

Score comparisons with Vim are for fun and orientation. Different editing models
have different strengths; a challenge that rewards a Vim-specific idiom will
score poorly for HUME, and vice versa. See `docs/learning/vimgolf.md` for a
discussion of where the models differ.
