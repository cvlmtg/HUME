#!/usr/bin/env python3
"""Regenerate runtime/plugins/core/lsp-install/sources.scm from mason-org/mason-registry.

Reads the pinned release tag from runtime/plugins/core/lsp-install/mason-pin.scm, downloads
that release's compiled registry.json.zip, joins it against the checked-in
runtime/plugins/core/lsp-install/servers.scm (server names Helix actually wires) through
an explicit name-mapping table, and rewrites sources.scm with per-server
install records.

Standalone and slow by design: on a from-scratch run it downloads every
selected github asset to compute a sha256. A repeat run reuses the sha256
already recorded in the checked-in sources.scm for any (repo, version,
asset-file) combination that hasn't changed, so a routine re-sync after an
unrelated pin bump downloads only the genuinely new or changed assets. Run
after sync-grammars.py if a helix-pin bump renamed or dropped any LSP
servers — see scripts/README.md for the run order.

Idempotent: running twice against the same pins produces a byte-identical
file.
"""

from __future__ import annotations

import collections
import hashlib
import http.client
import io
import json
import re
import sys
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parent))
from sync_common import (  # noqa: E402
    fetch_bytes,
    read_pin,
    read_sexpr,
    scheme_str,
    write_generated_file,
)

REPO = Path(__file__).resolve().parent.parent
MASON_PIN_SCM = REPO / "runtime" / "plugins" / "core" / "lsp-install" / "mason-pin.scm"
LSP_SERVERS_SCM = REPO / "runtime" / "plugins" / "core" / "lsp-install" / "servers.scm"
LSP_SOURCES_SCM = REPO / "runtime" / "plugins" / "core" / "lsp-install" / "sources.scm"

LSP_SOURCES_HEADER = """\
;;; runtime/plugins/core/lsp-install/sources.scm — HUME bundled LSP server install catalog.
;;; Generated — do not hand-edit. Record format: README.md, this directory.
;;; Source: mason-org/mason-registry @ {tag}
"""

# Helix server name -> Mason package name, for the cases where the two
# namespaces genuinely differ. Every entry below was confirmed by checking
# that the Mason package's `bin` map contains a key exactly equal to the
# Helix server's `command` string (see scripts/README.md) — a name
# match alone is not enough evidence (e.g. Mason's "vetur-vls" looked like a
# plausible match for Helix's "vuels" by name/lspconfig alias, but its bin
# key "vls" does not match vuels's actual command "vue-language-server";
# Mason's "vue-language-server" package is the real match).
HELIX_TO_MASON = {
    "actions-language-server": "gh-actions-language-server",
    "ada-gpr-language-server": "ada-language-server",
    "astro-ls": "astro-language-server",
    "dhall-lsp-server": "dhall-lsp",
    "djlsp": "django-template-lsp",
    "docker-compose-langserver": "docker-compose-language-service",
    "docker-langserver": "dockerfile-language-server",
    "fsharp-ls": "fsautocomplete",
    "graphql-language-service": "graphql-language-service-cli",
    "helm_ls": "helm-ls",
    "kcl-lsp": "kcl",
    "luau": "luau-lsp",
    "nls": "nickel-lang-lsp",
    "ocamllsp": "ocaml-lsp",
    "pylsp": "python-lsp-server",
    "slangd": "slang",
    "solc": "solidity",
    "svelteserver": "svelte-language-server",
    "typespec": "tsp-server",
    "verible-verilog-ls": "verible",
    "vhdl_ls": "rust_hdl",
    "vlang-language-server": "v-analyzer",
    "vscode-css-language-server": "css-lsp",
    "vscode-html-language-server": "html-lsp",
    "vscode-json-language-server": "json-lsp",
    "vuels": "vue-language-server",
    "yls": "yls-yara",
}

# Kinds whose install is a package-manager command, so a Mason
# `supported_platforms` restriction applies to them. A downloaded asset is
# already per-platform through its own targets.
PACKAGE_MANAGER_KINDS = ("npm", "cargo", "golang", "pypi", "gem", "nuget")

# Mason packages that share a Helix server's name but run a different program
# than the command Helix registers, so installing one would register the wrong
# binary.
MASON_NOT_HELIX_SERVER = {
    "cuelsp": "Helix runs `cue lsp serve`; Mason's cuelsp is a different program",
}

# A generic package whose download lists several files names the one that is
# the server; the others (e.g. jdtls's lombok.jar) are not fetched.
GENERIC_PRIMARY_FILE = {"jdtls": "jdtls.tar.gz"}

