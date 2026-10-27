//! Decode the debug table blobs, encode them again and validate the tables.
//!
//! Each decoded blob goes through its encoder. The example counts the blobs that encode to the
//! same bytes and prints each blob that encodes to other bytes, with the reason. It decodes the
//! embedded source of each document and checks it against the document hash. A source that
//! does not match its SHA-256 hash stops the example with an error.

use std::collections::BTreeMap;

use pdb_rs::io::ReadAt;
use pdb_rs::portable::Pdb;
use pdb_rs::portable::debug::{
    Document, DocumentName, HashAlgorithm, Imports, Language, LocalConstantSignature,
    LocalSignatureForm, Outcome as RuleOutcome, RawImport, SequencePoints, ValidateOptions,
    split_document_name,
};
use pdb_rs::portable::embedded::{EmbeddedFormat, SourceCheck};
use pdb_rs::portable::metadata::{
    BlobOffset, DocumentId, DocumentRow, ImportScopeRow, LocalScopeRow,
};

use crate::{Counts, Outcome};

/// The largest embedded source that the example decodes.
const MAX_EMBEDDED_SOURCE: usize = 64 << 20;

/// Decode, encode and validate all debug table data of `pdb`.
pub fn debug_tables(pdb: &Pdb<impl ReadAt>, counts: &mut Counts) -> Outcome {
    documents(pdb, counts)?;
    sequence_points(pdb, counts)?;
    scopes(pdb, counts)?;
    imports(pdb, counts)?;
    for method in pdb.state_machine_methods() {
        method?;
        counts.add("state machine methods", 1);
    }
    let report = pdb.validate_debug(ValidateOptions::default())?;
    for rule in report.rules() {
        match rule.outcome {
            RuleOutcome::Checked { items } => {
                println!("validated {:?}: {items} items", rule.rule);
            }
            RuleOutcome::NotChecked { reason } => {
                println!("not validated {:?}: {reason}", rule.rule);
            }
        }
    }
    Ok(())
}

/// Count a blob that encodes to the same bytes, or print the reason of a difference.
fn compare(counts: &mut Counts, what: &str, encoded: &[u8], original: &[u8], reason: &str) {
    if encoded == original {
        counts.add("blobs encoded to the same bytes", 1);
    } else {
        counts.add("blobs encoded to other bytes", 1);
        println!("encoded to other bytes: {what}: {reason}");
    }
}

/// Decode the document names and check the Roslyn split of each name.
fn documents(pdb: &Pdb<impl ReadAt>, counts: &mut Counts) -> Outcome {
    for document in pdb.documents() {
        let (id, document) = document?;
        counts.add("documents", 1);
        if document
            .language
            .as_ref()
            .and_then(Language::from_guid)
            .is_some()
        {
            counts.add("documents with a defined language", 1);
        }
        if document
            .hash_algorithm
            .as_ref()
            .and_then(HashAlgorithm::from_guid)
            .is_some()
        {
            counts.add("documents with a defined hash algorithm", 1);
        }
        embedded_source(pdb, id, &document, counts)?;
        let row: DocumentRow = pdb.row(id)?;
        let blob = pdb.blob(row.name)?;
        let name = DocumentName::parse(&blob)?;
        let parts: Vec<BlobOffset> = name.parts().collect();
        let mut texts = BTreeMap::new();
        for &part in &parts {
            texts.insert(pdb.blob(part)?, part);
        }
        let split = split_document_name(&document.name);
        let separator = split.separator();
        let split: Vec<&[u8]> = split.collect();
        let what = format!("Document row {}", id.0);
        let same_split = name.separator() == Some(separator)
            && split.len() == parts.len()
            && split
                .iter()
                .zip(&parts)
                .all(|(text, part)| texts.get(*text) == Some(part));
        if same_split {
            let mut encoded = Vec::new();
            DocumentName::encode_parts(Some(separator), &parts, &mut encoded)?;
            compare(counts, &what, &encoded, &blob, "compressed part offsets");
        } else {
            compare(
                counts,
                &what,
                &[],
                &blob,
                "the separator or the parts are not the Roslyn split",
            );
        }
    }
    Ok(())
}

/// Decode the embedded source of `document` and check it against the document hash.
fn embedded_source(
    pdb: &Pdb<impl ReadAt>,
    id: DocumentId,
    document: &Document<'_>,
    counts: &mut Counts,
) -> Outcome {
    let Some(source) = pdb.document_source(id, document, MAX_EMBEDDED_SOURCE)? else {
        return Ok(());
    };
    counts.add(
        match source.format() {
            EmbeddedFormat::Raw => "embedded sources (raw)",
            EmbeddedFormat::Deflate => "embedded sources (DEFLATE)",
            _ => "embedded sources (other format)",
        },
        1,
    );
    match source.check() {
        SourceCheck::Verified => counts.add("embedded sources that match their SHA-256 hash", 1),
        SourceCheck::NotVerified => counts.add(
            "embedded sources not verified (hash algorithm is not SHA-256)",
            1,
        ),
        SourceCheck::Mismatch => {
            return Err(format!(
                "the embedded source of Document row {} does not match its SHA-256 hash",
                id.0
            )
            .into());
        }
        _ => counts.add("embedded sources with another check result", 1),
    }
    Ok(())
}

