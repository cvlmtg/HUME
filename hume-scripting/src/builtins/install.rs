//! LSP server install pipeline builtins: platform identification, sha256
//! verification, archive unpacking, and a `$PATH` lookup predicate.
//!
//! sha256 verification and archive unpacking shell out to per-platform
//! system tools (`hume_platform::process`) rather than pulling in
//! hashing/archive crates — see `docs/LSP-INSTALL.md`'s "Required external
//! tools" note for exactly what each platform needs installed.
//!
//! | Steel name                     | Signature              | Notes                              |
//! |---------------------------------|------------------------|-------------------------------------|
//! | `hume-target`                   | `() → string \| #f`    | install-target id, or `#f`         |
//! | `verify-sha256!`                | `string string → void` | deletes `path` on mismatch         |
//! | `unpack-gz`                     | `string string → void` | sandboxed to `<data>/servers/`     |
//! | `unpack-zip`                    | `string string → void` | sandboxed to `<data>/servers/`     |
//! | `exe-on-path?`                  | `string → bool`        | real `PATH` scan, no spawn         |

use std::path::PathBuf;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::log::LogLevel;

use super::one_string;
use super::shell::{SandboxKind, validate_new_path};

/// `(hume-target)` — the install-target identifier for the current platform
/// (`"darwin-arm64"`, `"darwin-x64"`, `"linux-x64"`, `"windows-x64"`), or
/// `#f` on any other platform/architecture. `#f`, not an error, so
/// `:lsp-servers` can render "unsupported platform" rather than aborting.
pub(crate) fn hume_target(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "hume-target expects 0 args, got {}", args.len());
    }
    Ok(match hume_platform::target::hume_target() {
        Some(t) => SteelVal::StringV(t.into()),
        None => SteelVal::BoolV(false),
    })
}

/// `(verify-sha256! path expected)` — verify `path`'s sha256 digest matches
/// `expected`. Accepts either the seeded data-file literal (`"sha256:<hex>"`)
/// or bare hex; the comparison is ASCII-case-insensitive.
///
/// `path` must resolve inside `<data>/servers/`. On mismatch, `path` is
/// deleted (mirrors `curl-fetch`'s partial-artifact cleanup) and the error
/// names the path plus both digests.
pub(crate) fn verify_sha256(
    ctx: &mut SteelCtx,
    path: String,
    expected: String,
) -> Result<SteelVal, SteelErr> {
    let canonical = hume_platform::fs::canonicalize(&PathBuf::from(&path)).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("verify-sha256!: cannot resolve '{path}': {e}"),
        )
    })?;
    if !super::sandbox::is_under_servers_sandbox(&canonical) {
        steel::stop!(Generic =>
            "verify-sha256!: path is outside the write sandbox (<data>/servers/): {}", path);
    }

    let expected_hex = expected
        .strip_prefix("sha256:")
        .unwrap_or(&expected)
        .to_ascii_lowercase();

    ctx.log(LogLevel::Trace, format!("verify-sha256!: hashing {path}"));

    let actual = hume_platform::process::sha256_file(&canonical).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("verify-sha256!: cannot hash '{path}': {e}"),
        )
    })?;

    if actual != expected_hex {
        let _ = hume_platform::fs::remove_file(&canonical);
        steel::stop!(Generic =>
            "verify-sha256!: sha256 mismatch for '{path}': expected {expected_hex}, got {actual}");
    }
    Ok(SteelVal::Void)
}

