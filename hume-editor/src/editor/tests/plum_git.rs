use super::*;

/// The id of `repo`'s current commit.
fn head_id(repo: &std::path::Path) -> String {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git rev-parse HEAD in {repo:?} failed"
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Commit `content` as `plugin.scm` on `repo`'s current branch.
fn commit_plugin(repo: &std::path::Path, content: &str) {
    std::fs::write(repo.join("plugin.scm"), content).unwrap();
    git(repo, &["add", "plugin.scm"]);
    git(repo, &["commit", "-q", "-m", content.trim()]);
}

/// A local stand-in for a plugin's GitHub repo: `main` holds one commit,
/// `; v1`, plus a `.gitattributes`.
fn plugin_origin() -> tempfile::TempDir {
    let origin = tempfile::tempdir().unwrap();
    git_init(origin.path());
    git(origin.path(), &["checkout", "-q", "-b", "main"]);
    // `-text` checks every file out byte for byte, whatever the host's
    // `core.autocrlf` (Git for Windows sets it to `true`).
    std::fs::write(origin.path().join(".gitattributes"), "* -text\n").unwrap();
    git(origin.path(), &["add", ".gitattributes"]);
    commit_plugin(origin.path(), "; v1\n");
    origin
}

/// Where PLUM keeps `testuser/testrepo` under `data_dir`.
fn plugin_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("hume/plugins/testuser/testrepo")
}

/// Where `plum/install-repo!` stages `testuser/testrepo` under `data_dir`.
fn plugin_stage(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("hume/plum-staging/testuser/testrepo")
}

/// The `plugin.scm` checked out in the clone at `dir`.
fn plugin_scm(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("plugin.scm")).unwrap()
}

/// Clone `origin` where PLUM keeps `testuser/testrepo` under `data_dir`,
/// returning the clone's directory.
fn install_plugin_clone(
    origin: &std::path::Path,
    data_dir: &std::path::Path,
) -> std::path::PathBuf {
    let clone_dir = plugin_dir(data_dir);
    std::fs::create_dir_all(clone_dir.parent().unwrap()).unwrap();
    git(
        data_dir,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            clone_dir.to_str().unwrap(),
        ],
    );
    clone_dir
}

/// A headless editor with PLUM loaded after `init_src`.
fn plum_editor(data_dir: &std::path::Path, init_src: &str) -> Editor {
    let mut ed = editor_from("-[x]>\n");
    ed.tui = crate::editor::tui::Tui::OnHeadless;
    load_plum_with(&mut ed, data_dir, init_src, None);
    ed
}

/// Load PLUM after `init_src` and run `:plum-update-plugins`, asserting it
/// logged no error.
fn run_plum_update_plugins(data_dir: &std::path::Path, init_src: &str) -> Editor {
    let mut ed = plum_editor(data_dir, init_src);

    type_cmd(&mut ed, ":plum-update-plugins");

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        ":plum-update-plugins against a local origin must not error: {errors:?}"
    );
    ed
}

/// Install `origin` where PLUM keeps `testuser/testrepo` under `data_dir`,
/// through `plum/install-repo!`, the funnel every PLUM install command calls.
/// `args` is the Scheme source of the arguments after the URL and the
/// destination: the ref (a string literal, or `#f`) and any keyword arguments.
/// Returns the error log.
fn install_errors(origin: &std::path::Path, data_dir: &std::path::Path, args: &str) -> Vec<String> {
    let helpers = repo_runtime_dir().join("plugins/core/plum/lib/helpers.scm");
    let mut ed = plum_editor(
        data_dir,
        &format!(
            r#"(require {})
               (define-typed-command! "test-install-repo" ""
                 (lambda () (plum/install-repo! {} {} {args}))
                 #:inline-output #t)"#,
            steel_path(&helpers),
            steel_path(origin),
            steel_path(&plugin_dir(data_dir)),
        ),
    );

    type_cmd(&mut ed, ":test-install-repo");

    error_log(&ed)
}

/// [`install_errors`], asserting the install logged no error. Returns the
/// install directory.
fn install_ok(
    origin: &std::path::Path,
    data_dir: &std::path::Path,
    args: &str,
) -> std::path::PathBuf {
    let errors = install_errors(origin, data_dir, args);
    assert!(
        errors.is_empty(),
        "install of {args} must not error: {errors:?}"
    );
    plugin_dir(data_dir)
}

/// `:plum-update-plugins` runs real `git` against a local origin, no network.
/// The origin gets a second commit after the clone is made, and the update
/// must bring the clone to it, inside the inline-output terminal bracket
/// (the command is `#:inline-output`, so `Tui::OnHeadless` is required for
/// `inline_output_enter_count` to move at all).
#[test]
fn plum_update_brings_an_unpinned_plugin_to_origins_latest_commit() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();
    let clone_dir = install_plugin_clone(origin.path(), data_tmp.path());
    commit_plugin(origin.path(), "; v2\n");

    let ed = run_plum_update_plugins(data_tmp.path(), "");

    assert_eq!(plugin_scm(&clone_dir), "; v2\n");
    assert!(
        ed.inline_output_enter_count() > 0,
        ":plum-update-plugins is #:inline-output, so a real update must have entered \
         the terminal bracket for git's progress to be visible"
    );
}

