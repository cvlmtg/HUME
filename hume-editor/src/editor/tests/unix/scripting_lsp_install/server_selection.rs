//! Default server order, user `language-servers`, and installing by server name.

use super::*;

/// The servers `language` uses, in order.
fn effective(ed: &Editor, language: &str) -> Vec<String> {
    crate::editor::lsp::introspect::language_servers(&ed.state.lsp, language)
        .into_iter()
        .map(|entry| entry.name.to_string())
        .collect()
}

#[test]
fn scan_registers_secondary_and_sets_default_order() {
    let data_tmp = tempfile::tempdir().unwrap();
    // Fabricated ruff before ty, the reverse of Helix's order, so the
    // result shows the catalog order, not the install order.
    fabricate_server(data_tmp.path(), "ruff", "0.0.1", "ruff");
    fabricate_server(data_tmp.path(), "ty", "0.0.1", "ty");
    fabricate_server(
        data_tmp.path(),
        "typescript-language-server",
        "0.0.1",
        "typescript-language-server",
    );

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install_eager(&mut ed, data_tmp.path());

    assert_eq!(
        effective(&ed, "python"),
        ["ty", "ruff"],
        "Helix lists python as ty, ruff, jedi, pylsp, zuban: installed ones in that order"
    );
    let gjs = crate::editor::lsp::introspect::language_servers(&ed.state.lsp, "gjs");
    let entry = &gjs[0];
    assert_eq!(entry.name.as_str(), "typescript-language-server");
    assert!(!entry.filter.admits(hume_scripting::LspFeature::Format));
    assert!(!entry.filter.admits(hume_scripting::LspFeature::Diagnostics));
    assert!(entry.filter.admits(hume_scripting::LspFeature::Hover));
}

#[test]
fn user_set_language_servers_overrides_install_default() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "ruff", "0.0.1", "ruff");
    fabricate_server(data_tmp.path(), "ty", "0.0.1", "ty");

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)\n\
         (set-language-servers! \"python\" '(\"ruff\"))",
    );

    assert_eq!(effective(&ed, "python"), ["ruff"]);
}

#[test]
fn a_manual_registration_no_list_names_does_not_attach() {
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "ruff", "0.0.1", "ruff");
    fabricate_server(data_tmp.path(), "ty", "0.0.1", "ty");

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (register-lsp-server! \"mypy\" #:command \"mypy-ls\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)",
    );

    assert_eq!(effective(&ed, "python"), ["ty", "ruff"]);
}

/// A user list may name any registered server for any language, including a
/// language the catalog does not list the server under.
#[test]
fn a_user_list_attaches_a_server_its_catalog_row_does_not_list() {
    let data_tmp = tempfile::tempdir().unwrap();
    for server in [
        "typescript-language-server",
        "vscode-eslint-language-server",
    ] {
        fabricate_server(data_tmp.path(), server, "0.0.1", server);
    }

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)\n\
         (set-language-servers! \"jsx\" '(\"typescript-language-server\" \"vscode-eslint-language-server\"))",
    );

    assert_eq!(
        effective(&ed, "jsx"),
        [
            "typescript-language-server",
            "vscode-eslint-language-server"
        ]
    );
    let warnings: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .map(|e| e.text.as_str())
        .collect();
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Every catalog language gets its default list at load, with no server
/// installed: a server registered by hand is then picked up by name.
#[test]
fn default_lists_cover_every_catalog_language_with_nothing_installed() {
    let data_tmp = tempfile::tempdir().unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_with_init(
        &mut ed,
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n\
         (register-lsp-server! \"typescript-language-server\" #:command \"my-tsls\")\n\
         (load-plugin! \"core:lsp-install\") (%activate-plugin-inline! \"core:lsp-install\" #f)",
    );

    assert_eq!(effective(&ed, "jsx"), ["typescript-language-server"]);
}

#[test]
fn install_by_server_name_runs_the_same_pipeline() {
    let data_tmp = tempfile::tempdir().unwrap();
    let version = seeded_version("ruff");
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());
    fabricate_server(data_tmp.path(), "ruff", &version, "ruff");

    type_cmd(&mut ed, ":lsp-install ruff");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(format!("LSP: ruff already installed (v{version}) — up to date").as_str())
    );
    assert!(ed.state.lsp.registered_command_for_test("ruff").is_some());
    assert_eq!(effective(&ed, "python"), ["ruff"]);
}

#[test]
fn install_rejects_a_name_that_is_neither_language_nor_server() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-install nosuch");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp-install: no language or server named \"nosuch\" is seeded"),
        "{log}"
    );
}

/// `:lsp-install <name>` reads a name that is both a language and a server as
/// the server, so such a name must be its own language's primary.
#[test]
fn a_name_that_is_both_language_and_server_is_its_own_primary() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/plugins/core/lsp-install/data");
    let languages = std::fs::read_to_string(dir.join("language-servers.scm")).unwrap();
    for row in languages.lines().filter(|l| l.starts_with(" (\"")) {
        let language = row[3..].split('"').next().unwrap();
        if !dir
            .join("registrations")
            .join(format!("{language}.scm"))
            .exists()
        {
            continue;
        }
        let primary = row
            .split("(servers (\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(
            primary, language,
            "{language} is a server name but its language's primary is {primary}"
        );
    }
}

/// The Python sync's feature vocabulary is the one HUME routes by.
#[test]
fn sync_script_feature_vocabulary_matches_lsp_feature_all() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/sync-grammars.py");
    let src = std::fs::read_to_string(path).unwrap();
    let block = &src[src.find("LSP_FEATURES = (").unwrap()..];
    let block = &block[..block.find("\n)\n").unwrap()];
    let mut script: Vec<&str> = block
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"')?.strip_suffix("\","))
        .collect();
    let mut hume: Vec<&str> = hume_scripting::LspFeature::ALL
        .iter()
        .map(|f| f.name())
        .collect();
    script.sort_unstable();
    hume.sort_unstable();
    assert_eq!(script, hume);
}
