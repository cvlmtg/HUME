// Editor-level tests for core:lsp-install's server install pipeline:
// scan-on-load registration, :lsp-install/:lsp-uninstall/:lsp-catalog,
// receipts, orphan warnings, and the on-language-set discovery hint.
//
// Fixture servers, chosen from the real core:lsp-install
// registrations/ and sources/ data:
//   rust-analyzer (language "rust"): github, plain .gz, installable
//   svlangserver (language "systemverilog"): npm, settings contain a real
//     JSON array (systemverilog.includeIndexing)
//   gopls (language "go"): golang, installable through `go install`
//   pest-language-server (language "pest"): cargo, crates.io semver,
//     installable
//   nil (language "nix"): cargo-git, a Mason git-tag pin, a stub: not
//     installable
//   ocamllsp (language "ocaml"): opam, a stub: not installable

use std::path::{Path, PathBuf};

use super::*;
use hume_core::Severity;

/// `<data_dir>/servers/`, where `core:lsp-install` installs every server.
fn servers_dir(data_dir_root: &Path) -> PathBuf {
    canonical_data_dir(data_dir_root).join("servers")
}

/// `<data_dir>/servers/<name>/`, which holds server `name`'s install folders.
fn server_dir(data_dir_root: &Path, name: &str) -> PathBuf {
    servers_dir(data_dir_root).join(name)
}

/// The numbered install folders in `servers/<name>/`, oldest first.
fn install_folders(data_dir_root: &Path, name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(server_dir(data_dir_root, name)) else {
        return Vec::new();
    };
    let mut numbered: Vec<(u64, PathBuf)> = entries
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let number = path.file_name()?.to_str()?.parse().ok()?;
            Some((number, path))
        })
        .collect();
    numbered.sort();
    numbered.into_iter().map(|(_, path)| path).collect()
}

/// `name`'s current install: its newest numbered folder holding a receipt.
fn install_dir(data_dir_root: &Path, name: &str) -> PathBuf {
    install_folders(data_dir_root, name)
        .into_iter()
        .rev()
        .find(|folder| folder.join("receipt.scm").exists())
        .unwrap_or_else(|| panic!("{name} has no install folder with a receipt"))
}

/// Asserts that `servers/<name>/` holds nothing but `name`'s current install.
fn assert_only_install(data_dir_root: &Path, name: &str) {
    let entries: Vec<PathBuf> = std::fs::read_dir(server_dir(data_dir_root, name))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries, [install_dir(data_dir_root, name)]);
}

/// Write a receipt + a dummy binary file for `name` directly into the install
/// folder `<data_dir>/servers/<name>/1/`, matching what
/// `lsp-install/install-server!` would produce, for scan-time tests that don't
/// need a real network install.
fn fabricate_server(data_dir_root: &Path, name: &str, version: &str, bin: &str) {
    let dir = server_dir(data_dir_root, name).join("1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("receipt.scm"),
        format!(r#"((name . "{name}") (version . "{version}") (bin . "{bin}"))"#),
    )
    .unwrap();
    std::fs::write(dir.join(bin), b"#!/bin/sh\n").unwrap();
}

/// The folder an install of `name` that was stopped before its receipt
/// leaves: numbered after every other install of `name`, with a partial file
/// and no receipt.
fn stop_an_install(data_dir_root: &Path, name: &str) -> PathBuf {
    let next = install_folders(data_dir_root, name)
        .last()
        .map_or(1, |last| {
            last.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                + 1
        });
    let folder = server_dir(data_dir_root, name).join(next.to_string());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("partial"), b"").unwrap();
    folder
}

/// Holds `servers/.install-lock` as another HUME's install would, until the
/// returned file is dropped.
fn hold_install_lock(data_dir_root: &Path) -> std::fs::File {
    let servers_dir = servers_dir(data_dir_root);
    std::fs::create_dir_all(&servers_dir).unwrap();
    let lock = std::fs::File::create(servers_dir.join(".install-lock")).unwrap();
    lock.try_lock().unwrap();
    lock
}

/// Asserts that `name`'s receipt records `version` and that `name` is
/// registered with `bin` in its install directory as its command.
fn assert_installed(data_root: &Path, ed: &Editor, name: &str, version: &str, bin: &str) {
    let log = ed.state.message_log.format_for_display();
    let dir = install_dir(data_root, name);
    let receipt = std::fs::read_to_string(dir.join("receipt.scm"))
        .unwrap_or_else(|e| panic!("{name} has no receipt: {e}\n{log}"));
    assert!(
        receipt.contains(&format!("(version . \"{version}\")")),
        "{receipt}\n{log}"
    );
    assert_eq!(
        ed.state.lsp.registered_command_for_test(name).as_deref(),
        dir.join(bin).to_str(),
        "{log}"
    );
}

