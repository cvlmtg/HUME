#!/usr/bin/env python3
"""Regenerate runtime/scheme/languages.scm from helix-editor/helix languages.toml.

Reads the pinned SHA from runtime/scheme/helix-pin.scm, fetches helix's
languages.toml at that commit, and rewrites languages.scm with:
  - (register-grammar-source! …) for every [[grammar]] block
  - (define-language! …) for every [[language]] block

Idempotent: running twice produces a byte-identical file.
"""

import os
import re
import sys
import urllib.request
from pathlib import Path
from urllib.error import URLError

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        sys.exit("error: requires Python 3.11+ or 'pip install tomli'")


REPO = Path(__file__).resolve().parent.parent
HELIX_PIN_SCM = REPO / "runtime" / "scheme" / "helix-pin.scm"
LANGUAGES_SCM = REPO / "runtime" / "scheme" / "languages.scm"
GRAMMAR_SOURCES_SCM = REPO / "runtime" / "scheme" / "grammar-sources.scm"

LANGUAGES_HEADER = """\
;;; runtime/scheme/languages.scm — HUME bundled default language identities.
;;;
;;; Evaluated at startup before init.scm.  Override any entry by redefining
;;; it in init.scm — `define-language!` fully replaces the prior registration
;;; for a given name (see init.scm.example for override examples).
;;;
;;; Identity only: extensions, globs, and shebangs.  No tree-sitter grammars
;;; are shipped here — to enable highlighting, call (register-grammar! …) with
;;; paths to a compiled grammar library and highlights query (typically provided
;;; by a plugin).
;;;
;;; Grammar source metadata lives in grammar-sources.scm (loaded by a grammar-
;;; manager plugin, typically core:plum — not by hume at startup).
;;;
;;; Source: helix-editor/helix languages.toml @ {sha}
;;; Full sync: run scripts/sync-grammars.py after updating helix-pin.scm.
"""

GRAMMAR_SOURCES_HEADER = """\
;;; runtime/scheme/grammar-sources.scm — HUME bundled tree-sitter grammar source catalog.
;;;
;;; PURE DATA. One literal sexpr — a list of (name git-url rev symbol subpath)
;;; 5-tuples. All fields are fully canonicalised; no defaults are applied at
;;; read time. Read via the R7RS idiom from any plugin:
;;;
;;;   (define *grammar-sources*
;;;     (call-with-input-file
;;;       (path-join (runtime-dir) "scheme" "grammar-sources.scm")
;;;       read))
;;;
;;; Source: helix-editor/helix languages.toml @ {sha}
;;; Full sync: run scripts/sync-grammars.py after updating helix-pin.scm.
"""


def scheme_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def scheme_list(items: list[str]) -> str:
    if not items:
        return "'()"
    return "'(" + " ".join(scheme_str(x) for x in items) + ")"


