//! The github download pipeline, lock staleness included.

use super::*;

// ── github-kind download pipeline (fabricated sources, fake curl) ───────────────
//
// The real catalog pins each asset's sha256, so a fabricated archive can't
// pass it. These tests run the shipped plugin sources against a runtime copy
// whose `core:lsp-install` sources name a fabricated asset with its true
// digest, and a `curl` shim that copies that asset into place.

fn sha256_hex(path: &Path) -> String {
    let out = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .expect("spawn shasum");
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// `name`'s github-kind source record serving `asset` (digest of `fixture`)
/// for every Unix install target, with `bin` as the binary's path in the
/// server directory.
fn github_source(name: &str, asset: &str, fixture: &Path, bin: &str) -> Vec<SourceFixture> {
    let sha = sha256_hex(fixture);
    let targets: String = ["darwin-arm64", "darwin-x64", "linux-x64"]
        .iter()
        .map(|t| format!("({t} \"{asset}\" \"sha256:{sha}\" \"{bin}\")"))
        .collect::<Vec<_>>()
        .join(" ");
    vec![(
        name.to_string(),
        format!("((kind . github) (version . \"9.9.9\") (repo . \"o/r\") (targets {targets}))"),
    )]
}

/// Fake `curl` that copies `fixture` to its `-o` argument and records its
/// argv to `args_file`. The real `tar`/`gzip`/`unzip` stay reachable through
/// the returned `$PATH` value.
fn write_fake_curl_shim(fixture: &Path, args_file: &Path) -> (tempfile::TempDir, String) {
    let shim_dir = write_fake_tool_shim(
        "curl",
        args_file,
        &format!(
            "out=\"\"; prev=\"\"\n\
             for a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; done\n\
             cp {fixture} \"$out\"",
            fixture = fixture.display(),
        ),
    );
    let path = format!("{}:/usr/bin:/bin", shim_dir.path().display());
    (shim_dir, path)
}

#[test]
fn lsp_install_tar_gz_unpacks_a_nested_binary_and_registers() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/lua-language-server"), b"#!/bin/sh\n").unwrap();
    let archive = work.path().join("lls.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    let sources = &github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/lua-language-server",
    );
    let (data_tmp, ed, _runtime) = install_from_fixture(sources, &archive, "lua");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("lua-language-server");
    let bin = server_dir.join("pkg/bin/lua-language-server");
    assert_eq!(
        std::fs::metadata(&bin)
            .expect("binary must be unpacked")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(
        !server_dir.join("lls.tar.gz").exists(),
        "the downloaded archive must be deleted after unpacking"
    );
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("lua-language-server")
        .expect("lua must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

#[test]
fn lsp_install_tar_gz_with_empty_directories_marks_every_file_executable() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    for empty in ["a", "b", "c", "d"] {
        std::fs::create_dir_all(payload.join(format!("empty-{empty}"))).unwrap();
    }
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    let files: Vec<String> = ["server", "x1", "x2", "x3", "x4", "x5"]
        .iter()
        .map(|name| format!("pkg/bin/{name}"))
        .collect();
    for file in &files {
        std::fs::write(payload.join(file), b"#!/bin/sh\n").unwrap();
    }
    let archive = work.path().join("lls.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    let sources = &github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/server",
    );
    let (data_tmp, _ed, _runtime) = install_from_fixture(sources, &archive, "lua");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("lua-language-server");
    for file in &files {
        let mode = std::fs::metadata(server_dir.join(file))
            .unwrap_or_else(|e| panic!("{file} must be unpacked: {e}"))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "{file}");
    }
}

#[test]
fn lsp_install_raw_binary_is_marked_executable_and_kept() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let asset = work.path().join("marksman-fake");
    std::fs::write(&asset, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&asset, std::fs::Permissions::from_mode(0o644)).unwrap();
    let sources = &github_source("marksman", "marksman-fake", &asset, "marksman-fake");
    let (data_tmp, ed, _runtime) = install_from_fixture(sources, &asset, "markdown");

    let bin = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("marksman")
        .join("marksman-fake");
    assert_eq!(
        std::fs::metadata(&bin)
            .expect("the raw download is the binary and must be kept")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("marksman")
        .expect("markdown must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

/// A download-and-register install replaces a server the user registered by
/// hand under the same name: the installed copy is what runs afterwards.
#[test]
fn lsp_install_replaces_a_manual_registration_of_the_same_name() {
    let work = tempfile::tempdir().unwrap();
    let asset = work.path().join("marksman-fake");
    std::fs::write(&asset, b"#!/bin/sh\n").unwrap();
    let sources = github_source("marksman", "marksman-fake", &asset, "marksman-fake");
    let args_file = asset.with_extension("curl-argv");
    let (_shim, path) = write_fake_curl_shim(&asset, &args_file);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    let runtime = runtime_with_sources(&sources);
    load_with_init_in_runtime(
        &mut ed,
        runtime.path(),
        data_tmp.path(),
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp-install\")\n\
         (register-lsp-server! \"marksman\" #:command \"my-marksman\")",
    );
    assert_eq!(
        ed.state.lsp.registered_command_for_test("marksman"),
        Some("my-marksman".to_owned()),
        "precondition: the manual registration from init.scm took effect"
    );

    ed.state.dirs.search_path = Some(path.into());
    type_cmd(&mut ed, ":lsp-install markdown");

    let bin = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("marksman")
        .join("marksman-fake");
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("marksman")
        .expect("marksman stays registered");
    assert_eq!(Path::new(&cmd), bin);
}

/// An install over an existing one touches the server's files only after
/// the unregister the command queued has stopped the running server: a
/// running binary cannot be replaced on every platform.
#[test]
fn lsp_install_over_an_existing_install_waits_for_the_server_to_stop() {
    let work = tempfile::tempdir().unwrap();
    let asset = work.path().join("marksman-fake");
    std::fs::write(&asset, b"#!/bin/sh\necho new\n").unwrap();
    let sources = github_source("marksman", "marksman-fake", &asset, "marksman-fake");
    let (_shim, path) = write_fake_curl_shim(&asset, &asset.with_extension("curl-argv"));
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "marksman", "0.0.1", "marksman-fake");
    let mut ed = editor_from("-[x]>\n");
    let _runtime = load_lsp_with_sources(&mut ed, &sources, data_tmp.path());
    let bin = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("marksman")
        .join("marksman-fake");
    ed.state.dirs.search_path = Some(path.into());

    type_cmd(&mut ed, ":lsp-install markdown");
    assert_eq!(
        std::fs::read_to_string(&bin).unwrap(),
        "#!/bin/sh\n",
        "the existing install's files stay until the server has stopped"
    );

    ed.drain_async_sources();
    ed.settle();
    assert_eq!(
        std::fs::read_to_string(&bin).unwrap(),
        "#!/bin/sh\necho new\n"
    );
    assert_eq!(
        ed.state
            .lsp
            .registered_command_for_test("marksman")
            .as_deref(),
        bin.to_str()
    );
}

#[test]
fn lsp_install_generic_kind_downloads_the_recorded_url_and_registers() {
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("terraform-ls"), b"#!/bin/sh\n").unwrap();
    let archive = work.path().join("tfls.zip");
    assert!(
        std::process::Command::new("zip")
            .arg("-j")
            .arg(&archive)
            .arg(payload.join("terraform-ls"))
            .status()
            .unwrap()
            .success()
    );
    let sha = sha256_hex(&archive);
    let url = "https://example.test/terraform-ls/9.9.9/tfls.zip";
    let targets: String = ["darwin-arm64", "darwin-x64", "linux-x64"]
        .iter()
        .map(|t| format!("({t} \"tfls.zip\" \"{url}\" \"sha256:{sha}\" \"terraform-ls\")"))
        .collect::<Vec<_>>()
        .join(" ");
    let sources = &[(
        "terraform-ls".to_string(),
        format!("((kind . generic) (version . \"9.9.9\") (targets {targets}))"),
    )];
    let (data_tmp, ed, _runtime) = install_from_fixture(sources, &archive, "hcl");

    let argv = std::fs::read_to_string(archive.with_extension("curl-argv"))
        .expect("curl shim must have run");
    assert_eq!(
        argv.lines().last(),
        Some(url),
        "curl must fetch the url recorded in the row verbatim: {argv}"
    );
    let bin = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("terraform-ls")
        .join("terraform-ls");
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("terraform-ls")
        .expect("hcl must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

/// Load `core:lsp-install` against a runtime whose sources are `sources`, with a curl
/// shim serving `fixture`, and run `:lsp-install <lang>`. Returns the data dir
/// and the editor for post-install assertions.
fn install_from_fixture(
    sources: &[SourceFixture],
    fixture: &Path,
    lang: &str,
) -> (tempfile::TempDir, Editor, tempfile::TempDir) {
    let args_file = fixture.with_extension("curl-argv");
    let (_shim, path) = write_fake_curl_shim(fixture, &args_file);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    let runtime = load_lsp_with_sources(&mut ed, sources, data_tmp.path());
    ed.state.dirs.search_path = Some(path.into());
    type_cmd(&mut ed, &format!(":lsp-install {lang}"));
    (data_tmp, ed, runtime)
}

/// Install `server` for `lang` from a gzip fixture while an executable named
/// `command` is on `$PATH`, and return the status message the install leaves.
fn install_status_with_command_on_path(server: &str, lang: &str, command: &str) -> String {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let plain = work.path().join("fixture-bin");
    std::fs::write(&plain, b"#!/bin/sh\n").unwrap();
    assert!(
        std::process::Command::new("gzip")
            .arg("-k")
            .arg(&plain)
            .status()
            .unwrap()
            .success()
    );
    let archive = work.path().join("fixture-bin.gz");
    let sources = github_source(server, "fixture-bin.gz", &archive, command);
    let (shim, _) = write_fake_curl_shim(&archive, &archive.with_extension("curl-argv"));
    let on_path = shim.path().join(command);
    std::fs::write(&on_path, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&on_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", shim.path().display());

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    let _runtime = load_lsp_with_sources(&mut ed, &sources, data_tmp.path());
    ed.state.dirs.search_path = Some(path.into());
    type_cmd(&mut ed, &format!(":lsp-install {lang}"));
    ed.state.status_msg.clone().unwrap_or_default()
}

/// A server whose catalog record has no `command` field runs a command
/// named like itself.
#[test]
fn install_notes_a_path_command_named_like_the_server() {
    let msg = install_status_with_command_on_path("rust-analyzer", "rust", "rust-analyzer");

    assert!(
        msg.contains("rust-analyzer is also on $PATH"),
        "the note must name the server's own name as its command: {msg}"
    );
}

#[test]
fn install_notes_a_path_command_from_the_commands_catalog() {
    let msg =
        install_status_with_command_on_path("ada-language-server", "ada", "ada_language_server");

    assert!(
        msg.contains("ada_language_server is also on $PATH"),
        "the note must name the command the commands catalog lists: {msg}"
    );
}

#[test]
fn lsp_install_gz_asset_is_decoded_and_marked_executable() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let plain = work.path().join("rust-analyzer-fake");
    std::fs::write(&plain, b"#!/bin/sh\necho decoded\n").unwrap();
    assert!(
        std::process::Command::new("gzip")
            .arg("-k")
            .arg(&plain)
            .status()
            .unwrap()
            .success()
    );
    let archive = work.path().join("rust-analyzer-fake.gz");
    let sources = github_source(
        "rust-analyzer",
        "rust-analyzer-fake.gz",
        &archive,
        "rust-analyzer",
    );

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "rust");

    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("rust-analyzer");
    let bin = server_dir.join("rust-analyzer");
    assert_eq!(std::fs::read(&bin).unwrap(), b"#!/bin/sh\necho decoded\n");
    assert_eq!(
        std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!server_dir.join("rust-analyzer-fake.gz").exists());
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("rust-analyzer")
        .expect("rust must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

#[test]
fn lsp_install_zip_marks_every_regular_file_executable_and_never_follows_symlinks() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let outside = work.path().join("outside-target");
    std::fs::write(&outside, b"not part of the archive").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o644)).unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    for name in ["server", "helper"] {
        let f = payload.join("pkg/bin").join(name);
        std::fs::write(&f, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::os::unix::fs::symlink(&outside, payload.join("pkg/link")).unwrap();
    let archive = work.path().join("tfls.zip");
    assert!(
        std::process::Command::new("zip")
            .current_dir(&payload)
            .arg("-ry")
            .arg(&archive)
            .arg("pkg")
            .status()
            .unwrap()
            .success()
    );
    let sources = github_source("terraform-ls", "tfls.zip", &archive, "pkg/bin/server");

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "hcl");

    let pkg = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("terraform-ls")
        .join("pkg");
    for name in ["server", "helper"] {
        assert_eq!(
            std::fs::metadata(pkg.join("bin").join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "{name} must be made executable"
        );
    }
    assert!(
        std::fs::symlink_metadata(pkg.join("link"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the symlink entry must stay a symlink"
    );
    assert_eq!(
        std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
        0o644,
        "a symlink's target must never be chmod'd"
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("terraform-ls")
            .is_some()
    );
}

#[test]
fn lsp_install_missing_binary_after_unpack_fails_loudly() {
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("other"), b"x").unwrap();
    let archive = work.path().join("lls.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    let sources = github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "bin/lua-language-server",
    );

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "lua");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("expected binary"),
        "an archive without the recorded binary must fail loudly: {log}"
    );
    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("lua-language-server");
    assert!(!server_dir.join("receipt.scm").exists());
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("lua-language-server")
            .is_none()
    );
}

#[test]
fn lsp_install_sha256_mismatch_deletes_the_archive_and_fails() {
    let work = tempfile::tempdir().unwrap();
    let archive = work.path().join("marksman-fake");
    std::fs::write(&archive, b"served bytes").unwrap();
    let pinned = work.path().join("pinned-bytes");
    std::fs::write(&pinned, b"different bytes").unwrap();
    let sources = github_source("marksman", "marksman-fake", &pinned, "marksman-fake");

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "markdown");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("sha256 mismatch"),
        "a digest that differs from the pin must fail loudly: {log}"
    );
    let server_dir = canonical_data_dir(data_tmp.path())
        .join("servers")
        .join("marksman");
    assert!(!server_dir.join("marksman-fake").exists());
    assert!(!server_dir.join("receipt.scm").exists());
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("marksman")
            .is_none()
    );
}

