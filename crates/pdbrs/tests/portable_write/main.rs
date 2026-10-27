//! Behavior tests for the Portable PDB writer and editor with synthetic data.
//!
//! The tests cover the heap builders, index widths that change during a build (heaps, local
//! tables, external tables and coded indexes), the plan checks, the PDB ID policies, the
//! checksum, determinism, the `MPDB` output, exact copies, the rebuild of an existing
//! Portable PDB, builds and edits with `PdbEditor`, copy-on-write edits, the plan report, the
//! editor with unknown custom debug information and the editing tasks of #41.

mod build;
#[path = "../portable_metadata/builder.rs"]
mod builder;
mod checks;
mod copy_on_write;
mod edit;
mod editor;
#[cfg(feature = "std")]
mod files;
mod heaps;
mod identity;
mod rebuild;
mod tasks;
mod widths;

use pdbrs::Error;
use pdbrs::PortableLocation;
use pdbrs::portable::metadata::{CustomDebugInformationRow, DocumentRow, MetadataToken, TableId};
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
    let mut row = DocumentRow::default();
    row.name = parts.blobs.add(name).unwrap();
    row
}

/// Make a custom debug information row of `parent` with a kind GUID and a value.
pub(crate) fn information(
    parts: &mut PdbParts,
    parent: (TableId, u32),
    value: &[u8],
) -> CustomDebugInformationRow {
    let mut row = CustomDebugInformationRow::default();
    row.parent = MetadataToken::new(parent.0, parent.1).unwrap();
    row.kind = parts.guids.add([7; 16]).unwrap();
    row.value = parts.blobs.add(value).unwrap();
    row
}
