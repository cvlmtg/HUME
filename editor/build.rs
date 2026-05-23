fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // editor/ is one level below the workspace root where .git/ lives
    let workspace = std::path::Path::new(&manifest).parent().unwrap();

    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=HUME_GIT_SHA={sha}");
    println!("cargo:rerun-if-changed={}", workspace.join(".git/HEAD").display());
    println!("cargo:rerun-if-changed={}", workspace.join(".git/logs/HEAD").display());
}
