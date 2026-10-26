//! The `verify-edit` command.
//!
//! The command reads each Portable PDB of the fixture manifest (a standalone file or the
//! decoded `MPDB` payload of an image) with `Pdb::edit` and checks the pdbrs builder and editor
//! against the verifier (`System.Reflection.Metadata`):
//!
//! 1. A plan of the editor without a change has the original bytes.
//! 2. `PdbBuilder::plan` of the same records writes new heaps. The decoded sections of its
//!    verifier dump are the same as those of the original.
//! 3. An edit renames document 1, moves each visible sequence point 1000 lines down, adds the
//!    local constant `pdbrsAdded` (`i4=42`) to the last local scope and adds a custom debug
//!    information value of an unknown kind to `MethodDef` 1. The decoded sections of its
//!    verifier dump are the expected change of the original sections. The custom debug
//!    information lines are compared without their row IDs.
//! 4. For a standalone fixture, the program prints the `stack` mode trace with the original,
//!    the built (step 2) and a line-moved PDB. The PDBs use the original PDB ID. The built PDB
//!    gives the same trace, and the line-moved PDB gives each line number plus 1000.
//!
//! The command writes the PDBs, the dumps and the traces to `target/verify-edit/`.

mod expect;
mod stack;

use std::fs;
use std::path::Path;
use std::process::Command;

use pdbrs::portable::build::{CustomDebugInfo, CustomDebugParent, LocalConstantInfo};
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::debug::{ConstantValue, LocalConstantSignature, PrimitiveConstant};
use pdbrs::portable::embedded::decode_embedded;
use pdbrs::portable::metadata::{DocumentId, MetadataToken, TableId};
use pdbrs::portable::{IdentityPolicy, Pdb, PdbBuilder, PdbEditor};

use super::rebuild::{hex, yes};
use super::{
    Comparison, EMBEDDED_LIMIT, Input, Output, SHOWN_DIFFERENCES, comparisons, outputs, read,
};
use crate::tools::{self, verifier};

/// The number of lines that the edit adds to each visible sequence point.
const LINE_SHIFT: u32 = 1000;
/// The suffix that the edit adds to the name of document 1.
const RENAME_SUFFIX: &[u8] = b".edited";
/// The name of the added local constant.
const ADDED_CONSTANT: &[u8] = b"pdbrsAdded";
/// The value of the added local constant.
const ADDED_VALUE: i32 = 42;
/// The kind of the added custom debug information: `70646272-7365-4564-6974-2d636865636b`.
const ADDED_KIND: CustomDebugKind =
    CustomDebugKind::from_fields(0x7064_6272, 0x7365, 0x4564, *b"it-check");
/// The value of the added custom debug information.
const ADDED_INFORMATION: &[u8] = b"pdbrs-edit";

/// Edit and build each Portable PDB fixture and compare the results with the verifier.
///
/// Return `false` if a check fails.
pub fn verify_edit(workspace: &Path, fixtures: &Path) -> Result<bool, String> {
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
    let mut passed = true;
    for comparison in &comparisons {
        match check(&assembly, comparison, &outputs, &out) {
            Ok(true) => {}
            Ok(false) => passed = false,
            Err(error) => {
                println!("{}: error: {error}", comparison.fixture);
                passed = false;
            }
        }
    }
    println!(
        "wrote the PDBs, the dumps and the traces to {}",
        out.display()
    );
    Ok(passed)
}

