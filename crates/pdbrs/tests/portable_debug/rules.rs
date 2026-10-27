//! Validation of the ordering and reference rules, and the typed rows of the synthetic PDB.

use pdbrs::portable::Pdb;
use pdbrs::portable::debug::{
    ConstantValue, Import, Outcome, PrimitiveConstant, Rule, ValidateOptions, ValidationReport,
};
use pdbrs::portable::metadata::{
    IlOffset, ImportScopeId, LocalConstantId, LocalScopeId, LocalVariableId, MethodDefId,
    StateMachineMethodId, TableId,
};
use pdbrs::{Error, ErrorKind};

use crate::expect;
use crate::synthetic::{NAME_A, Synthetic};

/// Validate a synthetic PDB with the default options.
fn validate(synthetic: &Synthetic) -> pdbrs::Result<ValidationReport> {
    validate_with(synthetic, ValidateOptions::default())
}

fn validate_with(
    synthetic: &Synthetic,
    options: ValidateOptions,
) -> pdbrs::Result<ValidationReport> {
    let bytes = synthetic.bytes();
    Pdb::parse(&bytes).unwrap().validate_debug(options)
}

/// Validate a changed PDB and return the error.
fn violation(change: impl FnOnce(&mut Synthetic)) -> Error {
    let mut synthetic = Synthetic::valid();
    change(&mut synthetic);
    match validate(&synthetic) {
        Ok(report) => panic!("the validation passed: {report:?}"),
        Err(error) => error,
    }
}

/// Check the table and the row of an error location.
fn at(error: &Error, table: TableId, row: u32) {
    let location = error.portable_location().unwrap();
    assert_eq!(location.table(), Some(table), "{error}");
    assert_eq!(location.row(), Some(row), "{error}");
}

#[test]
fn the_valid_pdb_passes_every_rule() {
    let report = validate(&Synthetic::valid()).unwrap();
    let checked = |rule| match report.outcome(rule) {
        Some(Outcome::Checked { items }) => items,
        other => panic!("{rule:?}: {other:?}"),
    };
    assert_eq!(checked(Rule::DocumentNames), 2);
    assert_eq!(checked(Rule::DocumentHashes), 2);
    assert_eq!(checked(Rule::MethodDebugInformationCount), 4);
    assert_eq!(checked(Rule::SequencePoints), 5);
    assert_eq!(checked(Rule::LocalScopeOrder), 4);
    assert_eq!(checked(Rule::LocalScopeRanges), 4);
    assert_eq!(checked(Rule::VariableRuns), 3);
    assert_eq!(checked(Rule::LocalVariables), 3);
    assert_eq!(checked(Rule::ConstantRuns), 1);
    assert_eq!(checked(Rule::LocalConstants), 1);
    assert_eq!(checked(Rule::ImportScopeParents), 2);
    assert_eq!(checked(Rule::Imports), 2);
    assert_eq!(checked(Rule::StateMachineMethods), 1);
    assert_eq!(checked(Rule::CustomDebugInformationParents), 2);
    for rule in [Rule::InstructionBoundaries, Rule::MethodBodySizes] {
        assert!(matches!(
            report.outcome(rule),
            Some(Outcome::NotChecked { .. })
        ));
    }
    assert_eq!(report.rules().len(), 16);
}