/// `(unpack-gz src dest)` — decode the single-file gzip archive at `src`
/// into `dest` (shells out to `gzip -dc`; on Unix, `dest` is chmod'd `0o755`
/// after success — Mason `.gz` assets are bare server executables).
///
/// `src` must resolve inside `<data>/servers/`; `dest` is validated as a new
/// path in the same sandbox. On error, any partial `dest` is removed before
/// raising (mirrors `curl-fetch`'s cleanup contract).
pub(crate) fn unpack_gz(
    ctx: &mut SteelCtx,
    src: String,
    dest: String,
) -> Result<SteelVal, SteelErr> {
    let canonical_src = hume_platform::fs::canonicalize(&PathBuf::from(&src)).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("unpack-gz: cannot resolve src '{src}': {e}"),
        )
    })?;
    if !super::sandbox::is_under_servers_sandbox(&canonical_src) {
        steel::stop!(Generic =>
            "unpack-gz: src is outside the write sandbox (<data>/servers/): {}", src);
    }
    let canonical_dest =
        validate_new_path(&PathBuf::from(&dest), "unpack-gz", SandboxKind::Servers)?;

    ctx.log(LogLevel::Trace, format!("unpack-gz: {src} → {dest}"));

    if let Err(e) = hume_platform::process::unpack_gz(&canonical_src, &canonical_dest) {
        let _ = hume_platform::fs::remove_file(&canonical_dest);
        steel::stop!(Generic => "unpack-gz: {}", e);
    }
    Ok(SteelVal::Void)
}

/// `(unpack-zip src dest-dir)` — extract the zip archive at `src` into
/// `dest-dir` (`unzip -o` on Unix, `tar -xf` on Windows).
///
/// Zip-slip and symlink-entry protection is delegated to the system tool —
/// the residual risk is bounded by the sha256 pin verified before unpacking
/// (see `docs/LSP-INSTALL.md`'s accepted tradeoffs).
///
/// `src` must resolve inside `<data>/servers/`; `dest-dir` is validated as a
/// new path in the same sandbox and created if absent (`tar -C` requires an
/// existing directory). On error, `dest-dir` is left as-is — a
/// dir-without-receipt is already the interrupted-install signal the
/// installer relies on, so cleaning up here would duplicate that mechanism.
pub(crate) fn unpack_zip(
    ctx: &mut SteelCtx,
    src: String,
    dest_dir: String,
) -> Result<SteelVal, SteelErr> {
    let canonical_src = hume_platform::fs::canonicalize(&PathBuf::from(&src)).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("unpack-zip: cannot resolve src '{src}': {e}"),
        )
    })?;
    if !super::sandbox::is_under_servers_sandbox(&canonical_src) {
        steel::stop!(Generic =>
            "unpack-zip: src is outside the write sandbox (<data>/servers/): {}", src);
    }
    let canonical_dest = validate_new_path(
        &PathBuf::from(&dest_dir),
        "unpack-zip",
        SandboxKind::Servers,
    )?;
    hume_platform::fs::create_dir_all(&canonical_dest).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("unpack-zip: cannot create dest dir '{dest_dir}': {e}"),
        )
    })?;

    ctx.log(LogLevel::Trace, format!("unpack-zip: {src} → {dest_dir}"));

    hume_platform::process::unpack_zip(&canonical_src, &canonical_dest)
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("unpack-zip: {e}")))?;
    Ok(SteelVal::Void)
}

