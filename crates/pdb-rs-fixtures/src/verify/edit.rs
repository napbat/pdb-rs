//! The `verify-edit` command.
//!
//! The command reads each Portable PDB of the fixture manifest (a standalone file or the
//! decoded `MPDB` payload of an image) with `Pdb::edit` and checks the pdb-rs editor against
//! the verifier (`System.Reflection.Metadata`):
//!
//! 1. A plan of the editor without a change has the original bytes.
//! 2. A new `PdbEditor` with a copy of all records and of the empty list forms writes new
//!    heaps. The decoded sections of its verifier dump are the same as those of the original.
//! 3. An edit renames document 1, moves each visible sequence point 1000 lines down, adds the
//!    local constant `pdb_rs_added` (`i4=42`) to the last local scope and adds a custom debug
//!    information value of an unknown kind to `MethodDef` 1. The decoded sections of its
//!    verifier dump are the expected change of the original sections. The custom debug
//!    information lines are compared without their row IDs.
//! 4. For a standalone fixture, the program prints the `stack` mode trace with the original,
//!    the built (step 2) and a line-moved PDB. The PDBs use the original PDB ID. The built PDB
//!    gives the same trace, and the line-moved PDB gives each line number plus 1000.
//! 5. After these steps, the section `change and removal cases:` applies one field change or
//!    one removal in each case of [`cases`] and checks the output with the verifier and the
//!    plan report.
//! 6. The section `task cases:` applies the editing tasks (see [`tasks`]) with the same
//!    checks.
//! 7. The section `remap cases:` remaps the document names with the source path remap (see
//!    [`remap`]) and checks the names, the unchanged sequence points and the report.
//!
//! The command writes the PDBs, the dumps and the traces to `target/verify-edit/`.

mod cases;
mod expect;
mod remap;
mod stack;
mod tasks;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use pdb_rs::io::ReadAt;
use pdb_rs::portable::custom::{CustomDebugInformation, CustomDebugKind, CustomDebugParent};
use pdb_rs::portable::debug::{
    ConstantValue, LocalConstant, LocalConstantSignature, PrimitiveConstant, SequencePoints,
};
use pdb_rs::portable::embedded::decode_embedded;
use pdb_rs::portable::metadata::{DocumentId, MetadataToken, TableId};
use pdb_rs::portable::write::ExternalRowCounts;
use pdb_rs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

use super::rebuild::{hex, yes};
use super::{
    Comparison, EMBEDDED_LIMIT, Input, Output, SHOWN_DIFFERENCES, comparisons, outputs,
    print_in_order, read,
};
use crate::jobs::Jobs;
use crate::tools::{self, verifier};

/// The number of lines that the edit adds to each visible sequence point.
const LINE_SHIFT: u32 = 1000;
/// The suffix that the edit adds to the name of document 1.
const RENAME_SUFFIX: &[u8] = b".edited";
/// The name of the added local constant.
const ADDED_CONSTANT: &[u8] = b"pdb_rs_added";
/// The value of the added local constant.
const ADDED_VALUE: i32 = 42;
/// The kind of the added custom debug information: `70646272-7365-4564-6974-2d636865636b`.
const ADDED_KIND: CustomDebugKind =
    CustomDebugKind::from_fields(0x7064_6272, 0x7365, 0x4564, *b"it-check");
/// The value of the added custom debug information.
const ADDED_INFORMATION: &[u8] = b"pdb-rs-edit";

/// Edit and build each Portable PDB fixture and compare the results with the verifier.
///
/// Return `false` if a check fails.
pub fn verify_edit(workspace: &Path, fixtures: &Path, jobs: &Jobs) -> Result<bool, String> {
    let assembly = tools::built_tool(workspace, verifier::NAME)?;
    let out = workspace.join("target").join("verify-edit");
    fs::create_dir_all(&out).map_err(|error| format!("cannot make {}: {error}", out.display()))?;
    let outputs = outputs(fixtures)?;
    let comparisons = comparisons(&outputs)?;
    if comparisons.is_empty() {
        return Err(
            "the fixture manifest has no portable-pdb or embedded-pdb-payload output".to_owned(),
        );
    }
    let passed = print_in_order(jobs, &comparisons, |comparison, log| {
        check(&assembly, comparison, &outputs, &out, log)
    });
    let change_removal_cases = cases::run(jobs, &assembly, &comparisons, &out);
    let task_cases = tasks::run(jobs, &assembly, workspace, &comparisons, &out);
    let remap_cases = remap::run(jobs, &assembly, fixtures, &comparisons, &out);
    println!(
        "wrote the PDBs, the dumps and the traces to {}",
        out.display()
    );
    Ok(passed && change_removal_cases && task_cases && remap_cases)
}

/// Format a pdb-rs error.
fn pdb_rs(error: impl std::fmt::Display) -> String {
    format!("pdb-rs: {error}")
}

/// Run the verifier and return its dump.
fn dump(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("cannot run the verifier: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "the verifier failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("the verifier output is not UTF-8: {error}"))
}