# hume-target -> ordered Mason target names to try, most-preferred first
# (e.g. prefer a glibc Linux build over musl when both are offered). The
# trailing arch-agnostic entries (`darwin`, `linux`, `win`, `unix`) are real
# Mason target strings some packages use for a universal/interpreted asset
# (e.g. neocmakelsp's "universal-apple-darwin" build, or a zip'd script
# elixir-ls/kotlin-language-server ship once for all Unix hume-targets) —
# tried only after every arch-specific option is exhausted.
MASON_TARGET_PRIORITY = {
    "darwin-arm64": ("darwin_arm64", "darwin", "unix"),
    "darwin-x64": ("darwin_x64", "darwin", "unix"),
    "linux-x64": ("linux_x64_gnu", "linux_x64", "linux_x64_musl", "linux", "unix"),
    "windows-x64": ("win_x64", "win"),
}

ARCHIVE_EXTENSIONS = (".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".zip", ".gz", ".xz")
TAR_EXTENSIONS = (".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2")
# Extensions of downloads the installer cannot turn into a runnable binary.
# Anything else without one of the supported archive extensions is a bare
# executable.
UNSUPPORTED_EXTENSIONS = (
    ".xz", ".bz2", ".zst", ".tzst", ".7z", ".rar", ".dmg", ".pkg", ".deb", ".rpm",
    ".msi", ".vsix", ".nupkg", ".jar", ".whl",
)
_TEMPLATE_RE = re.compile(r"\{\{\s*([^}]+?)\s*\}\}")
_STRIP_PREFIX_RE = re.compile(r'version\s*\|\s*strip_prefix\s+"([^"]*)"')
_BIN_PREFIX_RE = re.compile(r"^[a-z][a-z0-9_]*:(?!//)")
_CARGO_SEMVER_RE = re.compile(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.\-]+)?")


def fetch_registry(tag: str) -> list:
    url = f"https://github.com/mason-org/mason-registry/releases/download/{tag}/registry.json.zip"
    blob = fetch_bytes(url, timeout=60)
    with zipfile.ZipFile(io.BytesIO(blob)) as zf:
        with zf.open("registry.json") as f:
            return json.loads(f.read().decode())


def parse_purl(purl: str) -> tuple[str, str, str]:
    """Return (kind, subject, version). subject is 'owner/repo' for github,
    or the (percent-decoded, possibly '@scope/name') package name otherwise.
    """
    if not purl.startswith("pkg:"):
        sys.exit(f"error: malformed purl (no 'pkg:' prefix): {purl}")
    kind, _, rest = purl[4:].partition("/")
    # Strip qualifiers (`?...`) and subpath (`#...`) — whichever appears
    # first, per the purl grammar `type/namespace/name@version?qualifiers#subpath`.
    # Dropping only `?` left a subpath fragment glued onto the version for any
    # purl with one (e.g. cuelsp's `...@v0.3.4#cmd/cuelsp`), silently
    # recording a wrong version.
    rest = re.split(r"[?#]", rest, maxsplit=1)[0]
    rest = urllib.parse.unquote(rest)
    subject, sep, version = rest.rpartition("@")
    if not sep:
        sys.exit(f"error: malformed purl (no @version): {purl}")
    return kind, subject, version


def strip_mason_subpath(file_field: str) -> str:
    """Drop a Mason archive-subpath-extraction suffix (`file.tar.gz:libexec/`)
    — HUME's installer extracts the whole archive and locates the binary by
    its recorded bin-path, so only the real download filename is kept."""
    if ":" not in file_field:
        return file_field
    head, _, _tail = file_field.partition(":")
    return head if head.endswith(ARCHIVE_EXTENSIONS) else file_field


def strip_mason_bin_prefix(value: str) -> str:
    """Drop a Mason interpreter/wrapper-hint prefix (`exec:`, `ruby:`, …) from
    a resolved github-kind bin path. HUME's installer chmod+x's every
    installed binary unconditionally and does not replicate Mason's
    interpreter-wrapper mechanics — only the underlying path is kept."""
    return _BIN_PREFIX_RE.sub("", value)


_TEMPLATE_MAX_PASSES = 5


