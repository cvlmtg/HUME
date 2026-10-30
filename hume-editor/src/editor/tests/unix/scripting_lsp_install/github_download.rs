//! The github download pipeline, lock staleness included.

use super::*;

// ── github-kind download pipeline (fabricated sources, fake curl) ───────────────
//
// The real catalog pins each asset's sha256, so a fabricated archive can't
// pass it. These tests run the shipped plugin sources against a runtime copy
// whose `core:lsp-install` sources name a fabricated asset with its true
// digest, and a `curl` shim that copies that asset into place.

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

/// A `marksman` source whose pinned digest the served asset does not match,
/// and that served asset.
fn mismatched_marksman(work: &Path) -> (PathBuf, Vec<SourceFixture>) {
    let served = work.join("marksman-fake");
    std::fs::write(&served, b"served bytes").unwrap();
    let pinned = work.join("pinned-bytes");
    std::fs::write(&pinned, b"different bytes").unwrap();
    let sources = github_source("marksman", "marksman-fake", &pinned, "marksman-fake");
    (served, sources)
}

#[test]
fn lsp_install_tar_gz_unpacks_a_nested_binary_and_registers() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/lua-language-server"), b"#!/bin/sh\n").unwrap();
    let archive = work.path().join("lls.tar.gz");
    tar_gz(&payload, &["."], &archive);
    let sources = &github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/lua-language-server",
    );
    let (data_tmp, ed, _runtime) = install_from_fixture(sources, &archive, "lua");

    let install_dir = install_dir(data_tmp.path(), "lua-language-server");
    let bin = install_dir.join("pkg/bin/lua-language-server");
    assert_eq!(
        std::fs::metadata(&bin)
            .expect("binary must be unpacked")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(
        !install_dir.join("lls.tar.gz").exists(),
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
    tar_gz(&payload, &["."], &archive);
    let sources = &github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/server",
    );
    let (data_tmp, _ed, _runtime) = install_from_fixture(sources, &archive, "lua");

    let install_dir = install_dir(data_tmp.path(), "lua-language-server");
    for file in &files {
        let mode = std::fs::metadata(install_dir.join(file))
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

    let bin = install_dir(data_tmp.path(), "marksman").join("marksman-fake");
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

    let bin = install_dir(data_tmp.path(), "marksman").join("marksman-fake");
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
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "marksman", "0.0.1", "marksman-fake");
    let installed = install_dir(data_tmp.path(), "marksman");

    let (data_tmp, mut ed, _runtime, _shim) =
        install_from_fixture_into(data_tmp, &sources, &asset, "markdown");
    assert_eq!(
        install_folders(data_tmp.path(), "marksman"),
        std::slice::from_ref(&installed),
        "the install starts only once the server has stopped"
    );

    ed.drain_async_sources();
    ed.settle();
    let bin = install_dir(data_tmp.path(), "marksman").join("marksman-fake");
    assert_eq!(
        std::fs::read_to_string(&bin).unwrap(),
        "#!/bin/sh\necho new\n"
    );
    assert_only_install(data_tmp.path(), "marksman");
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
    let bin = install_dir(data_tmp.path(), "terraform-ls").join("terraform-ls");
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
    let (data_tmp, ed, runtime, _shim) =
        install_from_fixture_into(tempfile::tempdir().unwrap(), sources, fixture, lang);
    (data_tmp, ed, runtime)
}

/// [`install_from_fixture`] over the data dir `data_tmp`, which the caller has
/// prepared. Also returns the curl shim's dir, which an install over an
/// existing one needs until it runs on a later tick.
fn install_from_fixture_into(
    data_tmp: tempfile::TempDir,
    sources: &[SourceFixture],
    fixture: &Path,
    lang: &str,
) -> (
    tempfile::TempDir,
    Editor,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let args_file = fixture.with_extension("curl-argv");
    let (shim, path) = write_fake_curl_shim(fixture, &args_file);
    let mut ed = editor_from("-[x]>\n");
    let runtime = load_lsp_with_sources(&mut ed, sources, data_tmp.path());
    ed.state.dirs.search_path = Some(path.into());
    type_cmd(&mut ed, &format!(":lsp-install {lang}"));
    (data_tmp, ed, runtime, shim)
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

    let install_dir = install_dir(data_tmp.path(), "rust-analyzer");
    let bin = install_dir.join("rust-analyzer");
    assert_eq!(std::fs::read(&bin).unwrap(), b"#!/bin/sh\necho decoded\n");
    assert_eq!(
        std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!install_dir.join("rust-analyzer-fake.gz").exists());
    let cmd = ed
        .state
        .lsp
        .registered_command_for_test("rust-analyzer")
        .expect("rust must be registered after a successful install");
    assert_eq!(Path::new(&cmd), bin);
}