/// Read the source of a comparison and dump it with the verifier. Return the Portable PDB
/// bytes (the decoded payload for an embedded PDB) and the dump.
fn source(assembly: &Path, comparison: &Comparison) -> Result<(Vec<u8>, String), String> {
    let (original, mut command) = match &comparison.input {
        Input::Standalone(path) => (read(path)?, verifier::command(assembly, path)),
        Input::Embedded { payload, image } => (
            decode_embedded(&read(payload)?, EMBEDDED_LIMIT).map_err(pdb_rs)?,
            verifier::embedded_command(assembly, image),
        ),
    };
    Ok((original, dump(&mut command)?))
}

/// Write `bytes` to `out`, dump the file with the verifier and write the dump.
fn write_and_dump(assembly: &Path, out: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
    let path = out.join(format!("{name}.pdb"));
    fs::write(&path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let text = dump(&mut verifier::command(assembly, &path))?;
    let dump_path = out.join(format!("{name}.srm.txt"));
    fs::write(&dump_path, &text)
        .map_err(|error| format!("cannot write {}: {error}", dump_path.display()))?;
    Ok(text)
}

/// Write the differences of two line lists to `log` and return `true` if they are equal.
fn same(log: &mut String, name: &str, expected: &[String], actual: &[String]) -> bool {
    let differences: Vec<usize> = (0..expected.len().max(actual.len()))
        .filter(|&index| expected.get(index) != actual.get(index))
        .collect();
    let _ = writeln!(
        log,
        "  {name}: {} lines, {} different lines",
        expected.len(),
        differences.len()
    );
    for &index in differences.iter().take(SHOWN_DIFFERENCES) {
        let none = String::from("<none>");
        let _ = writeln!(
            log,
            "    line {}:\n      expected: {}\n      actual:   {}",
            index + 1,
            expected.get(index).unwrap_or(&none),
            actual.get(index).unwrap_or(&none)
        );
    }
    differences.is_empty()
}

/// Edit, build and check one Portable PDB. Write the results to `log`.
fn check(
    assembly: &Path,
    comparison: &Comparison,
    outputs: &[Output],
    out: &Path,
    log: &mut String,
) -> Result<bool, String> {
    let fixture = &comparison.fixture;
    let (original, theirs) = source(assembly, comparison)?;
    let pdb = Pdb::parse(&original).map_err(pdb_rs)?;
    let identity = IdentityPolicy::Explicit(pdb.identity());
    let _ = writeln!(log, "{fixture}:");

    let unchanged = pdb
        .edit(IdentityPolicy::ContentDerived)
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .map_err(pdb_rs)?;
    let mut passed = unchanged == original;
    let _ = writeln!(
        log,
        "  edit without a change has the original bytes: {}",
        yes(passed)
    );

    let built = copy(&pdb, identity)
        .and_then(|editor| editor.plan(EditOptions::default()))
        .and_then(|plan| plan.to_vec())
        .map_err(pdb_rs)?;
    let _ = writeln!(
        log,
        "  builder with new heaps: {} bytes (original {} bytes)",
        built.len(),
        original.len()
    );
    let built_dump = write_and_dump(assembly, out, &format!("{fixture}.built"), &built)?;
    let original_lines = expect::decoded(&theirs);
    passed &= same(
        log,
        "decoded dump of the built PDB",
        &original_lines,
        &expect::decoded(&built_dump),
    );

    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    apply(&mut editor, true).map_err(pdb_rs)?;
    let edited = editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .map_err(pdb_rs)?;
    let mut again = pdb.edit(IdentityPolicy::ContentDerived);
    apply(&mut again, true).map_err(pdb_rs)?;
    let repeated = again
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .map_err(pdb_rs)?;
    let _ = writeln!(
        log,
        "  the same edit again gives the same bytes: {}",
        yes(repeated == edited)
    );
    passed &= repeated == edited;
    let edited_dump = write_and_dump(assembly, out, &format!("{fixture}.edited"), &edited)?;
    let expected = expect::edited(&original_lines, &edit_description())?;
    let actual = expect::decoded(&edited_dump);
    passed &= same(
        log,
        "decoded dump of the edited PDB (without custom debug information)",
        &expect::without_custom(&expected),
        &expect::without_custom(&actual),
    );
    passed &= same(
        log,
        "custom debug information of the edited PDB (sorted, without row IDs)",
        &expect::custom_blocks(&expected),
        &expect::custom_blocks(&actual),
    );

    if let Input::Standalone(path) = &comparison.input {
        let mut lines = pdb.edit(identity);
        apply(&mut lines, false).map_err(pdb_rs)?;
        let moved = lines
            .plan(EditOptions::default())
            .and_then(|plan| plan.to_vec())
            .map_err(pdb_rs)?;
        let documents = lines
            .documents_mut()
            .map_err(pdb_rs)?
            .iter()
            .filter_map(|(_, document)| String::from_utf8(document.name.to_vec()).ok())
            .collect();
        let pdbs = stack::Pdbs {
            original: &original,
            built: &built,
            moved: &moved,
            documents,
        };
        passed &= stack::check(fixture, outputs, path, out, &pdbs, log)?;
    }
    Ok(passed)
}

/// The values of the edit for the expected dump.
fn edit_description() -> expect::Edit {
    expect::Edit {
        rename_suffix: String::from_utf8_lossy(RENAME_SUFFIX).into_owned(),
        line_shift: LINE_SHIFT,
        constant: format!(
            "name=\"{}\" i4={ADDED_VALUE}",
            String::from_utf8_lossy(ADDED_CONSTANT)
        ),
        custom_parent: 0x0600_0001,
        custom: vec![
            format!(
                "cdi parent=0x06000001 kind={} codec=unknown",
                guid_text(&ADDED_KIND)
            ),
            format!(
                "cdi-raw size={:#x} bytes={}",
                ADDED_INFORMATION.len(),
                hex(ADDED_INFORMATION)
            ),
        ],
    }
}

/// Return a GUID as lowercase text.
fn guid_text(kind: &CustomDebugKind) -> String {
    let bytes = kind.as_bytes();
    let field = |range: std::ops::Range<usize>| {
        let mut part: Vec<u8> = bytes[range].to_vec();
        part.reverse();
        hex(&part)
    };
    format!(
        "{}-{}-{}-{}-{}",
        field(0..4),
        field(4..6),
        field(6..8),
        hex(&bytes[8..10]),
        hex(&bytes[10..16])
    )
}

/// Copy all records of `pdb` into a new editor, which writes new heaps.
fn copy<S: ReadAt>(pdb: &Pdb<S>, identity: IdentityPolicy) -> pdb_rs::Result<PdbEditor<'static>> {
    let mut editor = PdbEditor::new(
        ExternalRowCounts::from_pdb_stream(pdb.pdb_stream()),
        identity,
    );
    editor.set_entry_point(pdb.pdb_stream().entry_point());
    let mut source = pdb.edit(identity);
    *editor.methods_mut()? = source.methods_mut()?.clone();
    let documents = editor.documents_mut()?;
    for document in pdb.documents() {
        documents.push(document?.1)?;
    }
    // A source handle index is the source row - 1, so the parents of the custom debug
    // information stay valid. Keep the list forms of empty tables of the source producer.
    let source_scopes = source.local_scopes_mut()?;
    let (variable_list, constant_list) = (
        source_scopes.variable_list_form(),
        source_scopes.constant_list_form(),
    );
    let scopes = editor.local_scopes_mut()?;
    scopes.set_variable_list_form(variable_list);
    scopes.set_constant_list_form(constant_list);
    for scope in pdb.local_scopes() {
        scopes.push(scope?.1)?;
    }
    let imports = editor.import_scopes_mut()?;
    for scope in pdb.import_scopes() {
        imports.push(scope?.1)?;
    }
    let methods = editor.state_machine_methods_mut()?;
    for method in pdb.state_machine_methods() {
        methods.push(method?.1);
    }
    let custom = editor.custom_debug_information_mut()?;
    for information in source.custom_debug_information_mut()?.iter() {
        custom.push(information.1.clone())?;
    }
    Ok(editor)
}