/// One `sources/<name>.scm` fixture: the server's name and its record.
type SourceFixture = (String, String);

/// The version `sources/<server>.scm` seeds, read from the file so the test
/// follows a re-pin.
fn seeded_version(server: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runtime/plugins/core/lsp-install/data/sources")
        .join(format!("{server}.scm"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{server} has no sources file: {e}"));
    let after = &text[text.find("(version . \"").unwrap() + "(version . \"".len()..];
    after[..after.find('"').unwrap()].to_string()
}

/// Copy of the repo's `runtime/` whose `core:lsp-install` `sources/` holds only
/// `sources`, with its requirements regenerated from them.
fn runtime_with_sources(sources: &[SourceFixture]) -> tempfile::TempDir {
    let runtime = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("cp")
        .arg("-R")
        .arg(format!("{}/.", repo_runtime_dir().display()))
        .arg(runtime.path())
        .status()
        .expect("spawn cp");
    assert!(status.success());
    let data = runtime.path().join("plugins/core/lsp-install/data");
    let sources_dir = data.join("sources");
    std::fs::remove_dir_all(&sources_dir).unwrap();
    std::fs::create_dir(&sources_dir).unwrap();
    for (name, body) in sources {
        std::fs::write(sources_dir.join(format!("{name}.scm")), body).unwrap();
    }
    let status = std::process::Command::new("python3")
        .arg(repo_runtime_dir().join("../scripts/sync-lsp-sources.py"))
        .arg("--requirements-only")
        .arg("--sources")
        .arg(&sources_dir)
        .arg("--out")
        .arg(data.join("requirements"))
        .status()
        .expect("spawn python3");
    assert!(
        status.success(),
        "requirements must regenerate from the fixture's sources"
    );
    runtime
}

/// Write an executable fake `tool` shim into a fresh tempdir and return that
/// dir. The shim records its argv (one token per line) to `args_file`, then
/// runs `layout`, a shell fragment that creates what the real tool would
/// leave behind. It resets its own `$PATH` first so `mkdir`/`chmod` can still
/// be found, even though the *test*'s `$PATH` is pinned to the shim directory
/// alone.
fn write_fake_tool_shim(tool: &str, args_file: &Path, layout: &str) -> tempfile::TempDir {
    let shim_dir = tempfile::tempdir().unwrap();
    let body = format!(
        "#!/bin/sh\n\
         PATH=/usr/bin:/bin\n\
         printf '%s\\n' \"$@\" > {args_file}\n\
         {layout}\n",
        args_file = args_file.display(),
    );
    let shim_path = shim_dir.path().join(tool);
    std::fs::write(&shim_path, body).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim_dir
}

fn load_lsp_with_sources(
    ed: &mut Editor,
    sources: &[SourceFixture],
    data_dir: &Path,
) -> tempfile::TempDir {
    let runtime = runtime_with_sources(sources);
    load_with_init_in_runtime(
        ed,
        runtime.path(),
        data_dir,
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp-install\")",
    );
    runtime
}

/// Layout of `python3 -m venv <dir>`: a `<dir>/bin/python` that records its
/// own argv to `pip_args_file` and, when `create_binary`, leaves
/// `<dir>/bin/<bin_name>` behind like a real `pip install` would.
fn venv_layout(pip_args_file: &Path, bin_name: &str, create_binary: bool) -> String {
    let create = if create_binary {
        format!(
            "printf '#!/bin/sh\\n' > \"$(dirname \"$0\")/{bin_name}\"\n\
             chmod +x \"$(dirname \"$0\")/{bin_name}\"\n"
        )
    } else {
        String::new()
    };
    format!(
        "mkdir -p \"$3/bin\"\n\
         cat > \"$3/bin/python\" <<'EOS'\n\
         #!/bin/sh\n\
         PATH=/usr/bin:/bin\n\
         printf '%s\\n' \"$@\" > {pip_args_file}\n\
         {create}EOS\n\
         chmod +x \"$3/bin/python\"",
        pip_args_file = pip_args_file.display(),
    )
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("shim must have run and recorded argv")
        .lines()
        .map(str::to_string)
        .collect()
}

/// Layout of a real `go install` with `GOBIN` set: `$GOBIN/<bin_name>`.
fn go_layout(bin_name: &str) -> String {
    format!(
        "mkdir -p \"$GOBIN\"\n\
         printf '#!/bin/sh\\n' > \"$GOBIN/{bin_name}\"\n\
         chmod +x \"$GOBIN/{bin_name}\""
    )
}

mod discovery_and_cargo;
mod github_download;
mod install_commands;
mod package_installers;
mod scan;
mod server_selection;
mod uninstall_and_status;
