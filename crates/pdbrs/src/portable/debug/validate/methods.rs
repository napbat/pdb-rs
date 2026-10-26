//! Rules of the `Document` and `MethodDebugInformation` tables.

use alloc::collections::BTreeSet;

use super::{Rule, ValidateOptions, ValidationReport, count, external_exists};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::document::check_hash;
use crate::portable::debug::rows::row_error;
use crate::portable::metadata::{MetadataStream, TableId};
use crate::{Error, ErrorKind, Result};

/// Check the document names and hashes.
pub(super) fn documents<S: ReadAt>(pdb: &Pdb<S>, report: &mut ValidationReport) -> Result<()> {
    let mut names = BTreeSet::new();
    let mut hashes = 0;
    for document in pdb.documents() {
        let document = document?;
        check_hash(&document)?;
        if document.hash_algorithm().is_some() {
            hashes += 1;
        }
        let id = document.id().0;
        if !names.insert(document.name().to_vec()) {
            return Err(row_error(
                ErrorKind::InvalidFormat,
                "two documents have the same name",
                TableId::DOCUMENT,
                id,
                Some("Name"),
            ));
        }
    }
    report.checked(Rule::DocumentNames, count(names.len()));
    report.checked(Rule::DocumentHashes, hashes);
    Ok(())
}

/// Check the `MethodDebugInformation` row count against the `MethodDef` row count.
pub(super) fn method_count<S: ReadAt>(
    pdb: &Pdb<S>,
    options: ValidateOptions,
    report: &mut ValidationReport,
) -> Result<()> {
    let header = pdb.pdb_stream();
    let referenced = header.referenced_tables() & (1 << TableId::METHOD_DEF.raw()) != 0;
    let expected = options
        .method_def_count()
        .or_else(|| referenced.then(|| header.external_row_count(TableId::METHOD_DEF)));
    let Some(expected) = expected else {
        report.not_checked(
            Rule::MethodDebugInformationCount,
            "no MethodDef row count in the options or the #Pdb stream",
        );
        return Ok(());
    };
    let rows = pdb.tables().row_count(TableId::METHOD_DEBUG_INFORMATION);
    if rows != 0 && rows != expected {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("MethodDebugInformation row count is not the MethodDef row count")
            .with_portable(
                PortableLocation::new()
                    .with_stream(MetadataStream::Tables)
                    .with_table(TableId::METHOD_DEBUG_INFORMATION),
            ));
    }
    report.checked(Rule::MethodDebugInformationCount, u64::from(rows));
    Ok(())
}

/// Decode each sequence point blob and check its references.
pub(super) fn sequence_points<S: ReadAt>(
    pdb: &Pdb<S>,
    report: &mut ValidationReport,
) -> Result<()> {
    let documents = pdb.tables().row_count(TableId::DOCUMENT);
    let mut points = 0_u64;
    for method in pdb.method_debug_informations() {
        let method = method?;
        let Some(decoded) = method.sequence_points()? else {
            if !method.document().is_nil() {
                return Err(row_error(
                    ErrorKind::InvalidFormat,
                    "method without sequence points has a document",
                    TableId::METHOD_DEBUG_INFORMATION,
                    method.id().0,
                    Some("Document"),
                ));
            }
            continue;
        };
        let signature = decoded.local_signature();
        if !signature.is_nil()
            && !signature
                .token()
                .is_some_and(|token| external_exists(pdb, token))
        {
            return Err(method.blob_error(reference(
                "sequence point LocalSignature row does not exist",
            )));
        }
        for point in decoded {
            let point = point.map_err(|error| method.blob_error(error))?;
            if point.document.0 > documents {
                return Err(method.blob_error(reference("sequence point document does not exist")));
            }
            points += 1;
        }
    }
    report.checked(Rule::SequencePoints, points);
    Ok(())
}

/// Make an [`ErrorKind::InvalidReference`] error without a location.
pub(super) fn reference(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}
