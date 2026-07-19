# HUME Golf

Vimgolf-style editing challenges for HUME. Each challenge transforms a starting
text (`in`) into a target text (`out`) using a sequence of keystrokes (`cmd`).
Lowest keystroke count wins.

## Running

```sh
cd tools/golf
./golf.sh
```

The script builds HUME automatically before running.

Score a single challenge or all challenges at once:

```sh
./golf.sh 4d1a34ccce8814b72600002b  # one challenge
./golf.sh                           # all in challenges/
```

## Output

```
CHALLENGE                      HUME  KAKOUNE  RESULT
----------------------------------------------------------------------
4d1a34ccfa85f32065000004         10       10  OK
4d1a8bf2b8cb3409320002c4          6        8  OK
----------------------------------------------------------------------
2 passed, 0 failed
```

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

Score comparisons with Kakoune are for orientation, not benchmarking. The
`KAKOUNE` column shows the best known score from `mawww/golf`, adjusted by −2
to account for the mandatory `<space>q` (save-and-quit) overhead that Kakoune
solutions require but HUME does not.

HUME's goal is to be pragmatic and intuitive — features are designed to make
editing feel natural, not to minimise keystroke counts. A challenge where HUME
loses is not a bug; a challenge where it wins is not a design target. The
comparisons show where the two editing models diverge and why.
