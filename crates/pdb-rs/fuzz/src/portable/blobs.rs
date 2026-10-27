//! The debug table blobs: document names, sequence points, local constant signatures,
//! imports and compressed integers.
//!
//! Each round trip decodes a blob, encodes the value, and decodes the output again. The
//! second value must equal the first. The encoders write a canonical form, so the
//! output can differ from the input bytes.

use pdb_rs::io::ReadAt;
use pdb_rs::portable::Pdb;
use pdb_rs::portable::compressed::{decode_signed, decode_unsigned, encode_signed, encode_unsigned};
use pdb_rs::portable::debug::{
    DocumentName, Imports, LocalConstantSignature, SequencePoint, SequencePoints, ValidateOptions,
    split_document_name,
};
use pdb_rs::portable::metadata::{BlobOffset, DocumentId, DocumentRow, ImportScopeRow};

use crate::input::Input;
use crate::{MAX_STREAM, encoded, ok};

/// The most items that one walk visits, to bound the time of one run.
const MAX_ITEMS: usize = 1 << 14;

/// Decode one blob. The first input byte selects the blob kind.
pub fn blob(data: &[u8]) {
    let mut input = Input::new(data);
    match input.u8() % 6 {
        0 => document_name(input.rest()),
        1 => {
            let document = DocumentId(input.u32());
            sequence_points(input.rest(), document);
        }
        2 => constant(input.rest()),
        3 => imports(input.rest()),
        4 => compressed(input.rest()),
        _ => {
            let split = split_document_name(input.rest());
            let _ = split.separator();
            let _ = split.take(MAX_ITEMS).count();
        }
    }
}

/// Round-trip a document name blob.
fn document_name(blob: &[u8]) {
    let Some(name) = ok(DocumentName::parse(blob)) else {
        return;
    };
    let parts: Vec<BlobOffset> = name.parts().take(MAX_ITEMS).collect();
    let Some(encoded) = ok(encoded(|out| {
        DocumentName::encode_parts(name.separator(), &parts, out)
    })) else {
        return;
    };
    let again = DocumentName::parse(&encoded).expect("an encoded document name decodes");
    assert_eq!(again.separator(), name.separator());
    assert!(again.parts().eq(parts.iter().copied()));
}

/// Round-trip a sequence point blob.
fn sequence_points(blob: &[u8], document: DocumentId) {
    let Some(decoded) = ok(SequencePoints::parse(blob, document)) else {
        return;
    };
    let signature = decoded.local_signature();
    let form = decoded.local_signature_form();
    let _ = decoded.initial_document();
    let Some(points) = decoded
        .take(MAX_ITEMS)
        .collect::<pdb_rs::Result<Vec<SequencePoint>>>()
        .ok()
    else {
        return;
    };
    let _ = points.iter().filter(|point| point.is_hidden()).count();
    let Some(encoded) = ok(SequencePoints::encode_parts(signature, form, &points)) else {
        return;
    };
    if points.is_empty() {
        return;
    }
    let again = SequencePoints::parse(&encoded.sequence_points, encoded.document)
        .expect("encoded sequence points decode");
    assert_eq!(again.local_signature(), signature);
    assert_eq!(again.local_signature_form(), form);
    let again: Vec<SequencePoint> = again
        .collect::<pdb_rs::Result<_>>()
        .expect("encoded sequence points decode");
    assert_eq!(again, points);
}

/// Round-trip a local constant signature blob.
fn constant(blob: &[u8]) {
    let Some(signature) = ok(LocalConstantSignature::parse(blob)) else {
        return;
    };
    let modifiers: Vec<_> = signature.modifiers().take(MAX_ITEMS).collect();
    let Some(encoded) = ok(encoded(|out| {
        LocalConstantSignature::encode_parts(&modifiers, signature.value(), out)
    })) else {
        return;
    };
    let again = LocalConstantSignature::parse(&encoded).expect("an encoded constant decodes");
    assert!(again.modifiers().eq(modifiers.iter().copied()));
    let mut twice = Vec::new();
    LocalConstantSignature::encode_parts(&modifiers, again.value(), &mut twice)
        .expect("a decoded constant encodes");
    assert_eq!(twice, encoded);
}