/// Apply the edit. With `all` `false`, only move the sequence points.
fn apply<S: ReadAt>(editor: &mut PdbEditor<'_, S>, all: bool) -> pdb_rs::Result<()> {
    for method in editor.methods_mut()?.values_mut() {
        let Some(points) = method.decode()? else {
            continue;
        };
        let signature = points.local_signature();
        let form = points.local_signature_form();
        let mut moved = points.collect::<pdb_rs::Result<Vec<_>>>()?;
        for point in &mut moved {
            if let Some(span) = &mut point.span {
                span.start_line += LINE_SHIFT;
                span.end_line += LINE_SHIFT;
            }
        }
        *method = SequencePoints::encode_parts(signature, form, &moved)?;
    }
    if !all {
        return Ok(());
    }
    if let Some(document) = editor.documents_mut()?.get_mut(DocumentId(1)) {
        let mut name = document.name.to_vec();
        name.extend_from_slice(RENAME_SUFFIX);
        document.name = name.into();
    }
    if let Some((_, scope)) = editor.local_scopes_mut()?.iter_mut().last() {
        let mut signature = Vec::new();
        LocalConstantSignature::encode_parts(
            &[],
            &ConstantValue::Primitive(PrimitiveConstant::I4(ADDED_VALUE)),
            &mut signature,
        )?;
        let mut constant = LocalConstant::default();
        constant.name = ADDED_CONSTANT.into();
        constant.signature = signature.into();
        scope.constants.push(constant);
    }
    let method = MetadataToken::new(TableId::METHOD_DEF, 1).unwrap_or_default();
    let mut information =
        CustomDebugInformation::new(CustomDebugParent::Metadata(method), ADDED_KIND);
    information.value = ADDED_INFORMATION.into();
    editor.custom_debug_information_mut()?.push(information)?;
    Ok(())
}