#[test]
fn typed_rows_give_runs_names_and_values() {
    let bytes = Synthetic::valid().bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let scopes: Vec<_> = pdb.local_scopes().map(|scope| scope.unwrap().1).collect();
    let variable = |row| pdb.local_variable(LocalVariableId(row)).unwrap();
    let runs: Vec<_> = scopes
        .iter()
        .map(|scope| (scope.variables.clone(), scope.constants.len()))
        .collect();
    assert_eq!(
        runs,
        [
            (vec![variable(1)], 1),
            (vec![variable(2)], 0),
            (vec![], 0),
            (vec![variable(3)], 0)
        ]
    );
    assert_eq!(scopes[1].end_offset(), Some(IlOffset(0x10)));
    assert_eq!(scopes[3].method, MethodDefId(2));
    let variable = variable(2);
    assert_eq!((variable.name.as_bytes(), variable.index), (&b"b"[..], 1));
    assert!(variable.is_debugger_hidden());
    let constant = pdb.local_constant(LocalConstantId(1)).unwrap();
    assert_eq!(constant.name, b"c");
    assert_eq!(scopes[0].constants, core::slice::from_ref(&constant));
    assert_eq!(
        constant.decode().unwrap().value(),
        &ConstantValue::Primitive(PrimitiveConstant::I4(5))
    );
    let child = pdb.import_scope(ImportScopeId(2)).unwrap();
    assert_eq!(child.parent, ImportScopeId(1));
    let [Import::AliasType { alias, .. }] = &child.imports[..] else {
        panic!("unexpected imports {:?}", child.imports);
    };
    assert_eq!(alias, b"Alias");
    let machine = pdb.state_machine_method(StateMachineMethodId(1)).unwrap();
    assert_eq!(
        (machine.move_next_method, machine.kickoff_method),
        (MethodDefId(2), MethodDefId(1))
    );
    assert_eq!(pdb.state_machine_methods().len(), 1);
    assert_eq!(pdb.import_scopes().len(), 2);
    assert_eq!(pdb.local_constants().len(), 1);
    assert_eq!(pdb.local_variables().len(), 3);
    let scope = pdb.local_scope(LocalScopeId(4)).unwrap();
    assert_eq!(scope.import_scope, ImportScopeId(2));
}

#[test]
fn cyclic_import_scopes_are_rejected() {
    let error = violation(|pdb| pdb.import_scopes[0][0] = 2);
    expect(&error, ErrorKind::InvalidFormat, "cycle");
    at(&error, TableId::IMPORT_SCOPE, 1);
    let error = violation(|pdb| pdb.import_scopes[1][0] = 2);
    expect(&error, ErrorKind::InvalidFormat, "cycle");
    at(&error, TableId::IMPORT_SCOPE, 2);
}

#[test]
fn invalid_scope_ranges_are_rejected() {
    // Overlap: [0x8, 0x18) crosses the end of [0x2, 0x10).
    let error = violation(|pdb| pdb.scopes[2][4..].copy_from_slice(&[0x08, 0x10]));
    expect(&error, ErrorKind::InvalidFormat, "without nesting");
    at(&error, TableId::LOCAL_SCOPE, 3);
    // Outside the method: [0x10, 0x30) ends after the first scope [0x0, 0x20).
    let error = violation(|pdb| pdb.scopes[2][5] = 0x20);
    expect(&error, ErrorKind::InvalidFormat, "outside the first scope");
    // The first scope of method 2 starts at 0x4.
    let error = violation(|pdb| pdb.scopes[3][4] = 0x04);
    expect(
        &error,
        ErrorKind::InvalidFormat,
        "does not start at IL offset 0",
    );
    at(&error, TableId::LOCAL_SCOPE, 4);
    // An empty scope, and a scope that ends after 0x7FFFFFFF.
    let error = violation(|pdb| pdb.scopes[2][5] = 0);
    expect(&error, ErrorKind::InvalidFormat, "empty");
    let error = violation(|pdb| pdb.scopes[3][5] = 0x8000_0000);
    expect(&error, ErrorKind::InvalidFormat, "0x7FFFFFFF");
    let error = violation(|pdb| pdb.scopes[3][0] = 0);
    expect(&error, ErrorKind::InvalidFormat, "method is nil");
}