def read_pin(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    body = re.sub(r";;.*", "", text)
    m = re.search(r'"([0-9a-f]+)"', body)
    if not m:
        sys.exit(f"error: could not extract SHA from {path}")
    return m.group(1)


def fetch_toml(sha: str) -> dict:
    url = f"https://raw.githubusercontent.com/helix-editor/helix/{sha}/languages.toml"
    print(f"fetching {url}", file=sys.stderr)
    try:
        with urllib.request.urlopen(url, timeout=30) as r:
            return tomllib.loads(r.read().decode())
    except URLError as e:
        sys.exit(f"error: failed to fetch {url}: {e}")


def parse_grammars(doc: dict) -> dict[str, dict]:
    """Return {name: {url, rev, subpath}} for grammars with a git source."""
    grammars: dict[str, dict] = {}
    for entry in doc.get("grammar", []):
        try:
            name = entry["name"]
            src = entry.get("source", {})
        except KeyError as e:
            sys.exit(f"error: malformed grammar entry in languages.toml: missing key {e}")
        if not src.get("git"):
            print(f"  skip grammar '{name}': no source.git", file=sys.stderr)
            continue
        try:
            grammars[name] = {
                "url": src["git"],
                "rev": src["rev"],
                "subpath": src.get("subpath", ""),
            }
        except KeyError as e:
            sys.exit(f"error: malformed grammar '{name}' source in languages.toml: missing key {e}")
    return grammars


def parse_languages(doc: dict, grammars: dict[str, dict]) -> list[dict]:
    """Return [{name, extensions, globs, shebangs, grammar_name}] for each language."""
    langs = []
    overrides = []
    no_grammar = []

    for entry in doc.get("language", []):
        name = entry["name"]
        extensions = []
        globs = []
        for ft in entry.get("file-types", []):
            if isinstance(ft, str):
                extensions.append(ft)
            elif isinstance(ft, dict):
                if "glob" in ft:
                    globs.append(ft["glob"])
                else:
                    keys = list(ft.keys())
                    print(
                        f"  skip file-type entry in '{name}': unknown keys {keys}",
                        file=sys.stderr,
                    )
        shebangs = entry.get("shebangs", [])
        grammar_name = entry.get("grammar", name)
        if grammar_name != name:
            overrides.append(f"{name} -> {grammar_name}")
        if grammar_name not in grammars:
            no_grammar.append(name)
        langs.append(
            {
                "name": name,
                "extensions": extensions,
                "globs": globs,
                "shebangs": shebangs,
                "grammar_name": grammar_name,
            }
        )

    if overrides:
        print(f"grammar overrides ({len(overrides)}):", file=sys.stderr)
        for o in overrides:
            print(f"  {o}", file=sys.stderr)
    if no_grammar:
        print(f"languages without a helix grammar ({len(no_grammar)}):", file=sys.stderr)
        for n in no_grammar:
            print(f"  {n}", file=sys.stderr)

    return langs


def emit_grammar_sources(grammars: dict[str, dict], langs: list[dict]) -> list[str]:
    # Start with all direct grammars, then add one alias entry per language that
    # delegates to a different grammar (e.g. jsx → javascript).  Each alias gets
    # the target grammar's URL/rev/subpath but the *target* grammar's symbol,
    # so the compiled .so exports the right C function.
    entries: dict[str, dict] = {}
    for gname, g in grammars.items():
        entries[gname] = {**g, "sym": "tree_sitter_" + gname.replace("-", "_")}

    for lang in langs:
        lname = lang["name"]
        gname = lang["grammar_name"]
        if gname != lname and gname in grammars and lname not in entries:
            g = grammars[gname]
            entries[lname] = {**g, "sym": "tree_sitter_" + gname.replace("-", "_")}

    subpath_entries = [(n, e["subpath"]) for n, e in entries.items() if e["subpath"]]
    if subpath_entries:
        print(f"subpath grammars ({len(subpath_entries)}):", file=sys.stderr)
        for n, sp in sorted(subpath_entries):
            print(f"  {n}: {sp}", file=sys.stderr)

    # Emit a single literal sexpr (a list of 5-tuples) with no define/provide.
    # Consumers read this via (call-with-input-file path read).
    rows = []
    for name in sorted(entries):
        e = entries[name]
        row = " ({} {} {} {} {})".format(
            scheme_str(name),
            scheme_str(e["url"]),
            scheme_str(e["rev"]),
            scheme_str(e["sym"]),
            scheme_str(e["subpath"]),
        )
        rows.append(row)
    return ["("] + rows + [")"]


def emit_language_identities(langs: list[dict]) -> list[str]:
    lines = []
    for lang in sorted(langs, key=lambda x: x["name"]):
        name_s = scheme_str(lang["name"])
        exts = lang["extensions"]
        globs = lang["globs"]
        shebangs = lang["shebangs"]

        if shebangs:
            lines.append(
                f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)} {scheme_list(shebangs)})"
            )
        elif globs:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)})")
        elif exts:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)})")
        else:
            lines.append(f"(define-language! {name_s})")
    return lines


def write_atomic(path: Path, content: str) -> None:
    tmp = path.with_suffix(".scm.tmp")
    tmp.write_text(content, encoding="utf-8")
    os.replace(tmp, path)
    print(f"wrote {path}", file=sys.stderr)


def main() -> None:
    sha = read_pin(HELIX_PIN_SCM)
    print(f"helix-pin: {sha}", file=sys.stderr)

    doc = fetch_toml(sha)
    grammars = parse_grammars(doc)
    langs = parse_languages(doc, grammars)

    print(
        f"parsed: {len(langs)} languages, {len(grammars)} direct grammars with git source",
        file=sys.stderr,
    )

    # languages.scm — identity-only
    langs_parts = [LANGUAGES_HEADER.format(sha=sha)]
    langs_parts.append("\n".join(emit_language_identities(langs)))
    langs_parts.append("")  # trailing newline
    write_atomic(LANGUAGES_SCM, "\n".join(langs_parts))

    # grammar-sources.scm — source catalog only
    src_parts = [GRAMMAR_SOURCES_HEADER.format(sha=sha)]
    src_parts.append("\n".join(emit_grammar_sources(grammars, langs)))
    src_parts.append("")  # trailing newline
    write_atomic(GRAMMAR_SOURCES_SCM, "\n".join(src_parts))


if __name__ == "__main__":
    main()
