use std::str::FromStr;

use super::*;

fn uri(s: &str) -> lsp_types::Uri {
    lsp_types::Uri::from_str(s).expect("valid uri")
}

fn edit(text: &str) -> lsp_types::TextEdit {
    lsp_types::TextEdit {
        range: lsp_types::Range::default(),
        new_text: text.to_string(),
    }
}

fn document(
    path: &str,
    version: Option<i32>,
) -> lsp_types::OptionalVersionedTextDocumentIdentifier {
    lsp_types::OptionalVersionedTextDocumentIdentifier {
        uri: uri(path),
        version,
    }
}

#[test]
fn changes_map_entries_carry_no_version() {
    let we = lsp_types::WorkspaceEdit {
        changes: Some([(uri("file:///a"), vec![edit("x")])].into_iter().collect()),
        ..Default::default()
    };
    assert_eq!(
        file_edits(we).unwrap(),
        [FileEdits {
            uri: uri("file:///a"),
            edits: NonEmpty::new(vec![edit("x")]).unwrap(),
            version: None,
        }]
    );
}

#[test]
fn document_changes_win_over_changes_and_strip_annotations() {
    let annotated = lsp_types::AnnotatedTextEdit {
        text_edit: edit("annotated"),
        annotation_id: "note".to_string(),
    };
    let we = lsp_types::WorkspaceEdit {
        changes: Some(
            [(uri("file:///ignored"), vec![edit("x")])]
                .into_iter()
                .collect(),
        ),
        document_changes: Some(lsp_types::DocumentChanges::Edits(vec![
            lsp_types::TextDocumentEdit {
                text_document: document("file:///a", Some(4)),
                edits: vec![
                    lsp_types::OneOf::Left(edit("plain")),
                    lsp_types::OneOf::Right(annotated),
                ],
            },
        ])),
        ..Default::default()
    };
    assert_eq!(
        file_edits(we).unwrap(),
        [FileEdits {
            uri: uri("file:///a"),
            edits: NonEmpty::new(vec![edit("plain"), edit("annotated")]).unwrap(),
            version: Some(4),
        }]
    );
}

#[test]
fn a_file_operation_refuses_the_whole_edit() {
    let we = lsp_types::WorkspaceEdit {
        document_changes: Some(lsp_types::DocumentChanges::Operations(vec![
            lsp_types::DocumentChangeOperation::Edit(lsp_types::TextDocumentEdit {
                text_document: document("file:///a", None),
                edits: vec![lsp_types::OneOf::Left(edit("x"))],
            }),
            lsp_types::DocumentChangeOperation::Op(lsp_types::ResourceOp::Delete(
                lsp_types::DeleteFile {
                    uri: uri("file:///b"),
                    options: None,
                },
            )),
        ])),
        ..Default::default()
    };
    assert_eq!(
        file_edits(we).unwrap_err(),
        "workspace edit contains an unsupported file operation (create/rename/delete)"
    );
}

#[test]
fn an_empty_edit_has_no_files() {
    assert_eq!(file_edits(lsp_types::WorkspaceEdit::default()).unwrap(), []);
}

#[test]
fn a_file_with_no_edits_is_left_out() {
    let changes = lsp_types::WorkspaceEdit {
        changes: Some(
            [
                (uri("file:///empty"), vec![]),
                (uri("file:///kept"), vec![edit("x")]),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    };
    let document_changes = lsp_types::WorkspaceEdit {
        document_changes: Some(lsp_types::DocumentChanges::Edits(vec![
            lsp_types::TextDocumentEdit {
                text_document: document("file:///empty", Some(1)),
                edits: vec![],
            },
            lsp_types::TextDocumentEdit {
                text_document: document("file:///kept", Some(1)),
                edits: vec![lsp_types::OneOf::Left(edit("x"))],
            },
        ])),
        ..Default::default()
    };
    for we in [changes, document_changes] {
        let uris: Vec<_> = file_edits(we).unwrap().into_iter().map(|f| f.uri).collect();
        assert_eq!(uris, [uri("file:///kept")]);
    }
}
