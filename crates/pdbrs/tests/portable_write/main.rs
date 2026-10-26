//! Behavior tests for the Portable PDB writer, builder and editor with synthetic data.
//!
//! The tests cover the heap builders, index widths that change during a build (heaps, local
//! tables, external tables and coded indexes), the plan checks, the PDB ID policies, the
//! checksum, determinism, the `MPDB` output, exact copies, the rebuild of an existing
//! Portable PDB, the semantic builder and the editor with unknown custom debug information.

mod build;
#[path = "../portable_metadata/builder.rs"]
mod builder;
mod checks;
mod edit;
#[cfg(feature = "std")]
mod files;
mod heaps;
mod identity;
mod rebuild;
mod widths;

use pdbrs::Error;
use pdbrs::PortableLocation;
use pdbrs::portable::metadata::{
    BlobOffset, CustomDebugInformationRow, DocumentRow, GuidIndex, MetadataToken, TableId,
};
use pdbrs::portable::write::PdbParts;
use pdbrs::portable::{IdentityPolicy, Pdb, PdbWritePlan};

/// The ID of the explicit policy in the tests.
pub(crate) const ID: [u8; 20] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
];

/// Plan `parts` with the explicit test ID and return the plan. Panic on a planning error.
pub(crate) fn plan(parts: PdbParts) -> PdbWritePlan {
    parts
        .plan(IdentityPolicy::Explicit(
            pdbrs::portable::Identity::from_bytes(ID),
        ))
        .unwrap_or_else(|error| panic!("the plan failed: {error}"))
}

/// Plan `parts` and return the output bytes.
pub(crate) fn write(parts: PdbParts) -> Vec<u8> {
    plan(parts).to_vec().unwrap()
}

/// Plan `parts` and return the error. Panic if the plan succeeds.
pub(crate) fn plan_error(parts: PdbParts) -> Error {
    match parts.plan(IdentityPolicy::ContentDerived) {
        Ok(_) => panic!("the plan succeeded, but an error was expected"),
        Err(error) => error,
    }
}

/// Return the portable location of an error.
pub(crate) fn location(error: &Error) -> PortableLocation {
    *error
        .portable_location()
        .unwrap_or_else(|| panic!("the error has no portable location: {error}"))
}

/// Parse output bytes. Panic if the reader rejects them.
pub(crate) fn open(bytes: &[u8]) -> Pdb<&[u8]> {
    Pdb::parse(bytes).unwrap_or_else(|error| panic!("the reader rejects the output: {error}"))
}

/// Make parts with `methods` external `MethodDef` rows.
pub(crate) fn parts_with_methods(methods: u32) -> PdbParts {
    let mut parts = PdbParts::default();
    parts.external.set(TableId::METHOD_DEF, methods);
    parts
}

/// Make a document row with a name blob.
pub(crate) fn document(parts: &mut PdbParts, name: &[u8]) -> DocumentRow {
    DocumentRow {
        name: parts.blobs.add(name).unwrap(),
        hash_algorithm: GuidIndex::default(),
        hash: BlobOffset::default(),
        language: GuidIndex::default(),
    }
}

/// Make a custom debug information row of `parent` with a kind GUID and a value.
pub(crate) fn information(
    parts: &mut PdbParts,
    parent: (TableId, u32),
    value: &[u8],
) -> CustomDebugInformationRow {
    CustomDebugInformationRow {
        parent: MetadataToken::new(parent.0, parent.1).unwrap(),
        kind: parts.guids.add([7; 16]).unwrap(),
        value: parts.blobs.add(value).unwrap(),
    }
}