/// Decode the sequence points of each method and encode them again.
fn sequence_points(pdb: &Pdb<impl ReadAt>, counts: &mut Counts) -> Outcome {
    for method in pdb.method_debug_informations() {
        let (id, method) = method?;
        counts.add("methods", 1);
        let Some(decoded) = method.decode()? else {
            continue;
        };
        counts.add("methods with sequence points", 1);
        let signature = decoded.local_signature();
        let form = decoded.local_signature_form();
        if form == LocalSignatureForm::Token {
            counts.add(
                "sequence point headers with a LocalSignature token (F# form)",
                1,
            );
        }
        let points = decoded.collect::<Result<Vec<_>, _>>()?;
        let hidden = points.iter().filter(|point| point.is_hidden()).count();
        counts.add("sequence points", points.len());
        counts.add("hidden sequence points", hidden);
        let changes = points
            .windows(2)
            .filter(|pair| pair[0].document != pair[1].document)
            .count();
        counts.add("sequence point document changes", changes);
        // Keep the header form of the decoded blob, so an F# blob also gives the same bytes.
        let encoded = SequencePoints::encode_parts(signature, form, &points)?;
        let what = format!("MethodDebugInformation row {}", id.0);
        let original: &[u8] = &method.sequence_points;
        if encoded.document == method.document {
            compare(
                counts,
                &what,
                &encoded.sequence_points,
                original,
                "the record order or the integer sizes are not the Roslyn form",
            );
        } else {
            compare(
                counts,
                &what,
                &[],
                original,
                "the Document column is not the Roslyn form",
            );
        }
    }
    Ok(())
}

/// Read the scopes, variables and constants, and encode each constant signature again.
fn scopes(pdb: &Pdb<impl ReadAt>, counts: &mut Counts) -> Outcome {
    for scope in pdb.local_scopes() {
        let (scope_id, scope) = scope?;
        counts.add("local scopes", 1);
        counts.add("local variables", scope.variables.len());
        let first_constant = pdb.row::<LocalScopeRow>(scope_id)?.constant_list.0;
        for (row, constant) in (first_constant..).zip(&scope.constants) {
            let signature = constant.decode()?;
            counts.add("local constants", 1);
            let modifiers: Vec<_> = signature.modifiers().collect();
            counts.add("local constant custom modifiers", modifiers.len());
            let mut encoded = Vec::new();
            LocalConstantSignature::encode_parts(&modifiers, signature.value(), &mut encoded)?;
            let what = format!("LocalConstant row {row}");
            compare(
                counts,
                &what,
                &encoded,
                &constant.signature,
                "integer sizes",
            );
        }
    }
    Ok(())
}

/// Decode the imports of each import scope and encode them again.
fn imports(pdb: &Pdb<impl ReadAt>, counts: &mut Counts) -> Outcome {
    for scope in pdb.rows::<ImportScopeRow>() {
        let (id, row) = scope?;
        counts.add("import scopes", 1);
        let blob = pdb.blob(row.imports)?;
        let imports = Imports::parse(&blob).collect::<Result<Vec<RawImport>, _>>()?;
        for import in &imports {
            for text in [import.alias(), import.namespace()].into_iter().flatten() {
                pdb.blob(text)?;
            }
            counts.add(import_kind(import), 1);
        }
        let mut encoded = Vec::new();
        Imports::encode_parts(&imports, &mut encoded)?;
        let what = format!("ImportScope row {}", id.0);
        compare(counts, &what, &encoded, &blob, "integer sizes");
    }
    Ok(())
}

/// Return the count name of an import kind.
fn import_kind(import: &RawImport) -> &'static str {
    match import {
        RawImport::Namespace { .. } => "imports of kind 1 (namespace)",
        RawImport::AssemblyNamespace { .. } => "imports of kind 2 (assembly namespace)",
        RawImport::Type { .. } => "imports of kind 3 (type)",
        RawImport::XmlNamespace { .. } => "imports of kind 4 (XML namespace)",
        RawImport::AssemblyReferenceAlias { .. } => "imports of kind 5 (assembly reference alias)",
        RawImport::AliasAssemblyReference { .. } => "imports of kind 6 (alias assembly reference)",
        RawImport::AliasNamespace { .. } => "imports of kind 7 (alias namespace)",
        RawImport::AliasAssemblyNamespace { .. } => "imports of kind 8 (alias assembly namespace)",
        RawImport::AliasType { .. } => "imports of kind 9 (alias type)",
        _ => "imports of another kind",
    }
}
