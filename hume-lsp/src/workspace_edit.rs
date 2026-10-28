//! Flattening a `WorkspaceEdit` into one plain edit list per file.

use hume_core::NonEmpty;

/// One file's edits from a `WorkspaceEdit`: plain `TextEdit`s
/// (annotations stripped: HUME has no change-annotation UI), and the
/// version to check the document against, if the edit came from a
/// versioned `documentChanges` entry.
#[derive(Debug, PartialEq)]
pub struct FileEdits {
    pub uri: lsp_types::Uri,
    pub edits: NonEmpty<lsp_types::TextEdit>,
    pub version: Option<i32>,
}

/// `documentChanges` takes precedence over `changes` when both are present
/// (LSP spec). A file with no edits changes nothing, so it is left out.
/// `DocumentChangeOperation::Op` (create/rename/delete file) has
/// no HUME equivalent: errors rather than dropping a file
/// operation the caller expected to happen.
pub fn file_edits(we: lsp_types::WorkspaceEdit) -> Result<Vec<FileEdits>, String> {
    if let Some(doc_changes) = we.document_changes {
        match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => {
                Ok(edits.into_iter().filter_map(document_edit).collect())
            }
            lsp_types::DocumentChanges::Operations(ops) => ops
                .into_iter()
                .filter_map(|op| match op {
                    lsp_types::DocumentChangeOperation::Edit(tde) => document_edit(tde).map(Ok),
                    lsp_types::DocumentChangeOperation::Op(_) => Some(Err(
                        "workspace edit contains an unsupported file operation (create/rename/delete)"
                            .to_string(),
                    )),
                })
                .collect(),
        }
    } else if let Some(changes) = we.changes {
        Ok(changes
            .into_iter()
            .filter_map(|(uri, edits)| {
                Some(FileEdits {
                    uri,
                    edits: NonEmpty::new(edits)?,
                    version: None,
                })
            })
            .collect())
    } else {
        Ok(Vec::new())
    }
}

/// `tde`'s edits, or `None` when it has none.
fn document_edit(tde: lsp_types::TextDocumentEdit) -> Option<FileEdits> {
    let edits = tde
        .edits
        .into_iter()
        .map(|edit| match edit {
            lsp_types::OneOf::Left(te) => te,
            lsp_types::OneOf::Right(ate) => ate.text_edit,
        })
        .collect();
    Some(FileEdits {
        uri: tde.text_document.uri,
        edits: NonEmpty::new(edits)?,
        version: tde.text_document.version,
    })
}

#[cfg(test)]
mod tests;
