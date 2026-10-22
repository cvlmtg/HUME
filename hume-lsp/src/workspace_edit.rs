//! Flattening a `WorkspaceEdit` into one plain edit list per file.

/// One file's edits from a `WorkspaceEdit`: plain `TextEdit`s
/// (annotations stripped: HUME has no change-annotation UI), and the
/// version to check the document against, if the edit came from a
/// versioned `documentChanges` entry.
#[derive(Debug, PartialEq)]
pub struct FileEdits {
    pub uri: lsp_types::Uri,
    pub edits: Vec<lsp_types::TextEdit>,
    pub version: Option<i32>,
}

/// `documentChanges` takes precedence over `changes` when both are present
/// (LSP spec). `DocumentChangeOperation::Op` (create/rename/delete file) has
/// no HUME equivalent: errors rather than silently dropping a file
/// operation the caller expected to happen.
pub fn file_edits(we: lsp_types::WorkspaceEdit) -> Result<Vec<FileEdits>, String> {
    if let Some(doc_changes) = we.document_changes {
        match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => {
                Ok(edits.into_iter().map(document_edit).collect())
            }
            lsp_types::DocumentChanges::Operations(ops) => ops
                .into_iter()
                .map(|op| match op {
                    lsp_types::DocumentChangeOperation::Edit(tde) => Ok(document_edit(tde)),
                    lsp_types::DocumentChangeOperation::Op(_) => Err(
                        "workspace edit contains an unsupported file operation (create/rename/delete)"
                            .to_string(),
                    ),
                })
                .collect(),
        }
    } else if let Some(changes) = we.changes {
        Ok(changes
            .into_iter()
            .map(|(uri, edits)| FileEdits {
                uri,
                edits,
                version: None,
            })
            .collect())
    } else {
        Ok(Vec::new())
    }
}

fn document_edit(tde: lsp_types::TextDocumentEdit) -> FileEdits {
    FileEdits {
        uri: tde.text_document.uri,
        edits: tde
            .edits
            .into_iter()
            .map(|edit| match edit {
                lsp_types::OneOf::Left(te) => te,
                lsp_types::OneOf::Right(ate) => ate.text_edit,
            })
            .collect(),
        version: tde.text_document.version,
    }
}

#[cfg(test)]
mod tests;
