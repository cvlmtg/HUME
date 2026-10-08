use std::path::Path;

/// The OSC 7 URL for `cwd`: `file://host/path`. A path that already names an
/// authority (a Windows UNC share) keeps it. `None` for a path that has no
/// URL (relative, or not UTF-8).
pub(in crate::editor) fn working_directory_url(host: &str, cwd: &Path) -> Option<String> {
    let uri = hume_lsp::uri::path_to_uri(cwd).ok()?;
    let rest = uri.as_str().strip_prefix("file://")?;
    Some(if rest.starts_with('/') {
        format!("file://{host}{rest}")
    } else {
        format!("file://{rest}")
    })
}