#[test]
fn plum_update_moves_a_plugin_pinned_to_a_tag_onto_that_tag() {
    let origin = plugin_origin();
    git(origin.path(), &["tag", "-a", "v1.0", "-m", "v1.0"]);
    commit_plugin(origin.path(), "; v2\n");
    let data_tmp = tempfile::tempdir().unwrap();
    let clone_dir = install_plugin_clone(origin.path(), data_tmp.path());

    run_plum_update_plugins(
        data_tmp.path(),
        r#"(load-plugin! "testuser/testrepo" #:ref "v1.0")"#,
    );

    assert_eq!(plugin_scm(&clone_dir), "; v1\n");
}

#[test]
fn plum_update_follows_a_pinned_branch_rather_than_the_default_one() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();
    let clone_dir = install_plugin_clone(origin.path(), data_tmp.path());
    git(origin.path(), &["checkout", "-q", "-b", "stable"]);
    commit_plugin(origin.path(), "; stable\n");
    git(origin.path(), &["checkout", "-q", "main"]);
    commit_plugin(origin.path(), "; main\n");

    run_plum_update_plugins(
        data_tmp.path(),
        r#"(load-plugin! "testuser/testrepo" #:ref "stable")"#,
    );

    assert_eq!(plugin_scm(&clone_dir), "; stable\n");
}

#[test]
fn plum_update_moves_a_plugin_pinned_to_a_short_commit_id_onto_that_commit() {
    let origin = plugin_origin();
    let first = head_id(origin.path());
    commit_plugin(origin.path(), "; v2\n");
    let data_tmp = tempfile::tempdir().unwrap();
    let clone_dir = install_plugin_clone(origin.path(), data_tmp.path());

    run_plum_update_plugins(
        data_tmp.path(),
        &format!(
            r#"(load-plugin! "testuser/testrepo" #:ref "{}")"#,
            &first[..7]
        ),
    );

    assert_eq!(plugin_scm(&clone_dir), "; v1\n");
}