def resolve_template(template: str, *, version: str, asset: dict) -> str | None:
    """Resolve {{...}} constructs against one asset entry. Returns None if the
    template references something unresolvable from static data alone (a
    `source.build.*` reference, a filter expression, a computed field not
    present in the asset) — the caller skips that one target with a report,
    rather than aborting the whole sync.

    Resolution is fixed-point, not single-pass: a `{{source.asset.*}}`
    reference can pull in another asset field (e.g. `file`) whose own raw
    value still contains an unresolved `{{version}}` — Mason's registry
    nests templates this way for several packages (clangd, verible, …).
    Re-running substitution until no `{{` remains (bounded by
    `_TEMPLATE_MAX_PASSES` so a self-referential template can't loop
    forever) is what lets the emitted data honor "the runtime sees literals
    only": a `{{` surviving into the final string is caught by the
    emit-time assertion in `main`, never shipped."""
    unresolved = False

    def repl(m):
        nonlocal unresolved
        expr = m.group(1).strip()
        if expr == "version":
            return version
        stripped = _STRIP_PREFIX_RE.fullmatch(expr)
        if stripped:
            return version.removeprefix(stripped.group(1))
        ref = next((r for r in ("source.asset.", "source.download.") if expr.startswith(r)), None)
        if ref:
            value = asset
            for key in expr[len(ref) :].split("."):
                if not isinstance(value, dict) or key not in value:
                    unresolved = True
                    return ""
                value = value[key]
            if not isinstance(value, str):
                unresolved = True
                return ""
            return value
        unresolved = True
        return ""

    result = template
    for _ in range(_TEMPLATE_MAX_PASSES):
        if "{{" not in result:
            break
        result = _TEMPLATE_RE.sub(repl, result)
    return None if unresolved or "{{" in result else result


def index_assets_by_mason_target(assets, package_name: str) -> dict:
    by_target = {}
    for asset in assets:
        targets = asset.get("target")
        for t in targets if isinstance(targets, list) else [targets]:
            if t in by_target and by_target[t] is not asset:
                sys.exit(
                    f"error: mason package '{package_name}' has two different assets both "
                    f"claiming target {t!r} — ambiguous, cannot pick one automatically"
                )
            by_target[t] = asset
    return by_target


def asset_format(asset_file: str, bin_path: str) -> str | None:
    """Return 'tar', 'zip', 'gz' or 'raw' for a download the runtime's
    `lsp/asset-format` (core:lsp servers.scm) can install, else None. A raw
    download is the binary itself, so its bin path must equal its file name."""
    if asset_file.endswith(TAR_EXTENSIONS):
        return "tar"
    if asset_file.endswith(".zip"):
        return "zip"
    if asset_file.endswith(".gz"):
        return "gz"
    if asset_file == bin_path and not asset_file.endswith(UNSUPPORTED_EXTENSIONS):
        return "raw"
    return None


def pick_bin_template(bin_map: dict, helix_command: str, server_name: str):
    if len(bin_map) == 1:
        return next(iter(bin_map.values()))
    if helix_command in bin_map:
        return bin_map[helix_command]
    print(
        f"  skip '{server_name}': ambiguous bin map {sorted(bin_map)} — "
        f"none match helix command '{helix_command}'",
        file=sys.stderr,
    )
    return None


def load_sha256_cache(path: Path) -> dict[str, str]:
    """Return {download-url: sha256} read from the previously checked-in
    sources.scm, so re-syncing after an unrelated pin bump doesn't
    re-download and re-hash every unchanged github or generic asset. Best-effort: a
    missing or unparseable file yields an empty cache (equivalent to a
    from-scratch run) rather than aborting the sync — the cache is a speed
    optimization, not a correctness dependency."""
    if not path.exists():
        return {}
    try:
        data = read_sexpr(path)
    except Exception as e:
        print(f"  warning: could not read {path} for hash cache: {e}", file=sys.stderr)
        return {}

    cache: dict[str, str] = {}
    for rec in data:
        fields = {str(f[0]): f[1] for f in rec[1:] if isinstance(f, tuple)}
        kind = fields.get("kind")
        if kind not in ("github", "generic"):
            continue
        repo = fields.get("repo")
        version = fields.get("version")
        targets_entry = next(
            (f for f in rec[1:] if isinstance(f, list) and f and str(f[0]) == "targets"),
            None,
        )
        if not targets_entry or (kind == "github" and not (repo and version)):
            continue
        for target_row in targets_entry[1:]:
            if kind == "github" and len(target_row) == 4:
                _hume_target, asset_file, sha256, _bin_path = target_row
                url = f"https://github.com/{repo}/releases/download/{version}/{asset_file}"
            elif kind == "generic" and len(target_row) == 5:
                _hume_target, _asset_file, url, sha256, _bin_path = target_row
            else:
                continue
            cache[url] = sha256
    return cache


