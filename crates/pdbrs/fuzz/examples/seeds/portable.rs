//! Split a Portable PDB or an `MPDB` payload into the inputs of the portable targets.

use pdbrs::portable::Pdb;
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::embedded::{EMBEDDED_PDB_SIGNATURE, decode_embedded, encode_embedded};

use crate::{Corpus, Outcome};

/// The custom debug information kinds in the selector order of the `custom_debug_info` target.
const KINDS: [CustomDebugKind; 14] = [
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
];

/// The largest decoded `MPDB` payload that the generator reads.
const MAX_PDB: usize = 64 << 20;

/// Add the seeds of the Portable PDB or `MPDB` payload in `bytes`.
pub fn seeds(bytes: &[u8], corpus: &mut Corpus) -> Outcome {
    let decoded;
    let bytes = if bytes.starts_with(&EMBEDDED_PDB_SIGNATURE) {
        corpus.add("mpdb", bytes)?;
        corpus.add("portable_pdb", bytes)?;
        decoded = decode_embedded(bytes, MAX_PDB)?;
        &decoded[..]
    } else {
        bytes
    };
    let pdb = Pdb::parse(bytes)?;
    for target in ["portable_pdb", "portable_metadata", "portable_rebuild"] {
        corpus.add(target, bytes)?;
    }
    corpus.add("mpdb", &encode_embedded(bytes)?)?;
    for document in pdb.documents() {
        let document = document?;
        corpus.add_parts("portable_blobs", &[0], &[], &pdb.blob(document.row().name)?)?;
        corpus.add_parts("portable_blobs", &[5], &[], document.name())?;
    }
    for method in pdb.method_debug_informations() {
        let method = method?;
        let mut prefix = vec![1];
        prefix.extend_from_slice(&method.document().get().to_le_bytes());
        corpus.add_parts(
            "portable_blobs",
            &prefix,
            &[],
            method.sequence_points_blob(),
        )?;
    }
    for constant in pdb.local_constants() {
        corpus.add_parts("portable_blobs", &[2], &[], constant?.signature_blob())?;
    }
    for scope in pdb.import_scopes() {
        corpus.add_parts("portable_blobs", &[3], &[], scope?.imports_blob())?;
    }
    for row in pdb.custom_debug_informations() {
        let row = row?;
        let selector = KINDS
            .iter()
            .position(|kind| *kind == row.kind())
            .unwrap_or(KINDS.len());
        corpus.add_parts(
            "custom_debug_info",
            &[u8::try_from(selector)?],
            &[],
            row.value(),
        )?;
        if row.kind() == CustomDebugKind::EMBEDDED_SOURCE {
            corpus.add("embedded_source", row.value())?;
        }
    }
    Ok(())
}
