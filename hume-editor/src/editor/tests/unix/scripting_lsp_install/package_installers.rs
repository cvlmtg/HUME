//! The golang, pypi, gem and nuget installers, and per-platform restriction.

use super::*;

// ── golang installer (fake shim) ────────────────────────────────────────────────

#[test]
fn lsp_install_golang_runs_go_install_into_gobin_and_registers() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("go", &args_file, &go_layout("gopls"));

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install go");

    let argv: Vec<String> = std::fs::read_to_string(&args_file)
        .expect("shim must have run and recorded argv")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(argv[..2], ["install", "--"], "argv: {argv:?}");
    assert!(
        argv[2].starts_with("golang.org/x/tools/gopls@v"),
        "final argv token must be the module@version spec: {argv:?}"
    );

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("gopls");
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("gopls")
        .expect("go must be registered after a successful go install");
    assert_eq!(
        Path::new(&cmd),
        server_dir.join("bin").join("gopls"),
        "GOBIN must point at the server dir's bin/"
    );
}

#[test]
fn lsp_install_golang_missing_binary_after_install_fails_loudly() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("go", &args_file, "");

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install go");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after go install"),
        "a go install that produces no binary must fail loudly: {log}"
    );
    assert!(
        ed.state.lsp.registered_command_for_test("gopls").is_none(),
        "go must not be registered when the install failed"
    );
}

#[test]
fn lsp_install_golang_requires_go_on_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    type_cmd(&mut ed, ":lsp-install go");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'go' on $PATH"),
        "a golang-kind install must name the missing toolchain: {log}"
    );
}

// ── pypi installer (fake python3, fake venv python) ─────────────────────────────

#[test]
fn lsp_install_pypi_creates_a_venv_pip_installs_and_registers() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", true));

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install python");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("ty");
    assert_eq!(
        read_lines(&venv_args),
        ["-m", "venv", server_dir.join("venv").to_str().unwrap()]
    );
    let pip = read_lines(&pip_args);
    assert_eq!(
        pip[..5],
        ["-m", "pip", "install", "--disable-pip-version-check", "--"],
        "pip argv: {pip:?}"
    );
    assert!(
        pip[5].starts_with("ty=="),
        "final token must be pkg==version: {pip:?}"
    );

    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("ty")
        .expect("python must be registered after a successful pip install");
    assert_eq!(Path::new(&cmd), server_dir.join("venv/bin/ty"));
}

#[test]
fn lsp_install_pypi_passes_extras_in_the_requirement() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim(
        "python3",
        &venv_args,
        &venv_layout(&pip_args, "pylsp", true),
    );

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install snakemake");

    let pip = read_lines(&pip_args);
    assert!(
        pip[5].starts_with("python-lsp-server[all]=="),
        "an `extra=all` purl qualifier must reach pip as [all]: {pip:?}"
    );
}

#[test]
fn lsp_install_pypi_missing_binary_after_install_fails_loudly() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir =
        write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", false));

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install python");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after pip install"),
        "a pip install that produces no binary must fail loudly: {log}"
    );
    assert!(ed.state.lsp.registered_command_for_test("ty").is_none());
}

#[test]
fn lsp_install_pypi_requires_python3_on_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    type_cmd(&mut ed, ":lsp-install python");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'python3' on $PATH"),
        "a pypi-kind install must name the missing interpreter: {log}"
    );
}

// ── per-platform restriction ────────────────────────────────────────────────────

#[test]
fn lsp_install_refuses_a_server_restricted_to_other_platforms() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    let _runtime = load_lsp_with_sources(
        &mut ed,
        &[(
            "ty".to_string(),
            "((kind . pypi) (version . \"1.0.0\") (package . \"ty\") (extras) (bin . \"ty\") \
             (platforms windows-x64))"
                .to_string(),
        )],
        data_tmp.path(),
    );

    let args_tmp = tempfile::tempdir().unwrap();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", true));
    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install python");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("not supported on this platform"),
        "a platform-restricted server must refuse loudly: {log}"
    );
    assert!(
        !venv_args.exists(),
        "the toolchain must not run for a refused install"
    );
}

#[test]
fn lsp_install_allows_a_server_whose_platform_list_includes_this_one() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    let _runtime = load_lsp_with_sources(
        &mut ed,
        &[(
            "ty".to_string(),
            "((kind . pypi) (version . \"1.0.0\") (package . \"ty\") (extras) (bin . \"ty\") \
             (platforms darwin-arm64 darwin-x64 linux-x64))"
                .to_string(),
        )],
        data_tmp.path(),
    );

    let args_tmp = tempfile::tempdir().unwrap();
    let venv_args = args_tmp.path().join("venv-argv.txt");
    let pip_args = args_tmp.path().join("pip-argv.txt");
    let shim_dir = write_fake_tool_shim("python3", &venv_args, &venv_layout(&pip_args, "ty", true));
    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install python");

    assert!(
        ed.state.lsp.registered_command_for_test("ty").is_some(),
        "an install on a listed platform must register"
    );
}