def sha256_of_url(url: str):
    """Return the sha256 of the resource at `url`, or None if it can't be
    downloaded. A download failure here means Mason's own registry data is
    stale or wrong for this one asset (e.g. a release tag that was since
    deleted or renamed upstream) — the caller skips just that target rather
    than aborting the whole sync over one broken, unrelated package.

    Catches failures from both connect (`URLError`) and the chunked read
    loop after a connection was established — a mid-read timeout surfaces as
    `TimeoutError`/`socket.timeout` (an `OSError`, not a `URLError`) or a
    truncated response as `http.client.IncompleteRead` (not an `OSError` at
    all) — either would otherwise crash the whole sync over one flaky asset.
    """
    h = hashlib.sha256()
    try:
        with urllib.request.urlopen(url, timeout=180) as r:
            while True:
                chunk = r.read(1 << 16)
                if not chunk:
                    break
                h.update(chunk)
    except (URLError, OSError, http.client.IncompleteRead) as e:
        print(f"  download failed, skipping target: {url}: {e}", file=sys.stderr)
        return None
    return f"sha256:{h.hexdigest()}"


def build_github_record(
    name: str, package: dict, helix_command: str, reports: dict, hash_cache: dict[str, str]
):
    _kind, repo, version = parse_purl(package["source"]["id"])
    assets = package["source"].get("asset")
    if not assets:
        return {"kind": "github-build", "version": version}

    bin_map = package.get("bin") or {}
    bin_template = pick_bin_template(bin_map, helix_command, name)
    if bin_template is None:
        return None

    by_mason_target = index_assets_by_mason_target(
        assets if isinstance(assets, list) else [assets], name
    )

    targets = []
    for hume_target, mason_targets in MASON_TARGET_PRIORITY.items():
        asset = next((by_mason_target[t] for t in mason_targets if t in by_mason_target), None)
        if asset is None:
            print(f"  {name} [{hume_target}]: no matching Mason asset — dropped", file=sys.stderr)
            continue

        raw_file = strip_mason_subpath(asset["file"])
        resolved_file = resolve_template(raw_file, version=version, asset=asset)
        if resolved_file is None:
            reports["unresolved_targets"].append((name, hume_target, "file", raw_file))
            continue

        resolved_bin = resolve_template(bin_template, version=version, asset=asset)
        if resolved_bin is None:
            reports["unresolved_targets"].append((name, hume_target, "bin", bin_template))
            continue
        resolved_bin = strip_mason_bin_prefix(resolved_bin)

        fmt = asset_format(resolved_file, resolved_bin)
        if fmt is None:
            reports["unsupported_format"].append((name, hume_target, resolved_file))
            continue
        reports["format_census"][fmt] += 1

        url = f"https://github.com/{repo}/releases/download/{version}/{resolved_file}"
        cached = hash_cache.get(url)
        if cached is not None:
            print(f"  reusing cached sha256 for {name} [{hume_target}]: {resolved_file}", file=sys.stderr)
            sha256 = cached
        else:
            print(f"  hashing {name} [{hume_target}]: {resolved_file}", file=sys.stderr)
            sha256 = sha256_of_url(url)
            if sha256 is None:
                reports["download_failures"].append((name, hume_target, url))
                continue
        targets.append((hume_target, resolved_file, sha256, resolved_bin))

    for hume_target in MASON_TARGET_PRIORITY:
        if hume_target not in {t[0] for t in targets}:
            reports["missing_platform"][hume_target] += 1

    if not targets:
        # Every known target's asset was unmapped, unresolved, or failed to
        # download — a "github" record with an empty (targets) list would
        # claim installability that never exists on any platform. Downgrade
        # to the same stub shape used when Mason carries no assets at all
        # (`not assets` above): `:lsp-servers` reports it uniformly as
        # "not installable (kind github-build) in v1".
        reports["no_usable_targets"].append(name)
        return {"kind": "github-build", "version": version}

    return {"kind": "github", "version": version, "repo": repo, "targets": targets}


