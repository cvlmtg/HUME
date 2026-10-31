# core:plum — Plugin and theme repos

Third-party plugins (`lib/plugins.scm`) and themes (`themes.scm`) are both GitHub repos
cloned under `<data>/`, and share the clone and checkout code in `lib/helpers.scm`.

## Plugin discovery

Declared plugins live in `init.scm`. Installed plugins are found by walking
`<data>/plugins/<user>/<repo>/` and checking each leaf directory for a `plugin.scm`.
"Missing" and "orphan" are set differences between the declared list and that walk, with
`core:*` plugins excluded since they are bundled.

## Pinned plugins

`(load-plugin! "user/repo" #:ref "v1.2.0")` pins a plugin to a tag, a branch or a
commit id, full or abbreviated; `(declared-plugin-ref name)` reads the pin back.
`load-plugin!` raises for a ref `git check-ref-format` would refuse, for one starting
with `-`, which no tag or branch name can, and for a ref on a `core:` plugin.

### `plum/git-sync!`

`plum/git-sync!` puts a clone on a ref. Without a pin it fetches `HEAD`, the remote's
default branch, and for a full 40-digit commit id it fetches that commit alone, whether or
not a branch or tag reaches it. Any other pin is a tag, a branch or an abbreviated commit
id, and `git fetch` cannot resolve an abbreviated id on the remote, so it fetches every
branch and tag (`git fetch --force --prune --tags origin`) and resolves the ref locally,
taking the first of `refs/tags/<ref>`, `refs/remotes/origin/<ref>` and, for 4 or more hex
digits, the commit id itself that `git rev-parse --verify` accepts. A ref that resolves to
nothing (missing upstream, an ambiguous short id, or a short id of a commit no branch or
tag reaches, which the fetch never brings in) raises an error naming it. A tag or branch
named with 40 hex digits is read as a commit id.

Either way it ends with `git checkout --detach`, which moves a clone between any two
states: onto a tag, from one tag to another, from a tag to a branch, and off a tag once the
pin is removed, where `git pull` would refuse a detached HEAD. So every clone is on a
detached HEAD, and a local edit that conflicts with the new checkout fails the update for
that plugin. An update also leaves behind commits made in the clone and any branch checked
out there by hand.

### `plum/install-repo!`

`plum/install-repo!`, behind every plugin and theme install, runs `plum/git-sync!` on a
fresh repo: `git init` in `<data>/plum-staging/<parent>/<name>`, `git remote add origin
<url>`, origin made a promisor remote with `partialclonefilter blob:none` (a blobless
clone, which downloads only the files of the commits it checks out), `plum/git-sync!`,
then the caller's `#:build` on the staging clone, if any. Only then does
`plum/staged-install!` move the new clone into the old one's place with `replace-dir!`, in
one step the script budget cannot stop half way. A failed install deletes its staging
clone and leaves the old clone untouched, then raises the failure's own error.

`:plum-install-plugins` installs only missing plugins, so a changed pin reaches an
installed plugin through `:plum-update-plugins`.

## Theme install

A theme repo is a GitHub `user/repo` with a `themes/*.toml` directory at its root (for
example [everforest.hume](https://github.com/cvlmtg/everforest.hume)).
`:plum-install-theme <user/repo>` clones it and copies that directory's `.toml` files flat
into `<data>/themes/`. That is the search tier `hume-editor`'s theme loader and
`:theme <Tab>` completer read; both glob `*.toml` files there without recursing.
`:theme <name>` picks up an installed theme with no `:reload-config`, since theme lookup
reads the filesystem at load time and the completer re-scans on every `Tab`.

### Slug validation

A slug not shaped like `user/repo` is a usage mistake and logs `'info`, which reaches the
status line but not `:messages`. A slug of the right shape with a segment
`safe-path-segment?` rejects would otherwise reach `path-join` and `git`, so it raises and
stays in `:messages`.

### Layout and discovery

Each clone is kept at `<data>/themes/sources/<user>/<repo>/`. A `sources/` directory has
no extension, so it never matches the `*.toml` glob. The clone is the record of which repo
a given `<data>/themes/<name>.toml` came from, so `:plum-update-themes`,
`:plum-list-themes` and `:plum-remove-theme` need no state file.

Theme repos are discovered by walking `<data>/themes/sources/<user>/<repo>/` for a `.git`
marker rather than a `themes/` directory, so a repo stays discoverable and removable after
upstream drops its `themes/` directory and `:plum-update-themes` starts failing its sync.

### Name collisions

Two installed repos that provide the same theme name write the same
`<data>/themes/<name>.toml`. The sync step logs a `'warning` naming the other repos
whenever it is about to write a name another installed repo also provides.

### Reinstall

`:plum-install-theme` always installs from a fresh clone through `plum/install-repo!`,
which also repairs a broken install. Its `#:build` raises when the new clone has no
`themes/*.toml`, so a repo without themes fails before it replaces an installed clone, and
the installed copies stay as they were.

## Design decisions

- **Theme repos are found by `.git`, not `themes/`.** See
  [Layout and discovery](#layout-and-discovery).

## Known limitations

- **Removing the active theme does not unload it.** `:plum-remove-theme` deletes files on
  disk only; the theme stays loaded until the next `:theme <name>` or a restart.
