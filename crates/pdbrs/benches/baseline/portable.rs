//! The Portable PDB scenarios: read, rebuild and edit.

use std::path::Path;

use pdbrs::portable::debug::ValidateOptions;
use pdbrs::portable::metadata::{
    CustomDebugInformationRow, DocumentId, DocumentRow, ImportScopeRow, LocalConstantRow,
    LocalScopeRow, LocalVariableRow, MethodDebugInformationRow, StateMachineMethodRow, TableRow,
};
use pdbrs::portable::{IdentityPolicy, Pdb};

use crate::Outcome;
use crate::measure::Bench;

/// The largest embedded source that the read decodes, as in the `portable` example.
const MAX_EMBEDDED_SOURCE: usize = 64 << 20;

/// The number of lines that the edit adds to each sequence point.
const LINE_SHIFT: u32 = 1000;

/// The suffix that the edit appends to the name of document 1.
const RENAME_SUFFIX: &[u8] = b".edited";

/// Read all rows of the table of `T` and return the row count.
fn rows<T: TableRow>(
    pdb: &Pdb<&[u8]>,
    mut visit: impl FnMut(&Pdb<&[u8]>, T) -> pdbrs::Result<()>,
) -> Outcome<u64> {
    let mut count = 0;
    for row in pdb.rows::<T>() {
        let (_, row) = row?;
        visit(pdb, row)?;
        count += 1;
    }
    Ok(count)
}

/// Read every row of the eight debug tables with the heap values that it refers to.
fn tables(pdb: &Pdb<&[u8]>) -> Outcome<u64> {
    let mut count = rows(pdb, |pdb, row: DocumentRow| {
        pdb.blob(row.name)?;
        pdb.guid(row.hash_algorithm)?;
        pdb.blob(row.hash)?;
        pdb.guid(row.language)?;
        Ok(())
    })?;
    count += rows(pdb, |pdb, row: MethodDebugInformationRow| {
        pdb.blob(row.sequence_points).map(drop)
    })?;
    count += rows(pdb, |_, _: LocalScopeRow| Ok(()))?;
    count += rows(pdb, |pdb, row: LocalVariableRow| {
        pdb.string(row.name).map(drop)
    })?;
    count += rows(pdb, |pdb, row: LocalConstantRow| {
        pdb.string(row.name)?;
        pdb.blob(row.signature).map(drop)
    })?;
    count += rows(pdb, |pdb, row: ImportScopeRow| {
        pdb.blob(row.imports).map(drop)
    })?;
    count += rows(pdb, |_, _: StateMachineMethodRow| Ok(()))?;
    count += rows(pdb, |pdb, row: CustomDebugInformationRow| {
        pdb.guid(row.kind)?;
        pdb.blob(row.value).map(drop)
    })?;
    Ok(count)
}

/// Decode the custom debug information values and the debug table blobs, and validate the
/// debug tables. Return the number of decoded values.
fn blobs(pdb: &Pdb<&[u8]>) -> Outcome<u64> {
    let mut count = 0;
    for row in pdb.custom_debug_informations() {
        std::hint::black_box(row?.decode()?);
        count += 1;
    }
    for document in pdb.documents() {
        let document = document?;
        std::hint::black_box(pdb.document_source(&document, MAX_EMBEDDED_SOURCE)?);
        count += 1;
    }
    for method in pdb.method_debug_informations() {
        let method = method?;
        if let Some(points) = method.sequence_points()? {
            for point in points {
                std::hint::black_box(point.map_err(|error| method.blob_error(error))?);
                count += 1;
            }
        }
    }
    for scope in pdb.local_scopes() {
        let scope = scope?;
        for id in scope.variables() {
            pdb.local_variable(id)?;
            count += 1;
        }
        for id in scope.constants() {
            std::hint::black_box(pdb.local_constant(id)?.signature()?);
            count += 1;
        }
    }
    for scope in pdb.import_scopes() {
        let scope = scope?;
        for import in scope.imports() {
            std::hint::black_box(import.map_err(|error| scope.blob_error(error))?);
            count += 1;
        }
    }
    for method in pdb.state_machine_methods() {
        method?;
        count += 1;
    }
    std::hint::black_box(pdb.validate_debug(ValidateOptions::default())?);
    Ok(count)
}

/// Parse the Portable PDB and read and decode all its debug data.
///
/// The count is the number of rows and decoded values.
pub(crate) fn read(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    bench.measure(|| {
        let pdb = Pdb::parse(&bytes)?;
        Ok(tables(&pdb)? + blobs(&pdb)?)
    })
}

/// Rebuild the Portable PDB through [`Pdb::to_parts`] with the content-derived ID.
pub(crate) fn rebuild(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let output = pdb
            .to_parts()?
            .plan(IdentityPolicy::ContentDerived)?
            .to_vec()?;
        Ok(u64::try_from(output.len())?)
    })
}

/// Shift the lines of all sequence points, rename document 1 and write the PDB with the
/// content-derived ID.
pub(crate) fn edit(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit()?;
        for (_, method) in editor.methods_mut() {
            for point in &mut method.sequence_points {
                if let Some(span) = &mut point.span {
                    span.start_line += LINE_SHIFT;
                    span.end_line += LINE_SHIFT;
                }
            }
        }
        if let Some(document) = editor.document_mut(DocumentId(1)) {
            document.name.extend_from_slice(RENAME_SUFFIX);
        }
        let output = editor.plan(IdentityPolicy::ContentDerived)?.to_vec()?;
        Ok(u64::try_from(output.len())?)
    })
}