/// A clone left on a tag by an earlier pin has a detached HEAD.
#[test]
fn plum_update_brings_an_unpinned_plugin_off_a_tag_to_the_default_branch() {
    let origin = plugin_origin();
    git(origin.path(), &["tag", "-a", "v1.0", "-m", "v1.0"]);
    commit_plugin(origin.path(), "; v2\n");
    let data_tmp = tempfile::tempdir().unwrap();
    let clone_dir = install_plugin_clone(origin.path(), data_tmp.path());
    git(&clone_dir, &["checkout", "-q", "--detach", "v1.0"]);

    run_plum_update_plugins(data_tmp.path(), r#"(load-plugin! "testuser/testrepo")"#);

    assert_eq!(plugin_scm(&clone_dir), "; v2\n");
}

#[test]
fn plum_install_checks_a_pinned_commit_out() {
    let origin = plugin_origin();
    let first = head_id(origin.path());
    commit_plugin(origin.path(), "; v2\n");
    let data_tmp = tempfile::tempdir().unwrap();

    let dest = install_ok(origin.path(), data_tmp.path(), &format!("{first:?}"));

    assert_eq!(plugin_scm(&dest), "; v1\n");
}

#[test]
fn plum_install_checks_a_pinned_short_commit_id_out() {
    let origin = plugin_origin();
    let first = head_id(origin.path());
    commit_plugin(origin.path(), "; v2\n");
    let data_tmp = tempfile::tempdir().unwrap();

    let dest = install_ok(
        origin.path(),
        data_tmp.path(),
        &format!("{:?}", &first[..7]),
    );

    assert_eq!(plugin_scm(&dest), "; v1\n");
}

#[test]
fn plum_install_brings_an_unpinned_plugin_to_origins_latest_commit() {
    let origin = plugin_origin();
    commit_plugin(origin.path(), "; v2\n");
    let data_tmp = tempfile::tempdir().unwrap();

    let dest = install_ok(origin.path(), data_tmp.path(), "#f");

    assert_eq!(plugin_scm(&dest), "; v2\n");
}

#[test]
fn plum_install_of_a_missing_ref_installs_nothing_and_can_be_retried() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();

    let errors = install_errors(origin.path(), data_tmp.path(), r#""nope""#);

    assert!(
        errors
            .iter()
            .any(|e| e.contains("no tag, branch or commit 'nope'")),
        "a ref missing upstream must fail the install, naming the ref: {errors:?}"
    );
    assert!(
        !plugin_dir(data_tmp.path()).exists(),
        "a failed install must leave no plugin directory"
    );
    assert!(
        !plugin_stage(data_tmp.path()).exists(),
        "a failed install must delete its staging clone"
    );

    let dest = install_ok(origin.path(), data_tmp.path(), r#""main""#);

    assert_eq!(plugin_scm(&dest), "; v1\n");
}

/// A commit no branch or tag reaches is not fetched with them, and a short id
/// cannot be looked up on the remote.
#[test]
fn plum_install_of_an_unreachable_short_commit_id_names_it_in_the_failure() {
    let origin = plugin_origin();
    commit_plugin(origin.path(), "; v2\n");
    let dangling = head_id(origin.path());
    git(origin.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    let data_tmp = tempfile::tempdir().unwrap();

    let short = &dangling[..7];
    let errors = install_errors(origin.path(), data_tmp.path(), &format!("{short:?}"));

    let named = format!("no tag, branch or commit '{short}'");
    assert!(
        errors.iter().any(|e| e.contains(&named)),
        "an unreachable short commit id must fail the install, naming it: {errors:?}"
    );
    assert!(
        !plugin_dir(data_tmp.path()).exists(),
        "a failed install must leave no plugin directory"
    );
}

/// A full commit id is fetched by itself, so a commit no branch or tag
/// reaches still installs.
#[test]
fn plum_install_checks_an_unreachable_full_commit_id_out() {
    let origin = plugin_origin();
    commit_plugin(origin.path(), "; v2\n");
    let dangling = head_id(origin.path());
    git(origin.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    let data_tmp = tempfile::tempdir().unwrap();

    let dest = install_ok(origin.path(), data_tmp.path(), &format!("{dangling:?}"));

    assert_eq!(plugin_scm(&dest), "; v2\n");
}

#[test]
fn plum_install_over_an_existing_clone_replaces_it_only_once_the_new_one_is_ready() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();
    let dest = install_ok(origin.path(), data_tmp.path(), "#f");
    commit_plugin(origin.path(), "; v2\n");

    let errors = install_errors(origin.path(), data_tmp.path(), r#""nope""#);

    assert!(
        !errors.is_empty(),
        "a ref missing upstream must fail the install"
    );
    assert_eq!(
        plugin_scm(&dest),
        "; v1\n",
        "a failed reinstall must keep the old clone"
    );

    install_ok(origin.path(), data_tmp.path(), "#f");

    assert_eq!(plugin_scm(&dest), "; v2\n");
}

#[test]
fn plum_install_runs_its_build_on_the_new_clone_before_it_lands() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();

    let dest = install_ok(
        origin.path(),
        data_tmp.path(),
        r#"#f #:build (lambda (dir) (call! "stdlib/write-file!" (path-join dir "built") "yes"))"#,
    );

    let built = std::fs::read_to_string(dest.join("built")).unwrap();
    assert_eq!(built, "yes");
}

#[test]
fn plum_install_whose_build_fails_keeps_the_old_clone() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();
    let dest = install_ok(origin.path(), data_tmp.path(), "#f");
    commit_plugin(origin.path(), "; v2\n");

    let marker = data_tmp.path().join("build-ran");

    let errors = install_errors(
        origin.path(),
        data_tmp.path(),
        &format!(
            r#"#f #:build (lambda (dir) (call! "stdlib/write-file!" {} "") (error "build failed"))"#,
            steel_path(&marker)
        ),
    );

    assert!(marker.exists(), "the build must have run: {errors:?}");
    assert!(
        !errors.is_empty(),
        "the build's failure must fail the install"
    );
    assert_eq!(
        plugin_scm(&dest),
        "; v1\n",
        "a failed build must keep the old clone"
    );
    assert!(
        !plugin_stage(data_tmp.path()).exists(),
        "a failed build must delete its staging clone"
    );
}

/// A failed install reports the failure's own error, not one wrapping it,
/// and points at where it was raised.
#[test]
fn plum_install_whose_build_fails_reports_the_builds_own_error() {
    let origin = plugin_origin();
    let data_tmp = tempfile::tempdir().unwrap();

    let errors = install_errors(
        origin.path(),
        data_tmp.path(),
        r#"#f #:build (lambda (dir) (error "build failed"))"#,
    );

    let [report] = errors.as_slice() else {
        panic!("expected one error: {errors:#?}");
    };
    assert!(report.contains("build failed"), "{report}");
    assert!(
        !report.contains("helpers.scm") && !report.contains("Error: Generic:"),
        "{report}"
    );
}
