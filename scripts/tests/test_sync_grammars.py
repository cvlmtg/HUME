"""Tests for the parsing and emission in scripts/sync-grammars.py."""

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
_spec = importlib.util.spec_from_file_location("sync_grammars", SCRIPTS / "sync-grammars.py")
sync = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sync)

# Helix's `language-servers` feature names, written out from helix's
# `LanguageServerFeature` enum, independent of the script's own constant.
HELIX_FEATURES = {
    "format", "goto-declaration", "goto-definition", "goto-type-definition",
    "goto-reference", "goto-implementation", "signature-help", "hover",
    "document-highlight", "completion", "code-action", "document-links",
    "workspace-command", "document-symbols", "workspace-symbols", "diagnostics",
    "pull-diagnostics", "rename-symbol", "inlay-hints", "document-colors",
    "call-hierarchy",
}


def language(name, servers, roots=()):
    return {"name": name, "language-servers": servers, "roots": list(roots)}


def server_table(*names):
    return {name: {"command": name, "args": []} for name in names}


def parse(doc):
    with contextlib.redirect_stderr(io.StringIO()):
        return sync.parse_language_servers(doc)


class ParseLanguageServers(unittest.TestCase):
    def test_every_listed_server_is_kept_in_order(self):
        doc = {
            "language": [language("python", ["ty", "ruff", "pylsp"], ["pyproject.toml"])],
            "language-server": server_table("ty", "ruff", "pylsp"),
        }

        _, lists = parse(doc)

        self.assertEqual(lists["python"], [("ty", None), ("ruff", None), ("pylsp", None)])

    def test_a_language_list_carries_no_roots(self):
        doc = {
            "language": [language("typescript", ["tsls"], ["tsconfig.json"])],
            "language-server": server_table("tsls"),
        }

        servers, lists = parse(doc)

        self.assertEqual(lists["typescript"], [("tsls", None)])
        self.assertEqual(set(servers["tsls"]), {"command", "args", "config"})

    def test_inline_table_filters_are_carried_through(self):
        doc = {
            "language": [
                language(
                    "gjs",
                    [
                        {"name": "tsls", "except-features": ["format", "diagnostics"]},
                        {"name": "ember", "only-features": ["completion"]},
                    ],
                )
            ],
            "language-server": server_table("tsls", "ember"),
        }

        _, lists = parse(doc)

        self.assertEqual(
            lists["gjs"],
            [
                ("tsls", ("except", ["format", "diagnostics"])),
                ("ember", ("only", ["completion"])),
            ],
        )

    def test_both_filters_on_one_entry_are_fatal(self):
        doc = {
            "language": [
                language(
                    "gjs",
                    [{"name": "tsls", "only-features": ["hover"], "except-features": ["format"]}],
                )
            ],
            "language-server": server_table("tsls"),
        }

        with self.assertRaises(SystemExit):
            parse(doc)

    def test_a_server_with_no_command_table_is_dropped_from_the_lists_and_reported(self):
        doc = {
            "language": [language("haxe", ["haxe-language-server", "other"])],
            "language-server": server_table("other"),
        }
        stderr = io.StringIO()

        with contextlib.redirect_stderr(stderr):
            servers, lists = sync.parse_language_servers(doc)

        self.assertEqual(lists["haxe"], [("other", None)])
        self.assertNotIn("haxe-language-server", servers)
        self.assertIn("haxe-language-server", stderr.getvalue())


class ParseLanguages(unittest.TestCase):
    def test_roots_are_carried_per_language(self):
        doc = {
            "language": [
                {"name": "go", "file-types": ["go"], "roots": ["go.work", "go.mod"]},
                {"name": "gomod", "file-types": ["mod"]},
            ]
        }

        with contextlib.redirect_stderr(io.StringIO()):
            langs = sync.parse_languages(doc, {})

        self.assertEqual([lang["roots"] for lang in langs], [["go.work", "go.mod"], []])


class CheckInvariants(unittest.TestCase):
    LANGS = [{"name": "python"}]
    SERVERS = {"ty": {}, "ruff": {}}

    def check(self, servers_by_lang):
        sync.check_lsp_invariants(self.SERVERS, self.LANGS, servers_by_lang)

    def test_accepts_two_servers_for_one_language(self):
        self.check({"python": [("ty", None), ("ruff", ("only", ["format"]))]})

    def test_rejects_an_unknown_feature_name(self):
        with self.assertRaises(SystemExit):
            self.check({"python": [("ty", ("only", ["formatting"]))]})

    def test_rejects_a_duplicate_server_in_one_list(self):
        with self.assertRaises(SystemExit):
            self.check({"python": [("ty", None), ("ty", None)]})

    def test_rejects_a_listed_server_that_is_not_seeded(self):
        with self.assertRaises(SystemExit):
            self.check({"python": [("ty", None), ("ghost", None)]})

    def test_rejects_a_list_for_an_unknown_language(self):
        with self.assertRaises(SystemExit):
            self.check({"cobol": [("ty", None)]})