/// Format a pdbrs error.
fn pdbrs(error: impl std::fmt::Display) -> String {
    format!("pdbrs: {error}")
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

/// Print the differences of two line lists and return `true` if they are equal.
fn same(name: &str, expected: &[String], actual: &[String]) -> bool {
    let differences: Vec<usize> = (0..expected.len().max(actual.len()))
        .filter(|&index| expected.get(index) != actual.get(index))
        .collect();
    println!(
        "  {name}: {} lines, {} different lines",
        expected.len(),
        differences.len()
    );
    for &index in differences.iter().take(SHOWN_DIFFERENCES) {
        let none = String::from("<none>");
        println!(
            "    line {}:\n      expected: {}\n      actual:   {}",
            index + 1,
            expected.get(index).unwrap_or(&none),
            actual.get(index).unwrap_or(&none)
        );
    }
    differences.is_empty()
}

/// Edit, build and check one Portable PDB.
fn check(
    assembly: &Path,
    comparison: &Comparison,
    outputs: &[Output],
    out: &Path,
) -> Result<bool, String> {
    let fixture = &comparison.fixture;
    let (original, mut command) = match &comparison.input {
        Input::Standalone(path) => (read(path)?, verifier::command(assembly, path)),
        Input::Embedded { payload, image } => (
            decode_embedded(&read(payload)?, EMBEDDED_LIMIT).map_err(pdbrs)?,
            verifier::embedded_command(assembly, image),
        ),
    };
    let theirs = dump(&mut command)?;
    let pdb = Pdb::parse(&original).map_err(pdbrs)?;
    let identity = IdentityPolicy::Explicit(pdb.identity());
    println!("{fixture}:");

    let editor = pdb.edit().map_err(pdbrs)?;
    let unchanged = editor
        .plan(IdentityPolicy::ContentDerived)
        .and_then(|plan| plan.to_vec())
        .map_err(pdbrs)?;
    let mut passed = unchanged == original;
    println!(
        "  edit without a change has the original bytes: {}",
        yes(passed)
    );

    let builder: &PdbBuilder = &editor;
    let built = builder
        .plan(identity)
        .and_then(|plan| plan.to_vec())
        .map_err(pdbrs)?;
    println!(
        "  builder with new heaps: {} bytes (original {} bytes)",
        built.len(),
        original.len()
    );
    let built_dump = write_and_dump(assembly, out, &format!("{fixture}.built"), &built)?;
    let original_lines = expect::decoded(&theirs);
    passed &= same(
        "decoded dump of the built PDB",
        &original_lines,
        &expect::decoded(&built_dump),
    );

    let mut editor = pdb.edit().map_err(pdbrs)?;
    apply(&mut editor, true).map_err(pdbrs)?;
    let edited = editor
        .plan(IdentityPolicy::ContentDerived)
        .and_then(|plan| plan.to_vec())
        .map_err(pdbrs)?;
    let mut again = pdb.edit().map_err(pdbrs)?;
    apply(&mut again, true).map_err(pdbrs)?;
    let repeated = again
        .plan(IdentityPolicy::ContentDerived)
        .and_then(|plan| plan.to_vec())
        .map_err(pdbrs)?;
    println!(
        "  the same edit again gives the same bytes: {}",
        yes(repeated == edited)
    );
    passed &= repeated == edited;
    let edited_dump = write_and_dump(assembly, out, &format!("{fixture}.edited"), &edited)?;
    let expected = expect::edited(&original_lines, &edit_description())?;
    let actual = expect::decoded(&edited_dump);
    passed &= same(
        "decoded dump of the edited PDB (without custom debug information)",
        &expect::without_custom(&expected),
        &expect::without_custom(&actual),
    );
    passed &= same(
        "custom debug information of the edited PDB (sorted, without row IDs)",
        &expect::custom_blocks(&expected),
        &expect::custom_blocks(&actual),
    );

    if let Input::Standalone(path) = &comparison.input {
        let mut lines = pdb.edit().map_err(pdbrs)?;
        apply(&mut lines, false).map_err(pdbrs)?;
        let moved = lines
            .plan(identity)
            .and_then(|plan| plan.to_vec())
            .map_err(pdbrs)?;
        let documents = (1..=lines.document_count())
            .filter_map(|row| {
                let document = lines.document(DocumentId(u32::try_from(row).ok()?))?;
                String::from_utf8(document.name.clone()).ok()
            })
            .collect();
        let pdbs = stack::Pdbs {
            original: &original,
            built: &built,
            moved: &moved,
            documents,
        };
        passed &= stack::check(fixture, outputs, path, out, &pdbs)?;
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

/// Apply the edit. With `all` `false`, only move the sequence points.
fn apply(editor: &mut PdbEditor, all: bool) -> pdbrs::Result<()> {
    for (_, method) in editor.methods_mut() {
        for point in &mut method.sequence_points {
            if let Some(span) = &mut point.span {
                span.start_line += LINE_SHIFT;
                span.end_line += LINE_SHIFT;
            }
        }
    }
    if !all {
        return Ok(());
    }
    if let Some(document) = editor.document_mut(DocumentId(1)) {
        document.name.extend_from_slice(RENAME_SUFFIX);
    }
    let last = editor
        .local_scopes()
        .max_by_key(|(_, scope)| {
            (
                scope.method,
                scope.start_offset,
                std::cmp::Reverse(scope.length),
            )
        })
        .map(|(handle, _)| handle);
    if let Some(scope) = last {
        let mut constant = LocalConstantInfo::default();
        constant.name = ADDED_CONSTANT.to_vec();
        LocalConstantSignature::encode_parts(
            &[],
            &ConstantValue::Primitive(PrimitiveConstant::I4(ADDED_VALUE)),
            &mut constant.signature,
        )?;
        editor.add_local_constant(scope, constant)?;
    }
    let method = MetadataToken::new(TableId::METHOD_DEF, 1).unwrap_or_default();
    let mut information = CustomDebugInfo::new(CustomDebugParent::Metadata(method), ADDED_KIND);
    information.value = ADDED_INFORMATION.to_vec();
    editor.add_custom_debug_information(information)?;
    Ok(())
}