def build_generic_record(
    name: str, package: dict, helix_command: str, reports: dict, hash_cache: dict[str, str]
):
    _kind, _subject, version = parse_purl(package["source"]["id"])
    downloads = package["source"].get("download")
    if not downloads:
        return {"kind": "generic-build", "version": version}

    bin_template = pick_bin_template(package.get("bin") or {}, helix_command, name)
    if bin_template is None:
        return None
    python_launcher = bin_template.startswith("python:")

    by_mason_target = index_assets_by_mason_target(downloads, name)
    targets = []
    for hume_target, mason_targets in MASON_TARGET_PRIORITY.items():
        entry = next((by_mason_target[t] for t in mason_targets if t in by_mason_target), None)
        if entry is None:
            print(f"  {name} [{hume_target}]: no matching Mason download — dropped", file=sys.stderr)
            continue
        if python_launcher and hume_target == "windows-x64":
            print(f"  {name} [{hume_target}]: python launcher script, not directly executable — dropped", file=sys.stderr)
            continue

        files = entry["files"]
        local_name = GENERIC_PRIMARY_FILE.get(name) if len(files) > 1 else next(iter(files))
        if local_name not in files:
            sys.exit(f"error: generic package '{name}' has {sorted(files)}; add it to GENERIC_PRIMARY_FILE")
        url = resolve_template(files[local_name], version=version, asset=entry)
        resolved_bin = resolve_template(bin_template, version=version, asset=entry)
        if url is None or resolved_bin is None:
            reports["unresolved_targets"].append((name, hume_target, "url/bin", files[local_name]))
            continue
        resolved_bin = strip_mason_bin_prefix(resolved_bin)

        fmt = asset_format(local_name, resolved_bin)
        if fmt is None:
            reports["unsupported_format"].append((name, hume_target, local_name))
            continue
        reports["format_census"][fmt] += 1

        sha256 = hash_cache.get(url)
        if sha256 is None:
            print(f"  hashing {name} [{hume_target}]: {url}", file=sys.stderr)
            sha256 = sha256_of_url(url)
            if sha256 is None:
                reports["download_failures"].append((name, hume_target, url))
                continue
        targets.append((hume_target, local_name, url, sha256, resolved_bin))

    if not targets:
        reports["no_usable_targets"].append(name)
        return {"kind": "generic-build", "version": version}
    return {"kind": "generic", "version": version, "targets": targets}


def build_golang_record(name: str, package: dict, helix_command: str):
    purl = package["source"]["id"]
    _kind, subject, version = parse_purl(purl)
    subpath = purl.partition("#")[2]
    if "?" in purl.partition("#")[0]:
        print(f"  skip '{name}': golang purl has unsupported qualifiers: {purl}", file=sys.stderr)
        return None
    bin_template = pick_bin_template(package.get("bin") or {}, helix_command, name)
    if bin_template is None:
        return None
    if not bin_template.startswith("golang:"):
        print(f"  skip '{name}': golang-kind bin template has no golang: prefix: {bin_template!r}", file=sys.stderr)
        return None
    module = f"{subject}/{subpath}" if subpath else subject
    return {"kind": "golang", "version": version, "module": module, "bin": bin_template[len("golang:") :]}


def build_pypi_record(name: str, package: dict, helix_command: str):
    purl = package["source"]["id"]
    _kind, subject, version = parse_purl(purl)
    _, _, qualifier_str = purl.partition("?")
    qualifier_str = qualifier_str.partition("#")[0]
    qualifiers = [q.partition("=") for q in qualifier_str.split("&") if q]
    unknown = sorted({k for k, _, _ in qualifiers} - {"extra"})
    if unknown:
        print(f"  skip '{name}': pypi purl has unsupported qualifier(s) {unknown}", file=sys.stderr)
        return None
    extras = [v for k, _, v in qualifiers if k == "extra"]

    bin_template = pick_bin_template(package.get("bin") or {}, helix_command, name)
    if bin_template is None:
        return None
    if not bin_template.startswith("pypi:"):
        print(f"  skip '{name}': pypi-kind bin template has no pypi: prefix: {bin_template!r}", file=sys.stderr)
        return None
    return {
        "kind": "pypi",
        "version": version,
        "package": subject,
        "extras": extras,
        "bin": bin_template[len("pypi:") :],
    }


def build_gem_record(name: str, package: dict, helix_command: str):
    purl = package["source"]["id"]
    _kind, subject, version = parse_purl(purl)
    bin_template = pick_bin_template(package.get("bin") or {}, helix_command, name)
    if bin_template is None:
        return None
    if not bin_template.startswith("gem:"):
        print(f"  skip '{name}': gem-kind bin template has no gem: prefix: {bin_template!r}", file=sys.stderr)
        return None
    packages = [f"{subject}:{version}"] + list(package["source"].get("extra_packages", []))
    return {"kind": "gem", "version": version, "packages": packages, "bin": bin_template[len("gem:") :]}


