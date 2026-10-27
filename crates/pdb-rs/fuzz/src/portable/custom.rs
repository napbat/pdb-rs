//! Custom debug information values of Portable PDBs.
//!
//! The round trip decodes a value, encodes it, decodes the output and encodes it again.
//! The two outputs must be equal, and the second decode must give the same kind.

use pdb_rs::io::ReadAt;
use pdb_rs::portable::Pdb;
use pdb_rs::portable::custom::{CustomDebugKind, CustomDebugValue};

use crate::{encoded, ok};

/// The most items that one walk visits, to bound the time of one run.
const MAX_ITEMS: usize = 1 << 14;

/// The kinds that the first input byte selects. The last kind has no decoder.
const KINDS: [CustomDebugKind; 15] = [
    CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION,
    CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES,
    CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
    CustomDebugKind::TUPLE_ELEMENT_NAMES,
    CustomDebugKind::DEFAULT_NAMESPACE,
    CustomDebugKind::ENC_LOCAL_SLOT_MAP,
    CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP,
    CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP,
    CustomDebugKind::SOURCE_LINK,
    CustomDebugKind::EMBEDDED_SOURCE,
    CustomDebugKind::COMPILATION_METADATA_REFERENCES,
    CustomDebugKind::COMPILATION_OPTIONS,
    CustomDebugKind::TYPE_DEFINITION_DOCUMENTS,
    CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION,
    CustomDebugKind([0x5a; 16]),
];

/// Round-trip one custom debug information value, or a native one.
///
/// A first byte below 0x80 selects a portable kind. A first byte of 0x80 or more decodes
/// a native symbol attribute value.
pub fn value(data: &[u8]) {
    let Some((&selector, blob)) = data.split_first() else {
        return;
    };
    if selector >= 0x80 {
        crate::native::symbols::attribute_value(data);
        return;
    }
    round_trip(KINDS[usize::from(selector) % KINDS.len()], blob);
}

/// Decode `blob` as a value of `kind`, and check that encode and decode agree.
fn round_trip(kind: CustomDebugKind, blob: &[u8]) {
    let Some(value) = ok(CustomDebugValue::parse(kind, blob)) else {
        return;
    };
    assert_eq!(value.kind(), kind);
    visit(&value);
    let Some(encoded) = ok(encoded(|out| value.encode(out))) else {
        return;
    };
    let again = CustomDebugValue::parse(kind, &encoded).expect("an encoded value decodes");
    assert_eq!(again.kind(), kind);
    let mut twice = Vec::new();
    again.encode(&mut twice).expect("a decoded value encodes");
    assert_eq!(twice, encoded, "the encoding of {kind} is not stable");
}

/// Visit the entries of a decoded value.
fn visit(value: &CustomDebugValue<'_>) {
    match value {
        CustomDebugValue::AsyncMethodSteppingInformation(info) => {
            let _ = info.catch_handler_offset();
            assert_eq!(info.iter().count(), info.len());
        }
        CustomDebugValue::StateMachineHoistedLocalScopes(scopes) => {
            assert_eq!(scopes.iter().count(), scopes.len());
        }
        CustomDebugValue::DynamicLocalVariables(flags) => {
            assert_eq!(
                flags.iter().take(MAX_ITEMS).count(),
                flags.len().min(MAX_ITEMS)
            );
        }
        CustomDebugValue::TupleElementNames(names) => {
            assert_eq!(names.iter().count(), names.len());
        }
        CustomDebugValue::EncLocalSlotMap(map) => {
            let _ = map.syntax_offset_baseline();
            assert_eq!(map.iter().count(), map.len());
        }
        CustomDebugValue::EncLambdaAndClosureMap(map) => {
            let _ = (map.method_ordinal(), map.syntax_offset_baseline());
            let _ = (map.closures().count(), map.lambdas().count());
        }
        CustomDebugValue::EncStateMachineStateMap(map) => {
            let _ = (map.syntax_offset_baseline(), map.trailing());
            assert_eq!(map.iter().count(), map.len());
        }
        CustomDebugValue::CompilationMetadataReferences(references) => {
            assert_eq!(references.iter().count(), references.len());
        }
        CustomDebugValue::CompilationOptions(options) => {
            assert_eq!(options.iter().count(), options.len());
            let _ = options.get(b"language");
        }
        CustomDebugValue::TypeDefinitionDocuments(documents) => {
            assert_eq!(documents.iter().count(), documents.len());
        }
        CustomDebugValue::EmbeddedSource(source) => {
            let _ = (source.format(), source.decoded_size());
        }
        _ => {}
    }
}

/// Round-trip the custom debug information of each row of a Portable PDB.
pub(crate) fn walk_rows<S: ReadAt>(pdb: &Pdb<S>) {
    for row in pdb.custom_debug_informations().take(MAX_ITEMS) {
        let Some((id, row)) = ok(row) else {
            break;
        };
        let _ = (id, row.parent);
        round_trip(row.kind, &row.value);
        if let Some(value) = ok(row.decode()) {
            visit(&value);
        }
    }
}