class Emit(unittest.TestCase):
    def test_language_servers_rows(self):
        rows = sync.emit_language_servers(
            {
                "python": [("ty", None), ("ruff", ("only", ["format", "diagnostics"]))],
                "gjs": [("tsls", ("except", ["format"]))],
            }
        )

        self.assertEqual(
            rows,
            [
                "(",
                ' ("gjs" (servers ("tsls" (except-features "format"))))',
                ' ("python" (servers ("ty") ("ruff" (only-features "format" "diagnostics"))))',
                ")",
            ],
        )

    def test_identity_rows_carry_detection_data_only(self):
        base = {"extensions": ["go"], "globs": [], "shebangs": []}
        rows = sync.emit_language_identities(
            [
                {"name": "go", **base, "roots": ["go.work", "go.mod"], "language_id": None},
                {"name": "tsx", **base, "roots": ["package.json"], "language_id": "typescriptreact"},
                {"name": "sh", "extensions": [], "globs": ["*.sh"], "shebangs": ["bash"]},
            ]
        )

        self.assertEqual(
            rows,
            [
                """(define-language! "go" '("go"))""",
                """(define-language! "sh" '() '("*.sh") '("bash"))""",
                """(define-language! "tsx" '("go"))""",
            ],
        )

    def test_servers_rows_omit_the_command_args_and_config_that_are_defaults(self):
        rows = sync.emit_lsp_servers(
            {
                "b": {"command": "bb", "args": ["--stdio"], "config": {"x": 1}},
                "a": {"command": "aa", "args": [], "config": None},
                "same": {"command": "same", "args": [], "config": None},
            }
        )

        self.assertEqual(
            rows,
            [
                "(",
                ' ("a" (command . "aa"))',
                ' ("b" (command . "bb") (args "--stdio") (config . "{\\"x\\": 1}"))',
                ' ("same")',
                ")",
            ],
        )

    def test_the_feature_vocabulary_is_helixs(self):
        self.assertEqual(set(sync.LSP_FEATURES), HELIX_FEATURES)
        self.assertEqual(len(sync.LSP_FEATURES), len(HELIX_FEATURES))


def parse_langs(languages, grammars=None):
    with contextlib.redirect_stderr(io.StringIO()):
        return sync.parse_languages({"language": languages}, grammars or {})


class CommentTokens(unittest.TestCase):
    def tokens(self, entry):
        (lang,) = parse_langs([{"name": "x", **entry}])
        return lang["comment_tokens"], lang["block_comment_tokens"]

    def test_a_single_line_token_becomes_a_one_element_list(self):
        self.assertEqual(self.tokens({"comment-token": "#"}), (["#"], []))

    def test_a_bare_string_under_the_plural_key_becomes_a_one_element_list(self):
        self.assertEqual(self.tokens({"comment-tokens": "--"}), (["--"], []))

    def test_a_token_list_keeps_its_order(self):
        self.assertEqual(
            self.tokens({"comment-tokens": ["//", "///", "//!"]}), (["//", "///", "//!"], [])
        )

    def test_a_block_table_becomes_one_pair(self):
        self.assertEqual(
            self.tokens({"block-comment-tokens": {"start": "<!--", "end": "-->"}}),
            ([], [("<!--", "-->")]),
        )

    def test_a_list_of_block_tables_keeps_its_order(self):
        self.assertEqual(
            self.tokens(
                {
                    "block-comment-tokens": [
                        {"start": "/*", "end": "*/"},
                        {"start": "/**", "end": "*/"},
                    ]
                }
            ),
            ([], [("/*", "*/"), ("/**", "*/")]),
        )

    def test_a_positional_start_end_pair_becomes_one_pair(self):
        self.assertEqual(
            self.tokens({"block-comment-tokens": ["#-", "-#"]}), ([], [("#-", "-#")])
        )

    def test_no_tokens_gives_empty_lists(self):
        self.assertEqual(self.tokens({}), ([], []))

    def test_a_line_token_that_is_not_a_string_is_fatal(self):
        with self.assertRaises(SystemExit):
            self.tokens({"comment-token": 3})

    def test_a_block_table_without_an_end_is_fatal(self):
        with self.assertRaises(SystemExit):
            self.tokens({"block-comment-tokens": {"start": "/*"}})

    def test_a_three_string_block_list_is_fatal(self):
        with self.assertRaises(SystemExit):
            self.tokens({"block-comment-tokens": ["/*", "*/", "x"]})


class LanguageGrammar(unittest.TestCase):
    GRAMMARS = {
        "javascript": {"url": "https://js", "rev": "r1", "subpath": ""},
        "markdown_inline": {"url": "https://md", "rev": "r2", "subpath": "tree-sitter-markdown-inline"},
    }

    def grammar_of(self, entry):
        (lang,) = parse_langs([entry], self.GRAMMARS)
        return lang["grammar"]

    def test_a_language_uses_the_grammar_of_its_own_name(self):
        self.assertEqual(
            self.grammar_of({"name": "javascript"}),
            {"url": "https://js", "rev": "r1", "sym": "tree_sitter_javascript", "subpath": ""},
        )

    def test_a_language_naming_another_grammar_gets_that_grammars_source_and_symbol(self):
        self.assertEqual(
            self.grammar_of({"name": "jsx", "grammar": "javascript"}),
            {"url": "https://js", "rev": "r1", "sym": "tree_sitter_javascript", "subpath": ""},
        )

    def test_the_symbol_replaces_dashes_in_the_grammar_name(self):
        grammars = {"glimmer-javascript": {"url": "u", "rev": "r", "subpath": ""}}
        (lang,) = parse_langs([{"name": "gjs", "grammar": "glimmer-javascript"}], grammars)

        self.assertEqual(lang["grammar"]["sym"], "tree_sitter_glimmer_javascript")

    def test_a_language_with_no_git_grammar_has_none(self):
        self.assertIsNone(self.grammar_of({"name": "cabal"}))