/// Round-trip an imports blob.
fn imports(blob: &[u8]) {
    let Some(imports) = Imports::parse(blob)
        .take(MAX_ITEMS)
        .collect::<pdb_rs::Result<Vec<_>>>()
        .ok()
    else {
        return;
    };
    for import in &imports {
        let _ = (import.kind(), import.alias(), import.assembly());
        let _ = (import.namespace(), import.target_type());
    }
    let Some(encoded) = ok(encoded(|out| Imports::encode_parts(&imports, out))) else {
        return;
    };
    let again: Vec<_> = Imports::parse(&encoded)
        .collect::<pdb_rs::Result<_>>()
        .expect("encoded imports decode");
    assert_eq!(again, imports);
}

/// Round-trip compressed integers.
fn compressed(bytes: &[u8]) {
    if let Some((value, rest)) = ok(decode_unsigned(bytes)) {
        assert!(rest.len() < bytes.len());
        let encoded = encode_unsigned(value).expect("a decoded value encodes");
        let (again, tail) = decode_unsigned(encoded.as_bytes()).expect("an encoded value decodes");
        assert_eq!((again, tail.len()), (value, 0));
    }
    if let Some((value, _)) = ok(decode_signed(bytes)) {
        let encoded = encode_signed(value).expect("a decoded value encodes");
        let (again, tail) = decode_signed(encoded.as_bytes()).expect("an encoded value decodes");
        assert_eq!((again, tail.len()), (value, 0));
    }
    let mut input = Input::new(bytes);
    let word = input.u32();
    if let Some(encoded) = ok(encode_unsigned(word)) {
        assert_eq!(
            ok(decode_unsigned(encoded.as_bytes())).map(|v| v.0),
            Some(word)
        );
    }
    let signed = i32::from_le_bytes(word.to_le_bytes());
    if let Some(encoded) = ok(encode_signed(signed)) {
        assert_eq!(
            ok(decode_signed(encoded.as_bytes())).map(|v| v.0),
            Some(signed)
        );
    }
}

/// Decode the debug table rows of a Portable PDB through the typed accessors.
pub(crate) fn walk_debug<S: ReadAt>(pdb: &Pdb<S>) {
    for document in pdb.documents().take(MAX_ITEMS) {
        let Some((id, document)) = ok(document) else {
            break;
        };
        let _ = (document.name.len(), document.hash.len(), document.language);
        let _ = document.hash_algorithm;
        if let Some(Some(source)) = ok(pdb.document_source(id, &document, MAX_STREAM)) {
            let _ = (source.format(), source.check(), source.content().len());
        }
        let _ = pdb.embedded_source_row(id);
        if let Some(row) = ok(pdb.row::<DocumentRow>(id)) {
            if let Some(blob) = ok(pdb.blob(row.name)) {
                document_name(&blob);
            }
            let _ = pdb.document_name(row.name);
        }
    }
    for method in pdb.method_debug_informations().take(MAX_ITEMS) {
        let Some((_, method)) = ok(method) else {
            break;
        };
        sequence_points(&method.sequence_points, method.document);
        if let Some(Some(points)) = ok(method.decode()) {
            let _ = points.take(MAX_ITEMS).count();
        }
    }
    for scope in pdb.local_scopes().take(MAX_ITEMS) {
        let Some((_, scope)) = ok(scope) else {
            break;
        };
        let _ = (scope.end_offset(), scope.method, scope.import_scope);
        for variable in scope.variables.iter().take(MAX_ITEMS) {
            let _ = variable.is_debugger_hidden();
        }
        for local in scope.constants.iter().take(MAX_ITEMS) {
            let _ = local.decode();
            constant(&local.signature);
        }
    }
    for scope in pdb.import_scopes().take(MAX_ITEMS) {
        if scope.is_err() {
            break;
        }
    }
    for row in pdb.rows::<ImportScopeRow>().take(MAX_ITEMS) {
        let Some((_, row)) = ok(row) else {
            break;
        };
        let _ = row.parent;
        if let Some(blob) = ok(pdb.blob(row.imports)) {
            imports(&blob);
        }
    }
    for method in pdb.state_machine_methods().take(MAX_ITEMS) {
        if method.is_err() {
            break;
        }
    }
    if let Some(report) = ok(pdb.validate_debug(ValidateOptions::default())) {
        let _ = report.rules().len();
    }
}