#[test]
fn unsorted_tables_are_rejected() {
    // LocalScope: a later start before an earlier start.
    let error = violation(|pdb| {
        pdb.scopes[1][4..].copy_from_slice(&[0x10, 0x08]);
        pdb.scopes[2][4..].copy_from_slice(&[0x02, 0x0e]);
    });
    expect(&error, ErrorKind::InvalidFormat, "not sorted");
    at(&error, TableId::LOCAL_SCOPE, 3);
    // LocalScope: a shorter scope before a longer scope with the same start.
    let error = violation(|pdb| {
        pdb.scopes[0][5] = 0x10;
        pdb.scopes[1][4..].copy_from_slice(&[0x00, 0x20]);
    });
    expect(&error, ErrorKind::InvalidFormat, "not sorted");
    // LocalScope: a method before the previous method.
    let error = violation(|pdb| pdb.scopes[0][0] = 2);
    expect(&error, ErrorKind::InvalidFormat, "not sorted");
    // StateMachineMethod: MoveNext methods out of order, or a duplicate.
    let error = violation(|pdb| pdb.state_machines = vec![[3, 1], [2, 4]]);
    expect(&error, ErrorKind::InvalidFormat, "not sorted");
    at(&error, TableId::STATE_MACHINE_METHOD, 2);
    let error = violation(|pdb| pdb.state_machines = vec![[2, 1], [2, 4]]);
    expect(&error, ErrorKind::InvalidFormat, "duplicate");
    let error = violation(|pdb| pdb.state_machines = vec![[2, 1], [3, 1]]);
    expect(&error, ErrorKind::InvalidFormat, "same KickoffMethod");
    let error = violation(|pdb| pdb.state_machines = vec![[2, 0]]);
    expect(&error, ErrorKind::InvalidFormat, "nil");
    // CustomDebugInformation: parents out of order, and a nil parent.
    let error = violation(|pdb| pdb.custom.swap(0, 1));
    expect(&error, ErrorKind::InvalidFormat, "not sorted by Parent");
    at(&error, TableId::CUSTOM_DEBUG_INFORMATION, 2);
    let error = violation(|pdb| pdb.custom[0][0] = 0);
    expect(&error, ErrorKind::InvalidFormat, "parent is nil");
}

#[test]
fn invalid_row_references_are_rejected() {
    // Row columns: the row reader checks them.
    let error = violation(|pdb| pdb.methods[0][0] = 3);
    expect(&error, ErrorKind::InvalidReference, "after the end");
    let error = violation(|pdb| pdb.scopes[0][1] = 3);
    expect(&error, ErrorKind::InvalidReference, "after the end");
    // A document record to document 3.
    let error = violation(|pdb| {
        let blob = pdb.blob(&[
            0x00, 0x01, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x03, 0x04, 0x00, 0x02, 0x02, 0x00,
        ]);
        pdb.methods[1][1] = blob;
    });
    expect(
        &error,
        ErrorKind::InvalidReference,
        "document does not exist",
    );
    at(&error, TableId::METHOD_DEBUG_INFORMATION, 2);
    // A LocalSignature row 3; the #Pdb stream has 2 StandAloneSig rows.
    let error = violation(|pdb| {
        let blob = pdb.blob(&[0x03, 0x00, 0x00, 0x00]);
        pdb.methods[0][1] = blob;
    });
    expect(&error, ErrorKind::InvalidReference, "LocalSignature");
    // An enum type TypeRef 6; the #Pdb stream has 5 TypeRef rows.
    let error = violation(|pdb| {
        let blob = pdb.blob(&[0x08, 0x05, 0x00, 0x00, 0x00, 0x19]);
        pdb.constants[0][1] = blob;
    });
    expect(&error, ErrorKind::InvalidReference, "constant type");
    at(&error, TableId::LOCAL_CONSTANT, 1);
    // An import of AssemblyRef 3 and of TypeDef 4.
    let error = violation(|pdb| {
        let blob = pdb.blob(&[0x02, 0x03, 0x01]);
        pdb.import_scopes[0][1] = blob;
    });
    expect(&error, ErrorKind::InvalidReference, "AssemblyRef");
    let error = violation(|pdb| {
        let blob = pdb.blob(&[0x03, 0x10]);
        pdb.import_scopes[1][1] = blob;
    });
    expect(&error, ErrorKind::InvalidReference, "type row");
    at(&error, TableId::IMPORT_SCOPE, 2);
    // A namespace offset after the #Blob heap.
    let error = violation(|pdb| {
        let blob = pdb.blob(&[0x01, 0xc0, 0x00, 0x10, 0x00]);
        pdb.import_scopes[0][1] = blob;
    });
    expect(&error, ErrorKind::InvalidReference, "after the heap");
    at(&error, TableId::IMPORT_SCOPE, 1);
}