def build_nuget_record(name: str, package: dict, helix_command: str):
    _kind, subject, version = parse_purl(package["source"]["id"])
    bin_template = pick_bin_template(package.get("bin") or {}, helix_command, name)
    if bin_template is None:
        return None
    if not bin_template.startswith("nuget:"):
        print(f"  skip '{name}': nuget-kind bin template has no nuget: prefix: {bin_template!r}", file=sys.stderr)
        return None
    return {"kind": "nuget", "version": version, "package": subject, "bin": bin_template[len("nuget:") :]}


def build_npm_record(name: str, package: dict, helix_command: str):
    _kind, subject, version = parse_purl(package["source"]["id"])
    extra_packages = package["source"].get("extra_packages", [])
    bin_map = package.get("bin") or {}
    bin_template = pick_bin_template(bin_map, helix_command, name)
    if bin_template is None:
        return None
    if not bin_template.startswith("npm:"):
        print(f"  skip '{name}': npm-kind bin template has no npm: prefix: {bin_template!r}", file=sys.stderr)
        return None
    script = bin_template[len("npm:") :]
    packages = [f"{subject}@{version}"] + list(extra_packages)
    return {"kind": "npm", "version": version, "packages": packages, "bin": script}


def build_cargo_record(name: str, package: dict, helix_command: str):
    purl = package["source"]["id"]
    _kind, subject, version = parse_purl(purl)
    if not _CARGO_SEMVER_RE.fullmatch(version):
        # A git-tag pin (e.g. nil@2025-06-13) — not reachable via a crates.io
        # `cargo install crate@version`. Same downgrade shape as github ->
        # github-build: a distinct stub kind the runtime reports verbatim.
        return {"kind": "cargo-git", "version": version}

    _, _, qualifier_str = purl.partition("?")
    qualifier_str = qualifier_str.partition("#")[0]
    qualifiers = {q.partition("=")[0] for q in qualifier_str.split("&") if q}
    unknown = qualifiers - {"repository_url"}
    if unknown:
        # `repository_url` is informational for a semver install (beancount
        # carries it with a valid crates.io version) and safe to ignore. Any
        # other qualifier (e.g. a future `features=…`) would silently install
        # the wrong binary if not accounted for — skip-with-report instead.
        print(
            f"  skip '{name}': cargo purl has unsupported qualifier(s) {sorted(unknown)}",
            file=sys.stderr,
        )
        return None

    bin_template = pick_bin_template(package.get("bin") or {}, helix_command, name)
    if bin_template is None:
        return None
    if not bin_template.startswith("cargo:"):
        print(f"  skip '{name}': cargo-kind bin template has no cargo: prefix: {bin_template!r}", file=sys.stderr)
        return None
    return {"kind": "cargo", "version": version, "crate": subject, "bin": bin_template[len("cargo:") :]}


def supported_hume_targets(package: dict) -> list[str] | None:
    """The hume targets Mason's `supported_platforms` allows, or None when the
    package is unrestricted."""
    restricted = package["source"].get("supported_platforms")
    if not restricted:
        return None
    return [
        hume_target
        for hume_target, mason_targets in MASON_TARGET_PRIORITY.items()
        if any(t in restricted for t in mason_targets)
    ]


def platforms_field(record: dict) -> str:
    platforms = record.get("platforms")
    return " (platforms {})".format(" ".join(platforms)) if platforms is not None else ""


def scheme_str_list(items) -> str:
    return " ".join(scheme_str(x) for x in items)


def assert_no_unresolved_templates(rows: list[str]) -> None:
    """Emit-time backstop for the design guarantee "the runtime sees
    literals only": abort, naming every offending row, rather than write a
    file containing a `{{...}}` template that `resolve_template` failed to
    fully resolve (its per-target skip-with-report only covers file/bin
    fields — this is the last line of defense over the whole row)."""
    offenders = [row for row in rows if "{{" in row]
    if offenders:
        sys.exit(
            f"error: unresolved {{{{ }}}} template(s) survived into emitted data "
            f"({len(offenders)} row(s)):\n" + "\n".join(offenders)
        )