/// `payload`'s `pkg` tree as a zip at `archive`, its symlinks stored as links.
fn zip_pkg(payload: &Path, archive: &Path) {
    assert!(
        std::process::Command::new("zip")
            .current_dir(payload)
            .arg("-r")
            .arg("-y")
            .arg(archive)
            .arg("pkg")
            .status()
            .unwrap()
            .success()
    );
}

/// A file at `work/outside-target`, mode 0o644, for an archive's link to
/// point at from outside the server dir.
fn outside_target(work: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let outside = work.join("outside-target");
    std::fs::write(&outside, b"not part of the archive").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o644)).unwrap();
    outside
}

/// `outside` as [`outside_target`] made it: contents and mode unchanged.
fn assert_left_alone(outside: &Path) {
    use std::os::unix::fs::PermissionsExt;

    assert_eq!(std::fs::read(outside).unwrap(), b"not part of the archive");
    assert_eq!(
        std::fs::metadata(outside).unwrap().permissions().mode() & 0o777,
        0o644,
        "the link's target must be left alone"
    );
}

#[test]
fn lsp_install_zip_marks_regular_files_executable_and_leaves_link_targets_alone() {
    use std::os::unix::fs::PermissionsExt;

    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    for name in ["server", "helper"] {
        let f = payload.join("pkg/bin").join(name);
        std::fs::write(&f, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::create_dir_all(payload.join("pkg/data")).unwrap();
    std::fs::write(payload.join("pkg/data/table"), b"x").unwrap();
    std::fs::set_permissions(
        payload.join("pkg/data"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::os::unix::fs::symlink("data", payload.join("pkg/data-link")).unwrap();
    let archive = work.path().join("tfls.zip");
    zip_pkg(&payload, &archive);
    let sources = github_source("terraform-ls", "tfls.zip", &archive, "pkg/bin/server");

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "hcl");

    let pkg = install_dir(data_tmp.path(), "terraform-ls").join("pkg");
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
        std::fs::symlink_metadata(pkg.join("data-link"))
            .expect("the link must be unpacked")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::metadata(pkg.join("data"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700,
        "a symlink's target must not be chmod'd through it"
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("terraform-ls")
            .is_some()
    );
}

#[test]
fn lsp_install_zip_with_an_escaping_symlink_fails_and_leaves_no_server_dir() {
    let work = tempfile::tempdir().unwrap();
    let outside = outside_target(work.path());
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/server"), b"#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(&outside, payload.join("pkg/escape")).unwrap();
    let archive = work.path().join("tfls.zip");
    zip_pkg(&payload, &archive);
    let sources = github_source("terraform-ls", "tfls.zip", &archive, "pkg/bin/server");

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "hcl");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("install terraform-ls failed"),
        "an escaping link must fail the install: {log}"
    );
    assert_left_alone(&outside);
    assert_install_left_nothing(data_tmp.path(), &ed, "terraform-ls");
}

#[test]
fn lsp_install_refuses_an_archive_whose_symlink_points_outside_the_server_dir() {
    let work = tempfile::tempdir().unwrap();
    let outside = outside_target(work.path());
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/lua-language-server"), b"#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(&outside, payload.join("pkg/escape")).unwrap();
    let archive = work.path().join("lls.tar.gz");
    tar_gz(&payload, &["."], &archive);
    let sources = github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/lua-language-server",
    );

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "lua");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("symlink pointing outside") && log.contains("escape"),
        "the install must fail naming the escaping link: {log}"
    );
    assert_left_alone(&outside);
    assert_install_left_nothing(data_tmp.path(), &ed, "lua-language-server");
}

#[test]
fn lsp_install_keeps_a_symlink_that_stays_inside_the_server_dir() {
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/lua-language-server"), b"#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink("lua-language-server", payload.join("pkg/bin/run")).unwrap();
    let archive = work.path().join("lls.tar.gz");
    tar_gz(&payload, &["."], &archive);
    let sources = github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/lua-language-server",
    );

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "lua");

    let run = install_dir(data_tmp.path(), "lua-language-server").join("pkg/bin/run");
    assert!(
        std::fs::symlink_metadata(&run)
            .expect("the link must be unpacked")
            .file_type()
            .is_symlink()
    );
    assert!(
        ed.state
            .lsp
            .registered_command_for_test("lua-language-server")
            .is_some()
    );
}