#[test]
fn variable_and_constant_runs_are_checked() {
    // VariableList 1, 3, 2, 3: the run of scope 2 ends before it starts.
    let error = violation(|pdb| {
        pdb.scopes[1][2] = 3;
        pdb.scopes[2][2] = 2;
    });
    expect(&error, ErrorKind::InvalidFormat, "starts after");
    // The first scope starts its run at row 2, so row 1 has no owner.
    let error = violation(|pdb| pdb.scopes[0][2] = 2);
    expect(&error, ErrorKind::InvalidFormat, "no owner");
    // A nil list while the table has rows.
    let error = violation(|pdb| pdb.scopes[0][3] = 0);
    expect(&error, ErrorKind::InvalidFormat, "nil");
    // Two variables of scope 1 with the same name, then with the same slot index.
    let error = violation(|pdb| {
        pdb.scopes[1][2] = 3;
        pdb.scopes[2][2] = 3;
        pdb.variables[1][2] = NAME_A;
    });
    expect(&error, ErrorKind::InvalidFormat, "same name");
    at(&error, TableId::LOCAL_VARIABLE, 2);
    let error = violation(|pdb| {
        pdb.scopes[1][2] = 3;
        pdb.scopes[2][2] = 3;
        pdb.variables[1][1] = 0;
    });
    expect(&error, ErrorKind::InvalidFormat, "same slot index");
    // Rows without scopes.
    let error = violation(|pdb| {
        pdb.scopes.clear();
        pdb.custom.clear();
    });
    expect(
        &error,
        ErrorKind::InvalidFormat,
        "LocalScope table is empty",
    );
}

#[test]
fn nil_lists_are_empty_runs_of_empty_tables() {
    let mut synthetic = Synthetic::valid();
    synthetic.constants.clear();
    for scope in &mut synthetic.scopes {
        scope[3] = 0;
    }
    let report = validate(&synthetic).unwrap();
    assert_eq!(
        report.outcome(Rule::ConstantRuns),
        Some(Outcome::Checked { items: 0 })
    );
}

#[test]
fn document_and_method_rules_are_checked() {
    let error = violation(|pdb| pdb.documents[1][0] = pdb.documents[0][0]);
    expect(&error, ErrorKind::InvalidFormat, "same name");
    at(&error, TableId::DOCUMENT, 2);
    let error = violation(|pdb| {
        let hash = pdb.blob(&[1; 20]);
        pdb.documents[0][2] = hash;
    });
    expect(&error, ErrorKind::InvalidFormat, "hash size");
    let error = violation(|pdb| pdb.methods[2][0] = 1);
    expect(
        &error,
        ErrorKind::InvalidFormat,
        "without sequence points has a document",
    );
    at(&error, TableId::METHOD_DEBUG_INFORMATION, 3);
}

#[test]
fn the_method_count_comes_from_the_options_or_the_pdb_stream() {
    let synthetic = Synthetic::valid();
    let options = ValidateOptions::default().with_method_def_count(5);
    let error = validate_with(&synthetic, options).unwrap_err();
    expect(&error, ErrorKind::InvalidFormat, "MethodDef row count");
    let options = ValidateOptions::default().with_method_def_count(4);
    assert!(validate_with(&synthetic, options).is_ok());
}
