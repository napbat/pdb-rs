//! The debug table blobs: document names, sequence points, local constant signatures,
//! imports and compressed integers.
//!
//! Each round trip decodes a blob, encodes the value, and decodes the output again. The
//! second value must equal the first. The encoders write a canonical form, so the
//! output can differ from the input bytes.

use pdbrs::io::ReadAt;
use pdbrs::portable::Pdb;
use pdbrs::portable::compressed::{decode_signed, decode_unsigned, encode_signed, encode_unsigned};
use pdbrs::portable::debug::{
    DocumentName, Imports, LocalConstantSignature, SequencePoint, SequencePoints, ValidateOptions,
    encode_imports, encode_sequence_points, split_document_name,
};
use pdbrs::portable::metadata::{BlobOffset, DocumentId};

use crate::input::Input;
use crate::{MAX_STREAM, ok};

/// The most items that one walk visits, to bound the time of one run.
const MAX_ITEMS: usize = 1 << 14;

/// Decode one blob. The first input byte selects the blob kind.
pub fn blob(data: &[u8]) {
    let mut input = Input::new(data);
    match input.u8() % 6 {
        0 => document_name(input.rest()),
        1 => {
            let document = DocumentId::new(input.u32());
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
    let Some(name) = ok(DocumentName::decode(blob)) else {
        return;
    };
    let parts: Vec<BlobOffset> = name.parts().take(MAX_ITEMS).collect();
    let Some(encoded) = ok(DocumentName::encode(name.separator(), &parts)) else {
        return;
    };
    let again = DocumentName::decode(&encoded).expect("an encoded document name decodes");
    assert_eq!(again.separator(), name.separator());
    assert!(again.parts().eq(parts.iter().copied()));
}

/// Round-trip a sequence point blob.
fn sequence_points(blob: &[u8], document: DocumentId) {
    let Some(decoded) = ok(SequencePoints::decode(blob, document)) else {
        return;
    };
    let signature = decoded.local_signature();
    let form = decoded.local_signature_form();
    let _ = decoded.initial_document();
    let Some(points) = decoded
        .take(MAX_ITEMS)
        .collect::<pdbrs::Result<Vec<SequencePoint>>>()
        .ok()
    else {
        return;
    };
    let _ = points.iter().filter(|point| point.is_hidden()).count();
    let Some(encoded) = ok(encode_sequence_points(signature, form, &points)) else {
        return;
    };
    if points.is_empty() {
        return;
    }
    let again = SequencePoints::decode(&encoded.blob, encoded.document)
        .expect("encoded sequence points decode");
    assert_eq!(again.local_signature(), signature);
    assert_eq!(again.local_signature_form(), form);
    let again: Vec<SequencePoint> = again
        .collect::<pdbrs::Result<_>>()
        .expect("encoded sequence points decode");
    assert_eq!(again, points);
}

/// Round-trip a local constant signature blob.
fn constant(blob: &[u8]) {
    let Some(signature) = ok(LocalConstantSignature::decode(blob)) else {
        return;
    };
    let modifiers: Vec<_> = signature.modifiers().take(MAX_ITEMS).collect();
    let Some(encoded) = ok(LocalConstantSignature::encode(
        &modifiers,
        &signature.value(),
    )) else {
        return;
    };
    let again = LocalConstantSignature::decode(&encoded).expect("an encoded constant decodes");
    assert!(again.modifiers().eq(modifiers.iter().copied()));
    let twice = LocalConstantSignature::encode(&modifiers, &again.value())
        .expect("a decoded constant encodes");
    assert_eq!(twice, encoded);
}

/// Round-trip an imports blob.
fn imports(blob: &[u8]) {
    let Some(imports) = Imports::decode(blob)
        .take(MAX_ITEMS)
        .collect::<pdbrs::Result<Vec<_>>>()
        .ok()
    else {
        return;
    };
    for import in &imports {
        let _ = (import.kind(), import.alias(), import.assembly());
        let _ = (import.namespace(), import.target_type());
    }
    let Some(encoded) = ok(encode_imports(&imports)) else {
        return;
    };
    let again: Vec<_> = Imports::decode(&encoded)
        .collect::<pdbrs::Result<_>>()
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
        let Some(document) = ok(document) else {
            break;
        };
        let _ = (document.name(), document.hash(), document.language());
        let _ = document.hash_algorithm();
        if let Some(Some(source)) = ok(pdb.document_source(&document, MAX_STREAM)) {
            let _ = (source.format(), source.check(), source.content().len());
        }
        let _ = pdb.embedded_source_row(&document);
        if let Some(blob) = ok(pdb.blob(document.row().name)) {
            document_name(&blob);
        }
        let _ = pdb.document_name(document.row().name);
    }
    for method in pdb.method_debug_informations().take(MAX_ITEMS) {
        let Some(method) = ok(method) else {
            break;
        };
        sequence_points(method.sequence_points_blob(), method.document());
        if let Some(Some(points)) = ok(method.sequence_points()) {
            let _ = points.take(MAX_ITEMS).count();
        }
    }
    for scope in pdb.local_scopes().take(MAX_ITEMS) {
        let Some(scope) = ok(scope) else {
            break;
        };
        let _ = (scope.end_offset(), scope.method(), scope.import_scope());
        for id in scope.variables().take(MAX_ITEMS) {
            let _ = pdb.local_variable(id);
        }
        for id in scope.constants().take(MAX_ITEMS) {
            if let Some(local) = ok(pdb.local_constant(id)) {
                let _ = local.signature();
                constant(local.signature_blob());
            }
        }
    }
    for scope in pdb.import_scopes().take(MAX_ITEMS) {
        let Some(scope) = ok(scope) else {
            break;
        };
        let _ = scope.parent();
        imports(scope.imports_blob());
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