fn set_lock_mtime(lock_path: &Path, offset_secs: i64) {
    let now = std::time::SystemTime::now();
    let delta = std::time::Duration::from_secs(offset_secs.unsigned_abs());
    let when = if offset_secs < 0 {
        now - delta
    } else {
        now + delta
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(lock_path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn lsp_install_replaces_a_lock_older_than_an_hour() {
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    let lock_path = servers_dir.join(".install-lock");
    std::fs::write(&lock_path, b"").unwrap();
    set_lock_mtime(&lock_path, -2 * 3600);

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());
    // ocamllsp fails inside the locked section, after the lock was acquired.
    type_cmd(&mut ed, ":lsp-install ocaml");

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already in progress"),
        "a stale lock must be replaced, not honoured: {log}"
    );
    assert!(log.contains("opam"), "the install must have run: {log}");
    assert!(
        !lock_path.exists(),
        "the replaced lock must be released afterwards"
    );
}

#[test]
fn lsp_install_treats_a_lock_dated_in_the_future_as_live() {
    let data_tmp = tempfile::tempdir().unwrap();
    let servers_dir = canonical_data_dir(data_tmp.path()).join("servers");
    std::fs::create_dir_all(&servers_dir).unwrap();
    let lock_path = servers_dir.join(".install-lock");
    std::fs::write(&lock_path, b"").unwrap();
    set_lock_mtime(&lock_path, 2 * 3600);

    let mut ed = editor_from("-[x]>\n");
    load_lsp_install(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":lsp-install ocaml");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("already in progress"),
        "a lock with a future mtime must count as live: {log}"
    );
    assert!(lock_path.exists(), "a live lock must be left in place");
}
