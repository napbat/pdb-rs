//! Decode the debug table blobs, encode them again and validate the tables.
//!
//! Each decoded blob goes through its encoder. The example counts the blobs that encode to the
//! same bytes and prints each blob that encodes to other bytes, with the reason. It decodes the
//! embedded source of each document and checks it against the document hash. A source that
//! does not match its SHA-256 hash stops the example with an error.

use std::collections::BTreeMap;

use pdbrs::portable::Pdb;
use pdbrs::portable::debug::{
    Document, DocumentName, Import, LocalConstantSignature, LocalSignatureForm,
    Outcome as RuleOutcome, ValidateOptions, encode_imports, encode_sequence_points,
    split_document_name,
};
use pdbrs::portable::embedded::{EmbeddedFormat, SourceCheck};
use pdbrs::portable::metadata::BlobOffset;

use crate::{Counts, Outcome};

/// The largest embedded source that the example decodes.
const MAX_EMBEDDED_SOURCE: usize = 64 << 20;

/// Decode, encode and validate all debug table data of `pdb`.
pub fn debug_tables(pdb: &Pdb<&[u8]>, counts: &mut Counts) -> Outcome {
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
fn documents(pdb: &Pdb<&[u8]>, counts: &mut Counts) -> Outcome {
    for document in pdb.documents() {
        let document = document?;
        counts.add("documents", 1);
        if document.language().is_some() {
            counts.add("documents with a defined language", 1);
        }
        if document.hash_algorithm().is_some() {
            counts.add("documents with a defined hash algorithm", 1);
        }
        embedded_source(pdb, &document, counts)?;
        let blob = pdb.blob(document.row().name)?;
        let name = DocumentName::decode(&blob)?;
        let parts: Vec<BlobOffset> = name.parts().collect();
        let mut texts = BTreeMap::new();
        for &part in &parts {
            texts.insert(pdb.blob(part)?, part);
        }
        let split = split_document_name(document.name());
        let separator = split.separator();
        let split: Vec<&[u8]> = split.collect();
        let what = format!("Document row {}", document.id().get());
        let same_split = name.separator() == Some(separator)
            && split.len() == parts.len()
            && split
                .iter()
                .zip(&parts)
                .all(|(text, part)| texts.get(*text) == Some(part));
        if same_split {
            let encoded = DocumentName::encode(Some(separator), &parts)?;
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
fn embedded_source(pdb: &Pdb<&[u8]>, document: &Document, counts: &mut Counts) -> Outcome {
    let Some(source) = pdb.document_source(document, MAX_EMBEDDED_SOURCE)? else {
        return Ok(());
    };
    counts.add(
        match source.format() {
            EmbeddedFormat::Raw => "embedded sources (raw)",
            EmbeddedFormat::Deflate => "embedded sources (DEFLATE)",
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
                document.id().get()
            )
            .into());
        }
    }
    Ok(())
}

/// Decode the sequence points of each method and encode them again.
fn sequence_points(pdb: &Pdb<&[u8]>, counts: &mut Counts) -> Outcome {
    for method in pdb.method_debug_informations() {
        let method = method?;
        counts.add("methods", 1);
        let Some(decoded) = method.sequence_points()? else {
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
        let points = decoded
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| method.blob_error(error))?;
        let hidden = points.iter().filter(|point| point.is_hidden()).count();
        counts.add("sequence points", points.len());
        counts.add("hidden sequence points", hidden);
        let changes = points
            .windows(2)
            .filter(|pair| pair[0].document != pair[1].document)
            .count();
        counts.add("sequence point document changes", changes);
        // Keep the header form of the decoded blob, so an F# blob also gives the same bytes.
        let encoded = encode_sequence_points(signature, form, &points)?;
        let what = format!("MethodDebugInformation row {}", method.id().get());
        let original = method.sequence_points_blob();
        if encoded.document == method.document() {
            compare(
                counts,
                &what,
                &encoded.blob,
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
fn scopes(pdb: &Pdb<&[u8]>, counts: &mut Counts) -> Outcome {
    for scope in pdb.local_scopes() {
        let scope = scope?;
        counts.add("local scopes", 1);
        for id in scope.variables() {
            pdb.local_variable(id)?;
            counts.add("local variables", 1);
        }
        for id in scope.constants() {
            let constant = pdb.local_constant(id)?;
            let signature = constant.signature()?;
            counts.add("local constants", 1);
            let modifiers: Vec<_> = signature.modifiers().collect();
            counts.add("local constant custom modifiers", modifiers.len());
            let encoded = LocalConstantSignature::encode(&modifiers, &signature.value())?;
            let what = format!("LocalConstant row {}", id.get());
            compare(
                counts,
                &what,
                &encoded,
                constant.signature_blob(),
                "integer sizes",
            );
        }
    }
    Ok(())
}

/// Decode the imports of each import scope and encode them again.
fn imports(pdb: &Pdb<&[u8]>, counts: &mut Counts) -> Outcome {
    for scope in pdb.import_scopes() {
        let scope = scope?;
        counts.add("import scopes", 1);
        let imports = scope
            .imports()
            .collect::<Result<Vec<Import>, _>>()
            .map_err(|error| scope.blob_error(error))?;
        for import in &imports {
            for text in [import.alias(), import.namespace()].into_iter().flatten() {
                pdb.blob(text)?;
            }
            counts.add(import_kind(import), 1);
        }
        let encoded = encode_imports(&imports)?;
        let what = format!("ImportScope row {}", scope.id().get());
        compare(
            counts,
            &what,
            &encoded,
            scope.imports_blob(),
            "integer sizes",
        );
    }
    Ok(())
}

/// Return the count name of an import kind.
fn import_kind(import: &Import) -> &'static str {
    match import {
        Import::Namespace { .. } => "imports of kind 1 (namespace)",
        Import::AssemblyNamespace { .. } => "imports of kind 2 (assembly namespace)",
        Import::Type { .. } => "imports of kind 3 (type)",
        Import::XmlNamespace { .. } => "imports of kind 4 (XML namespace)",
        Import::AssemblyReferenceAlias { .. } => "imports of kind 5 (assembly reference alias)",
        Import::AliasAssemblyReference { .. } => "imports of kind 6 (alias assembly reference)",
        Import::AliasNamespace { .. } => "imports of kind 7 (alias namespace)",
        Import::AliasAssemblyNamespace { .. } => "imports of kind 8 (alias assembly namespace)",
        Import::AliasType { .. } => "imports of kind 9 (alias type)",
    }
}