def emit_lsp_sources(records: dict) -> list[str]:
    rows = []
    for name in sorted(records):
        r = records[name]
        if r["kind"] == "github":
            # build_github_record never returns a "github" record with an
            # empty targets list (it downgrades to "github-build" instead —
            # see its own trailing guard), so target_rows is always non-empty
            # here.
            target_rows = " ".join(
                "({} {} {} {})".format(
                    t, scheme_str(f), scheme_str(sha), scheme_str(b)
                )
                for t, f, sha, b in r["targets"]
            )
            row = " ({} (kind . github) (version . {}) (repo . {}) (targets {}))".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str(r["repo"]),
                target_rows,
            )
        elif r["kind"] == "generic":
            target_rows = " ".join(
                "({} {} {} {} {})".format(
                    t, scheme_str(f), scheme_str(u), scheme_str(sha), scheme_str(b)
                )
                for t, f, u, sha, b in r["targets"]
            )
            row = " ({} (kind . generic) (version . {}) (targets {}))".format(
                scheme_str(name), scheme_str(r["version"]), target_rows
            )
        elif r["kind"] == "npm":
            row = " ({} (kind . npm) (version . {}) (packages {}) (bin . {}){})".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str_list(r["packages"]),
                scheme_str(r["bin"]),
                platforms_field(r),
            )
        elif r["kind"] == "nuget":
            row = " ({} (kind . nuget) (version . {}) (package . {}) (bin . {}){})".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str(r["package"]),
                scheme_str(r["bin"]),
                platforms_field(r),
            )
        elif r["kind"] == "gem":
            row = " ({} (kind . gem) (version . {}) (packages {}) (bin . {}){})".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str_list(r["packages"]),
                scheme_str(r["bin"]),
                platforms_field(r),
            )
        elif r["kind"] == "pypi":
            row = " ({} (kind . pypi) (version . {}) (package . {}) (extras{}) (bin . {}){})".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str(r["package"]),
                "".join(" " + scheme_str(e) for e in r["extras"]),
                scheme_str(r["bin"]),
                platforms_field(r),
            )
        elif r["kind"] == "golang":
            row = " ({} (kind . golang) (version . {}) (module . {}) (bin . {}){})".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str(r["module"]),
                scheme_str(r["bin"]),
                platforms_field(r),
            )
        elif r["kind"] == "cargo":
            row = " ({} (kind . cargo) (version . {}) (crate . {}) (bin . {}){})".format(
                scheme_str(name),
                scheme_str(r["version"]),
                scheme_str(r["crate"]),
                scheme_str(r["bin"]),
                platforms_field(r),
            )
        else:
            row = " ({} (kind . {}) (version . {}))".format(
                scheme_str(name), r["kind"], scheme_str(r["version"])
            )
        rows.append(row)
    return ["("] + rows + [")"]