// ── gem installer (fake shim) ───────────────────────────────────────────────────

/// Layout of a real `gem install --bindir <dir>`: `<dir>/<bin_name>`.
fn gem_layout(bin_name: &str) -> String {
    format!(
        "bindir=\"\"; prev=\"\"\n\
         for a in \"$@\"; do [ \"$prev\" = \"--bindir\" ] && bindir=\"$a\"; prev=\"$a\"; done\n\
         mkdir -p \"$bindir\"\n\
         printf '#!/bin/sh\\n' > \"$bindir/{bin_name}\"\n\
         chmod +x \"$bindir/{bin_name}\""
    )
}

#[test]
fn lsp_install_gem_installs_into_the_server_dir_and_registers_gem_env() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("gem", &args_file, &gem_layout("ruby-lsp"));

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install ruby");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("ruby-lsp");
    let argv = read_lines(&args_file);
    assert_eq!(
        argv[..6],
        [
            "install",
            "--no-document",
            "--install-dir",
            server_dir.to_str().unwrap(),
            "--bindir",
            server_dir.join("bin").to_str().unwrap(),
        ],
        "argv: {argv:?}"
    );
    assert!(
        argv[6].starts_with("ruby-lsp:"),
        "the main package must be pinned as name:version: {argv:?}"
    );
    assert_eq!(argv[7], "ruby-lsp-rails", "extra packages follow: {argv:?}");

    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("ruby-lsp")
        .expect("ruby must be registered after a successful gem install");
    assert_eq!(Path::new(&cmd), server_dir.join("bin").join("ruby-lsp"));

    let env = ed
        .state
        .lsp
        .registered_env_for_test("ruby-lsp")
        .expect("ruby is registered");
    for key in ["GEM_HOME", "GEM_PATH"] {
        let value = env
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("{key} must be registered for the server: {env:?}"));
        assert_eq!(
            Path::new(&value.1),
            server_dir,
            "{key} must be the server dir"
        );
    }
}

#[test]
fn lsp_install_gem_missing_binary_after_install_fails_loudly() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("gem", &args_file, "");

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install ruby");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after gem install"),
        "a gem install that produces no binary must fail loudly: {log}"
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("ruby-lsp")
            .is_none()
    );
}

#[test]
fn lsp_install_gem_requires_gem_on_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    type_cmd(&mut ed, ":lsp-install ruby");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'gem' on $PATH"),
        "a gem-kind install must name the missing toolchain: {log}"
    );
}

// ── nuget installer (fake shim) ─────────────────────────────────────────────────

/// Layout of a real `dotnet tool install --tool-path <dir>`: `<dir>/<bin_name>`.
fn dotnet_layout(bin_name: &str) -> String {
    format!(
        "tp=\"\"; prev=\"\"\n\
         for a in \"$@\"; do [ \"$prev\" = \"--tool-path\" ] && tp=\"$a\"; prev=\"$a\"; done\n\
         mkdir -p \"$tp\"\n\
         printf '#!/bin/sh\\n' > \"$tp/{bin_name}\"\n\
         chmod +x \"$tp/{bin_name}\""
    )
}

#[test]
fn lsp_install_nuget_installs_a_dotnet_tool_and_registers() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim(
        "dotnet",
        &args_file,
        &dotnet_layout("roslyn-language-server"),
    );

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install c-sharp");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("roslyn-language-server");
    let argv = read_lines(&args_file);
    assert_eq!(
        argv[..5],
        [
            "tool",
            "install",
            "roslyn-language-server",
            "--tool-path",
            server_dir.join("bin").to_str().unwrap(),
        ],
        "argv: {argv:?}"
    );
    assert_eq!(argv[5], "--version", "argv: {argv:?}");
    assert!(
        !argv[6].is_empty(),
        "the seeded version must follow --version: {argv:?}"
    );

    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("roslyn-language-server")
        .expect("c-sharp must be registered after a successful dotnet tool install");
    assert_eq!(
        Path::new(&cmd),
        server_dir.join("bin").join("roslyn-language-server")
    );
}

#[test]
fn lsp_install_nuget_missing_binary_after_install_fails_loudly() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let args_tmp = tempfile::tempdir().unwrap();
    let args_file = args_tmp.path().join("argv.txt");
    let shim_dir = write_fake_tool_shim("dotnet", &args_file, "");

    ed.state.dirs.search_path = Some(shim_dir.path().into());
    type_cmd(&mut ed, ":lsp-install c-sharp");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary not found after dotnet install"),
        "a dotnet tool install that produces no binary must fail loudly: {log}"
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("roslyn-language-server")
            .is_none()
    );
}

#[test]
fn lsp_install_nuget_requires_dotnet_on_path() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());

    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());
    type_cmd(&mut ed, ":lsp-install c-sharp");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("requires 'dotnet' on $PATH"),
        "a nuget-kind install must name the missing toolchain: {log}"
    );
}
