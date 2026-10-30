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
            langs = sync.parse_languages(doc)

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

    def test_a_registration_body_omits_the_command_args_and_config_that_are_defaults(self):
        self.assertEqual(
            sync.emit_lsp_registration("a", {"command": "aa", "args": [], "config": None}),
            ['((command . "aa"))'],
        )
        self.assertEqual(
            sync.emit_lsp_registration(
                "b", {"command": "bb", "args": ["--stdio"], "config": {"x": 1}}
            ),
            ['((command . "bb") (args "--stdio") (config . "{\\"x\\": 1}"))'],
        )
        self.assertEqual(
            sync.emit_lsp_registration("same", {"command": "same", "args": [], "config": None}),
            ["()"],
        )


def parse_langs(languages):
    with contextlib.redirect_stderr(io.StringIO()):
        return sync.parse_languages({"language": languages})


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


    def test_graphqls_block_string_delimiter_is_not_a_block_comment(self):
        (lang,) = parse_langs(
            [{"name": "graphql", "comment-token": "#", "block-comment-tokens": ['"""', '"""']}]
        )
        self.assertEqual((lang["comment_tokens"], lang["block_comment_tokens"]), (["#"], []))

    def test_another_language_keeps_the_same_pair(self):
        self.assertEqual(
            self.tokens({"block-comment-tokens": ['"""', '"""']}), ([], [('"""', '"""')])
        )

    def test_an_exclusion_its_language_no_longer_has_is_fatal(self):
        with self.assertRaises(SystemExit):
            parse_langs([{"name": "graphql", "comment-token": "#"}])


class Shebangs(unittest.TestCase):
    def shebangs(self, name, shebangs):
        (lang,) = parse_langs([{"name": name, "shebangs": shebangs}])
        return lang["shebangs"]

    def test_interpreter_names_keep_their_order(self):
        self.assertEqual(self.shebangs("x", ["python", "uv"]), ["python", "uv"])

    def test_vs_v_run_becomes_the_v_interpreter(self):
        self.assertEqual(self.shebangs("v", ["v run"]), ["v"])

    def test_a_shebang_with_a_space_is_fatal(self):
        with self.assertRaises(SystemExit):
            self.shebangs("x", ["python -u"])

    def test_a_shebang_with_a_path_separator_is_fatal(self):
        for shebang in ["/usr/bin/python", "bin\\python", ""]:
            with self.subTest(shebang=shebang), self.assertRaises(SystemExit):
                self.shebangs("x", [shebang])

    def test_a_correction_its_language_no_longer_has_is_fatal(self):
        with self.assertRaises(SystemExit):
            self.shebangs("v", ["v"])


class LanguageGrammar(unittest.TestCase):
    def test_a_language_naming_a_grammar_carries_no_grammar_field(self):
        (lang,) = parse_langs([{"name": "jsx", "grammar": "javascript"}])

        self.assertNotIn("grammar", lang)


class HelixRev(unittest.TestCase):
    SHA = "ba40e547426b0f9896c8bdc699a4ab11f2b37dbc"

    def test_reads_the_full_commit_of_a_grammars_toml(self):
        text = f'# header\nhelix-rev = "{self.SHA}"\n\n[[language]]\nname = "c"\n'

        self.assertEqual(sync.helix_rev_of(text), self.SHA)

    def test_a_missing_rev_is_fatal(self):
        with self.assertRaises(SystemExit):
            sync.helix_rev_of('[[language]]\nname = "c"\n')

    def test_an_abbreviated_rev_is_fatal(self):
        with self.assertRaises(SystemExit):
            sync.helix_rev_of('helix-rev = "ba40e547426b"\n')


def parsed_language(**overrides):
    """A parsed language with no values, plus `overrides`."""
    return {
        "name": "lang",
        "comment_tokens": [],
        "block_comment_tokens": [],
        "language_id": None,
        "roots": [],
        **overrides,
    }


class EmitLanguageData(unittest.TestCase):
    def test_every_key_in_its_fixed_order(self):
        tsx = parsed_language(
            name="tsx",
            comment_tokens=["//", "///"],
            block_comment_tokens=[("/*", "*/"), ("/**", "*/")],
            language_id="typescriptreact",
            roots=["package.json", "tsconfig.json"],
        )

        self.assertEqual(
            sync.emit_language_data(tsx),
            [
                '((comment-tokens "//" "///")',
                ' (block-comment-tokens ("/*" . "*/") ("/**" . "*/"))',
                ' (language-id . "typescriptreact")',
                ' (roots "package.json" "tsconfig.json"))',
            ],
        )

    def test_a_language_id_equal_to_the_name_is_omitted(self):
        javascript = parsed_language(
            name="javascript", comment_tokens=["//"], language_id="javascript"
        )

        self.assertEqual(sync.emit_language_data(javascript), ['((comment-tokens "//"))'])

    def test_absent_values_are_omitted(self):
        absent = parsed_language(comment_tokens=["#"])

        self.assertEqual(sync.emit_language_data(absent), ['((comment-tokens "#"))'])

    def test_a_language_with_no_values_is_the_empty_list(self):
        self.assertEqual(sync.emit_language_data(parsed_language()), ["()"])


class WriteLanguageFiles(unittest.TestCase):
    LANG = parsed_language(name="markdown.inline", block_comment_tokens=[("<!--", "-->")])

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


if __name__ == "__main__":
    unittest.main()