#[test]
fn lsp_install_removes_the_server_dir_when_a_symlink_cannot_be_resolved() {
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("pkg/bin")).unwrap();
    std::fs::write(payload.join("pkg/bin/lua-language-server"), b"#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink("b", payload.join("pkg/a")).unwrap();
    std::os::unix::fs::symlink("a", payload.join("pkg/b")).unwrap();
    let archive = work.path().join("lls.tar.gz");
    tar_gz(&payload, &["."], &archive);
    let sources = github_source(
        "lua-language-server",
        "lls.tar.gz",
        &archive,
        "pkg/bin/lua-language-server",
    );

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &archive, "lua");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("install lua-language-server failed"),
        "a link cycle must fail the install: {log}"
    );
    assert_install_left_nothing(data_tmp.path(), &ed, "lua-language-server");
}

#[test]
fn lsp_install_missing_binary_after_unpack_fails_loudly() {
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("other"), b"x").unwrap();
    let archive = work.path().join("lls.tar.gz");
    tar_gz(&payload, &["."], &archive);
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
    assert_install_left_nothing(data_tmp.path(), &ed, "lua-language-server");
}

/// A reinstall that fails keeps the installed server: its files, its receipt
/// and its registration, with nothing left of the failed install.
#[test]
fn lsp_install_failed_reinstall_keeps_the_installed_server() {
    let work = tempfile::tempdir().unwrap();
    let (served, sources) = mismatched_marksman(work.path());
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "marksman", "0.0.1", "marksman-fake");

    let (data_tmp, mut ed, _runtime, _shim) =
        install_from_fixture_into(data_tmp, &sources, &served, "markdown");
    ed.drain_async_sources();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(log.contains("sha256 mismatch"), "{log}");
    let bin = install_dir(data_tmp.path(), "marksman").join("marksman-fake");
    assert_eq!(
        std::fs::read_to_string(&bin).unwrap(),
        "#!/bin/sh\n",
        "{log}"
    );
    assert_installed(data_tmp.path(), &ed, "marksman", "0.0.1", "marksman-fake");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("LSP: kept the installed marksman")
    );
    assert_only_install(data_tmp.path(), "marksman");
}

/// A reinstall deletes what an earlier stopped install left, once its own
/// receipt is written.
#[test]
fn lsp_install_reinstall_deletes_a_stopped_install() {
    let work = tempfile::tempdir().unwrap();
    let asset = work.path().join("marksman-fake");
    std::fs::write(&asset, b"#!/bin/sh\necho new\n").unwrap();
    let sources = github_source("marksman", "marksman-fake", &asset, "marksman-fake");
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "marksman", "0.0.1", "marksman-fake");
    stop_an_install(data_tmp.path(), "marksman");

    let (data_tmp, mut ed, _runtime, _shim) =
        install_from_fixture_into(data_tmp, &sources, &asset, "markdown");
    ed.drain_async_sources();
    ed.settle();

    assert_installed(data_tmp.path(), &ed, "marksman", "9.9.9", "marksman-fake");
    assert_only_install(data_tmp.path(), "marksman");
}

/// A reinstall that fails deletes only its own folder: the installed server
/// stays current and an earlier stopped install's folder waits for the next
/// install that commits.
#[test]
fn lsp_install_failed_reinstall_after_a_stopped_one_keeps_the_installed_server() {
    let work = tempfile::tempdir().unwrap();
    let (served, sources) = mismatched_marksman(work.path());
    let data_tmp = tempfile::tempdir().unwrap();
    fabricate_server(data_tmp.path(), "marksman", "0.0.1", "marksman-fake");
    let stopped = stop_an_install(data_tmp.path(), "marksman");

    let (data_tmp, mut ed, _runtime, _shim) =
        install_from_fixture_into(data_tmp, &sources, &served, "markdown");
    ed.drain_async_sources();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(log.contains("sha256 mismatch"), "{log}");
    assert_installed(data_tmp.path(), &ed, "marksman", "0.0.1", "marksman-fake");
    assert_eq!(
        install_folders(data_tmp.path(), "marksman"),
        [install_dir(data_tmp.path(), "marksman"), stopped],
        "{log}"
    );
}

#[test]
fn lsp_install_sha256_mismatch_deletes_the_archive_and_fails() {
    let work = tempfile::tempdir().unwrap();
    let (served, sources) = mismatched_marksman(work.path());

    let (data_tmp, ed, _runtime) = install_from_fixture(&sources, &served, "markdown");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("sha256 mismatch"),
        "a digest that differs from the pin must fail loudly: {log}"
    );
    assert_install_left_nothing(data_tmp.path(), &ed, "marksman");
}