def main() -> None:
    no_cache = "--no-cache" in sys.argv[1:]

    tag = read_pin(MASON_PIN_SCM)
    print(f"mason-pin: {tag}", file=sys.stderr)

    if not LSP_SERVERS_SCM.exists():
        sys.exit(
            f"error: {LSP_SERVERS_SCM} does not exist — run scripts/sync-grammars.py first"
        )
    servers_data = read_sexpr(LSP_SERVERS_SCM)
    commands = {}
    for rec in servers_data:
        server_name = str(rec[0])
        command = next(f[1] for f in rec[1:] if isinstance(f, tuple) and str(f[0]) == "command")
        commands[server_name] = command
    helix_names = sorted(commands)

    mason_pkgs = fetch_registry(tag)
    mason_lsp = {p["name"]: p for p in mason_pkgs if "LSP" in p.get("categories", [])}

    # The cache is keyed by version+asset-file, so it cannot serve a stale
    # hash across a version bump — but it also means a GitHub tag re-push
    # (same version, different bytes) is never re-detected, because the
    # cached hash short-circuits the re-download that would catch it. That
    # is exactly the threat the sha256 pin exists to catch, so `--no-cache`
    # bypasses the cache entirely and re-hashes every asset — use it
    # periodically, not just on a version bump, to catch re-pushed tags.
    hash_cache = {} if no_cache else load_sha256_cache(LSP_SOURCES_SCM)
    print(
        f"sha256 cache: {len(hash_cache)} entries loaded from prior sync"
        + (" (--no-cache: re-hashing everything)" if no_cache else ""),
        file=sys.stderr,
    )

    reports = {
        "unmatched": [],
        "deprecated": [],
        "kind_census": collections.Counter(),
        "unresolved_targets": [],
        "download_failures": [],
        "missing_platform": collections.Counter(),
        "format_census": collections.Counter(),
        "no_usable_targets": [],
        "unsupported_format": [],
        "excluded": [],
    }

    records = {}
    for helix_name in helix_names:
        if helix_name in MASON_NOT_HELIX_SERVER:
            reports["excluded"].append((helix_name, MASON_NOT_HELIX_SERVER[helix_name]))
            continue
        mason_name = HELIX_TO_MASON.get(helix_name, helix_name)
        package = mason_lsp.get(mason_name)
        if package is None:
            reports["unmatched"].append(helix_name)
            continue
        if "deprecation" in package:
            reports["deprecated"].append((helix_name, mason_name, package["deprecation"]))

        purl_kind, _subject, version = parse_purl(package["source"]["id"])
        helix_command = commands[helix_name]

        if purl_kind == "github":
            record = build_github_record(helix_name, package, helix_command, reports, hash_cache)
        elif purl_kind == "generic":
            record = build_generic_record(helix_name, package, helix_command, reports, hash_cache)
        elif purl_kind == "npm":
            record = build_npm_record(helix_name, package, helix_command)
        elif purl_kind == "cargo":
            record = build_cargo_record(helix_name, package, helix_command)
        elif purl_kind == "golang":
            record = build_golang_record(helix_name, package, helix_command)
        elif purl_kind == "pypi":
            record = build_pypi_record(helix_name, package, helix_command)
        elif purl_kind == "gem":
            record = build_gem_record(helix_name, package, helix_command)
        elif purl_kind == "nuget":
            record = build_nuget_record(helix_name, package, helix_command)
        else:
            record = {"kind": purl_kind, "version": version}

        if record is None:
            continue
        platforms = supported_hume_targets(package)
        if platforms is not None and record["kind"] in PACKAGE_MANAGER_KINDS:
            record["platforms"] = platforms
        records[helix_name] = record
        reports["kind_census"][record["kind"]] += 1

    if reports["unmatched"]:
        print(f"unmatched helix servers ({len(reports['unmatched'])}):", file=sys.stderr)
        for n in reports["unmatched"]:
            print(f"  {n}", file=sys.stderr)
    if reports["excluded"]:
        print(f"excluded helix servers ({len(reports['excluded'])}):", file=sys.stderr)
        for n, why in reports["excluded"]:
            print(f"  {n}: {why}", file=sys.stderr)
    if reports["deprecated"]:
        print(f"deprecated mason packages matched ({len(reports['deprecated'])}):", file=sys.stderr)
        for helix_name, mason_name, dep in reports["deprecated"]:
            print(f"  {helix_name} -> {mason_name}: {dep}", file=sys.stderr)
    if reports["unresolved_targets"]:
        print(f"unresolved targets ({len(reports['unresolved_targets'])}):", file=sys.stderr)
        for name, target, field, template in reports["unresolved_targets"]:
            print(f"  {name} [{target}] {field}: {template!r}", file=sys.stderr)
    if reports["unsupported_format"]:
        print(f"unsupported asset formats, skipped ({len(reports['unsupported_format'])}):", file=sys.stderr)
        for name, target, asset_file in reports["unsupported_format"]:
            print(f"  {name} [{target}]: {asset_file}", file=sys.stderr)
    if reports["download_failures"]:
        print(f"download failures, skipped ({len(reports['download_failures'])}):", file=sys.stderr)
        for name, target, url in reports["download_failures"]:
            print(f"  {name} [{target}]: {url}", file=sys.stderr)
    if reports["no_usable_targets"]:
        print(
            f"github servers downgraded to stub, no usable target resolved "
            f"({len(reports['no_usable_targets'])}):",
            file=sys.stderr,
        )
        for name in reports["no_usable_targets"]:
            print(f"  {name}", file=sys.stderr)
    print(f"kind census: {dict(reports['kind_census'])}", file=sys.stderr)
    print(f"missing-platform census: {dict(reports['missing_platform'])}", file=sys.stderr)
    print(f"asset-format census: {dict(reports['format_census'])}", file=sys.stderr)
    print(f"parsed: {len(records)} installable server records", file=sys.stderr)

    for name in records:
        if name not in commands:
            sys.exit(f"error: invariant violated: emitted server '{name}' not in servers.scm")

    source_rows = emit_lsp_sources(records)
    assert_no_unresolved_templates(source_rows)

    write_generated_file(LSP_SOURCES_SCM, LSP_SOURCES_HEADER.format(tag=tag), source_rows)

    read_sexpr(LSP_SOURCES_SCM)  # self-check: emitted file must re-parse


if __name__ == "__main__":
    main()