/// `(exe-on-path? name)` → bool. A real `PATH` scan, no spawn — a lookup
/// predicate must be side-effect-free (some tools do real work on
/// `--version`). Rejects `name` containing a path separator (must be a bare
/// command name).
pub(crate) fn exe_on_path(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let name = one_string(args, "exe-on-path?")?;
    Ok(SteelVal::BoolV(hume_platform::process::exe_on_search_path(
        &name,
    )))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> PathBuf {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        super::super::sandbox::init_dirs(Some(data_dir.clone()), None);
        data_dir.join("servers")
    }

    // ── hume-target ────────────────────────────────────────────────────────

    #[test]
    fn hume_target_returns_string_or_false() {
        let result = hume_target(&[]).unwrap();
        match result {
            SteelVal::StringV(s) => assert!(
                matches!(
                    s.as_str(),
                    "darwin-arm64" | "darwin-x64" | "linux-x64" | "windows-x64"
                ),
                "unexpected hume-target value: {s}"
            ),
            SteelVal::BoolV(false) => {}
            other => panic!("expected string or #f, got {other:?}"),
        }
    }

    #[test]
    fn hume_target_rejects_extra_args() {
        assert!(hume_target(&[SteelVal::StringV("x".into())]).is_err());
    }

    // ── verify-sha256! ─────────────────────────────────────────────────────

    #[test]
    fn verify_sha256_accepts_matching_digest_with_prefix() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let expected =
            "sha256:604f73953b84e48e552fea0b7fed0d938b038b5b1b18f7c10f5bb640ae5e9c40".to_string();
        let result = verify_sha256(&mut ctx, f.to_string_lossy().to_string(), expected);
        assert!(result.is_ok(), "expected ok, got {result:?}");
        assert!(f.exists(), "matching digest must not delete the file");
    }

    #[test]
    fn verify_sha256_accepts_bare_hex_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let expected =
            "604F73953B84E48E552FEA0B7FED0D938B038B5B1B18F7C10F5BB640AE5E9C40".to_string();
        assert!(verify_sha256(&mut ctx, f.to_string_lossy().to_string(), expected).is_ok());
    }

    #[test]
    fn verify_sha256_mismatch_deletes_file_and_raises() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = verify_sha256(
            &mut ctx,
            f.to_string_lossy().to_string(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("mismatch"),
            "expected mismatch error, got: {err}"
        );
        assert!(!f.exists(), "mismatched digest must delete the file");
    }

    #[test]
    fn verify_sha256_rejects_path_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        let outside = tmp.path().join("hume/plugins");
        fs::create_dir_all(&outside).unwrap();
        let f = outside.join("evil.gz");
        fs::write(&f, b"data").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = verify_sha256(
            &mut ctx,
            f.to_string_lossy().to_string(),
            "sha256:00".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    // ── unpack-gz / unpack-zip sandbox rejections ───────────────────────────
    //
    // Happy-path unpack behavior (round-trip content, exec bit, zip entries)
    // is covered by `hume-platform`'s own tests against the real system
    // tools; these tests pin the Steel-boundary sandbox contract only.

    #[test]
    fn unpack_gz_rejects_src_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let outside = tmp.path().join("hume/plugins/evil.gz");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, b"data").unwrap();
        let dest = servers.join("out-bin").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_gz(&mut ctx, outside.to_string_lossy().to_string(), dest).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_gz_rejects_dest_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let src = servers.join("rust-analyzer.gz");
        fs::write(&src, b"data").unwrap();
        let bad_dest = tmp.path().join("evil-bin").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_gz(&mut ctx, src.to_string_lossy().to_string(), bad_dest).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_zip_rejects_src_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let outside = tmp.path().join("hume/plugins/evil.zip");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, b"data").unwrap();
        let dest_dir = servers.join("out-dir").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err =
            unpack_zip(&mut ctx, outside.to_string_lossy().to_string(), dest_dir).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_zip_rejects_dest_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let src = servers.join("rust-analyzer.zip");
        fs::write(&src, b"data").unwrap();
        let bad_dest_dir = tmp.path().join("evil-dir").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err =
            unpack_zip(&mut ctx, src.to_string_lossy().to_string(), bad_dest_dir).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    // ── exe-on-path? ─────────────────────────────────────────────────────────

    #[test]
    fn exe_on_path_rejects_path_separator_names() {
        assert_eq!(
            exe_on_path(&[SteelVal::StringV("some/path".into())]).unwrap(),
            SteelVal::BoolV(false)
        );
    }

    #[test]
    fn exe_on_path_missing_tool_is_false() {
        assert_eq!(
            exe_on_path(&[SteelVal::StringV("definitely-not-a-real-tool-xyz".into())]).unwrap(),
            SteelVal::BoolV(false)
        );
    }
}
