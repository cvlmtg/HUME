use std::path::Path;

/// The OSC 7 URL for `cwd`, naming this machine as the host. `None` for a
/// path that has no URL (relative, or not UTF-8).
pub(in crate::editor) fn working_directory_url(cwd: &Path) -> Option<String> {
    let host = gethostname::gethostname();
    hume_lsp::uri::path_to_file_url(cwd, &host.to_string_lossy()).ok()
}
