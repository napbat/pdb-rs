//! Portable PDB round trips: exact copy, rebuild from parts and edit.
//!
//! A round trip checks self-consistency only: the written PDB must read back with the same
//! rows and heaps. It does not check that other readers accept the output.

use pdbrs::io::copy_exact;
use pdbrs::portable::embedded::decode_embedded;
use pdbrs::portable::write::PdbParts;
use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb};

use crate::ok;
use crate::portable::{options, walk};

/// Rebuild a Portable PDB and check that the output reads back with the same parts.
///
/// # Panics
///
/// Panics if a copy differs from the input, or if a written PDB does not read back with the
/// same parts.
pub fn rebuild(data: &[u8]) {
    let Some(pdb) = ok(Pdb::with_options(data, options())) else {
        return;
    };
    let mut copy = Vec::new();
    let length = copy_exact(pdb.source(), &mut copy).expect("an exact copy writes");
    assert_eq!(copy, data);
    assert_eq!(u64::try_from(data.len()).ok(), Some(length));

    let Some(parts) = ok(pdb.to_parts()) else {
        return;
    };
    let identity = pdb.identity();
    let Some(plan) = ok(parts.clone().plan(IdentityPolicy::Explicit(identity))) else {
        return;
    };
    assert_eq!(plan.identity(), identity);
    let bytes = plan.to_vec().expect("a planned PDB writes");
    assert_eq!(u64::try_from(bytes.len()).ok(), Some(plan.file_size()));
    let written = Pdb::with_options(bytes.as_slice(), options()).expect("a rebuilt PDB reads");
    assert_eq!(written.identity(), identity);
    let again = written.to_parts().expect("a rebuilt PDB gives parts");
    same_parts(&parts, &again);
    walk(&written);
    let embedded = plan.to_embedded().expect("a planned PDB embeds");
    let decoded = decode_embedded(&embedded, bytes.len()).expect("an embedded PDB decodes");
    assert_eq!(decoded, bytes);

    if let Some(derived) = ok(parts.plan(IdentityPolicy::ContentDerived)) {
        let bytes = derived.to_vec().expect("a planned PDB writes");
        let written = Pdb::parse(&bytes).expect("a rebuilt PDB reads");
        assert_eq!(written.identity(), derived.identity());
    }
    edit(&pdb);
}

/// Check that two sets of parts have the same rows and heap contents.
fn same_parts(left: &PdbParts, right: &PdbParts) {
    assert_eq!(left.version, right.version);
    assert_eq!(left.entry_point, right.entry_point);
    assert_eq!(left.tables, right.tables);
    assert_eq!(left.strings.bytes(), right.strings.bytes());
    assert_eq!(left.user_strings.bytes(), right.user_strings.bytes());
    assert_eq!(left.blobs.bytes(), right.blobs.bytes());
    assert_eq!(left.guids.bytes(), right.guids.bytes());
}

/// Make an editor from a Portable PDB, write it and read the output.
fn edit(pdb: &Pdb<&[u8]>) {
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let Some(documents) = ok(editor.documents_mut().map(|documents| documents.len())) else {
        return;
    };
    let Some(plan) = ok(editor.plan(EditOptions::default())) else {
        return;
    };
    let _ = plan.report().streams().len();
    let bytes = plan.to_vec().expect("a planned PDB writes");
    let written = Pdb::with_options(bytes.as_slice(), options()).expect("an edited PDB reads");
    assert_eq!(
        written
            .tables()
            .row_count(pdbrs::portable::metadata::TableId::DOCUMENT),
        u32::try_from(documents).unwrap_or(u32::MAX),
    );
    walk(&written);
    let mut again = written.edit(IdentityPolicy::ContentDerived);
    if ok(again.decode_all()).is_some()
        && let Some(plan) = ok(again.plan(EditOptions::default()))
    {
        let _ = plan.report().streams().len();
    }
}
