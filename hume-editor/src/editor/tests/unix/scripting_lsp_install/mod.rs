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

/// `<data_dir>/servers/<name>/`, the install directory of server `name`.
fn server_dir(data_dir_root: &Path, name: &str) -> PathBuf {
    servers_dir(data_dir_root).join(name)
}

/// Write a receipt + a dummy binary file for `name` directly into
/// `<data_dir>/servers/<name>/`, matching what `lsp-install/install-server!` would
/// produce, for scan-time tests that don't need a real network install.
fn fabricate_server(data_dir_root: &Path, name: &str, version: &str, bin: &str) {
    let dir = server_dir(data_dir_root, name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("receipt.scm"),
        format!(r#"((name . "{name}") (version . "{version}") (bin . "{bin}"))"#),
    )
    .unwrap();
    std::fs::write(dir.join(bin), b"#!/bin/sh\n").unwrap();
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
