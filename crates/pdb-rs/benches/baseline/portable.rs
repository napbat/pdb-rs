//! The Portable PDB scenarios: read, rebuild and edit.

use std::path::Path;

use pdb_rs::portable::debug::{MethodDebugInformation, SequencePoints, ValidateOptions};
use pdb_rs::portable::metadata::{
    CustomDebugInformationRow, DocumentId, DocumentRow, ImportScopeRow, LocalConstantRow,
    LocalScopeRow, LocalVariableRow, MethodDebugInformationRow, StateMachineMethodRow, TableRow,
};
use pdb_rs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

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
    mut visit: impl FnMut(&Pdb<&[u8]>, T) -> pdb_rs::Result<()>,
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
        std::hint::black_box(row?.1.decode()?);
        count += 1;
    }
    for document in pdb.documents() {
        let (id, document) = document?;
        std::hint::black_box(pdb.document_source(id, &document, MAX_EMBEDDED_SOURCE)?);
        count += 1;
    }
    for method in pdb.method_debug_informations() {
        let (_, method) = method?;
        if let Some(points) = method.decode()? {
            for point in points {
                std::hint::black_box(point?);
                count += 1;
            }
        }
    }
    for scope in pdb.local_scopes() {
        let (_, scope) = scope?;
        count += u64::try_from(scope.variables.len())?;
        for constant in &scope.constants {
            std::hint::black_box(constant.decode()?);
            count += 1;
        }
    }
    for scope in pdb.import_scopes() {
        let (_, scope) = scope?;
        count += u64::try_from(std::hint::black_box(scope.imports).len())?;
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

/// Add [`LINE_SHIFT`] to the lines of the sequence points of `method`. Return `false` if the
/// method has no sequence points.
fn shift_lines(method: &mut MethodDebugInformation<'static>) -> pdb_rs::Result<bool> {
    let Some(decoded) = method.decode()? else {
        return Ok(false);
    };
    let signature = decoded.local_signature();
    let form = decoded.local_signature_form();
    let mut points = decoded.collect::<pdb_rs::Result<Vec<_>>>()?;
    for point in &mut points {
        if let Some(span) = &mut point.span {
            span.start_line += LINE_SHIFT;
            span.end_line += LINE_SHIFT;
        }
    }
    *method = SequencePoints::encode_parts(signature, form, &points)?;
    Ok(true)
}

/// Append [`RENAME_SUFFIX`] to the name of document 1.
fn rename_document(editor: &mut PdbEditor<'_, &[u8]>) -> pdb_rs::Result<()> {
    if let Some(document) = editor.documents_mut()?.get_mut(DocumentId(1)) {
        let mut name = std::mem::take(&mut document.name).into_vec();
        name.extend_from_slice(RENAME_SUFFIX);
        document.name = name.into();
    }
    Ok(())
}

/// Plan and write the edit with the default options. Return the output size.
fn write(editor: &PdbEditor<'_, &[u8]>) -> Outcome<u64> {
    let output = editor.plan(EditOptions::default())?.to_vec()?;
    Ok(u64::try_from(output.len())?)
}

/// Shift the lines of all sequence points, rename document 1 and write the PDB with the
/// content-derived ID.
pub(crate) fn edit(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
        for method in editor.methods_mut()?.values_mut() {
            shift_lines(method)?;
        }
        rename_document(&mut editor)?;
        write(&editor)
    })
}

/// Rename document 1 and write the PDB with the content-derived ID.
pub(crate) fn edit_document(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
        rename_document(&mut editor)?;
        write(&editor)
    })
}

/// Shift the lines of the first method with sequence points and write the PDB with the
/// content-derived ID.
pub(crate) fn edit_method(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
        for method in editor.methods_mut()?.values_mut() {
            if shift_lines(method)? {
                break;
            }
        }
        write(&editor)
    })
}