class LanguageNames(unittest.TestCase):
    def test_an_uppercase_name_is_fatal(self):
        langs = parse_langs([{"name": "Nix"}])

        with self.assertRaises(SystemExit):
            sync.check_language_names(langs)

    def test_a_drive_relative_name_is_fatal(self):
        langs = parse_langs([{"name": "C:x"}])

        with self.assertRaises(SystemExit):
            sync.check_language_names(langs)

    def test_a_name_with_a_path_separator_is_fatal(self):
        langs = parse_langs([{"name": "a/b"}])

        with self.assertRaises(SystemExit):
            sync.check_language_names(langs)

    def test_dotted_names_are_accepted(self):
        sync.check_language_names(parse_langs([{"name": "markdown.inline"}, {"name": "robots.txt"}]))


class EmitLanguageData(unittest.TestCase):
    def test_every_key_in_its_fixed_order(self):
        lang = {
            "name": "tsx",
            "comment_tokens": ["//", "///"],
            "block_comment_tokens": [("/*", "*/"), ("/**", "*/")],
            "language_id": "typescriptreact",
            "roots": ["package.json", "tsconfig.json"],
            "grammar": {"url": "https://ts", "rev": "r", "sym": "tree_sitter_tsx", "subpath": "tsx"},
        }

        self.assertEqual(
            sync.emit_language_data(lang),
            [
                '((comment-tokens "//" "///")',
                ' (block-comment-tokens ("/*" . "*/") ("/**" . "*/"))',
                ' (language-id . "typescriptreact")',
                ' (roots "package.json" "tsconfig.json")',
                ' (grammar "https://ts" "r" "tree_sitter_tsx" "tsx"))',
            ],
        )

    def test_a_language_id_equal_to_the_name_is_omitted(self):
        lang = {
            "name": "javascript",
            "comment_tokens": ["//"],
            "block_comment_tokens": [],
            "language_id": "javascript",
            "roots": [],
            "grammar": None,
        }

        self.assertEqual(sync.emit_language_data(lang), ['((comment-tokens "//"))'])

    def test_absent_values_are_omitted(self):
        lang = {
            "comment_tokens": ["#"],
            "block_comment_tokens": [],
            "language_id": None,
            "roots": [],
            "grammar": None,
        }

        self.assertEqual(sync.emit_language_data(lang), ['((comment-tokens "#"))'])

    def test_a_language_with_no_values_is_the_empty_list(self):
        lang = {
            "comment_tokens": [],
            "block_comment_tokens": [],
            "language_id": None,
            "roots": [],
            "grammar": None,
        }

        self.assertEqual(sync.emit_language_data(lang), ["()"])


class WriteLanguageFiles(unittest.TestCase):
    LANG = {
        "name": "markdown.inline",
        "comment_tokens": [],
        "block_comment_tokens": [("<!--", "-->")],
        "language_id": None,
        "roots": [],
        "grammar": None,
    }

    def write(self, directory):
        with contextlib.redirect_stderr(io.StringIO()):
            sync.write_language_files(directory, "abc123", [self.LANG])

    def test_writes_one_file_per_language_with_its_header(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / "languages"

            self.write(directory)

            text = (directory / "markdown.inline.scm").read_text(encoding="utf-8")
        self.assertEqual(
            text,
            ";;; runtime/scheme/languages/markdown.inline.scm — HUME bundled language data.\n"
            ";;; Generated — do not hand-edit. Record format: README.md, parent directory.\n"
            ";;; Source: helix-editor/helix languages.toml @ abc123\n"
            "\n"
            '((block-comment-tokens ("<!--" . "-->")))\n',
        )

    def test_removes_a_file_for_a_language_that_is_gone(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "gone.scm").write_text("()\n", encoding="utf-8")

            self.write(directory)

            self.assertEqual(sorted(p.name for p in directory.iterdir()), ["markdown.inline.scm"])

    def test_a_dotfile_is_left_alone(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / ".DS_Store").write_bytes(b"finder")

            self.write(directory)

            self.assertEqual(
                sorted(p.name for p in directory.iterdir()), [".DS_Store", "markdown.inline.scm"]
            )

    def test_a_file_that_is_not_scm_is_fatal(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "notes.txt").write_text("hand-written\n", encoding="utf-8")

            with self.assertRaises(SystemExit):
                self.write(directory)


if __name__ == "__main__":
    unittest.main()
