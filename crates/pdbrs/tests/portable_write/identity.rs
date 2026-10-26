//! The PDB ID policies, the checksum, the `CodeView` fields, determinism and `MPDB` output.

use pdbrs::portable::Identity;
use pdbrs::portable::embedded::decode_embedded;
use pdbrs::portable::metadata::{StreamHeader, TableId};
use pdbrs::portable::write::{ChecksumAlgorithm, IdPolicy, PdbParts};

use crate::{ID, document, information, open, parts_with_methods, plan};

/// Make parts with a document and a custom debug information row.
fn sample(value: &[u8]) -> PdbParts {
    let mut parts = parts_with_methods(3);
    let row = document(&mut parts, b"/src");
    parts.tables.documents.push(row);
    let row = information(&mut parts, (TableId::METHOD_DEF, 2), value);
    parts.tables.custom_debug_information.push(row);
    parts
}

#[test]
fn explicit_id_is_written_and_returned() {
    let plan = plan(sample(b"x"));
    let bytes = plan.to_vec().unwrap();
    assert_eq!(plan.identity().to_bytes(), ID);
    assert_eq!(open(&bytes).pdb_stream().id(), &ID);
    let codeview = plan.codeview();
    assert_eq!(codeview.guid, *plan.identity().guid());
    assert_eq!(codeview.stamp, plan.identity().stamp());
    assert_eq!(codeview.age, 1);
    assert!(
        plan.identity()
            .matches_codeview(&codeview.guid, codeview.stamp, codeview.age)
    );
}

#[test]
fn content_derived_id_comes_from_the_sha256_checksum() {
    let plan = sample(b"x").plan(IdPolicy::ContentDerived).unwrap();
    let bytes = plan.to_vec().unwrap();
    let id = plan.identity().to_bytes();
    assert_eq!(open(&bytes).pdb_stream().id(), &id);
    // The PDB checksum and the content ID hash the same input: the output with a zero ID.
    let digest = plan.checksum(ChecksumAlgorithm::Sha256);
    let digest = digest.digest();
    let mut expected = [0; 20];
    expected.copy_from_slice(&digest[..20]);
    expected[7] = (expected[7] & 0x0F) | 0x40;
    expected[8] = (expected[8] & 0x3F) | 0x80;
    expected[19] |= 0x80;
    assert_eq!(id, expected);

    // Another ID policy changes only the ID bytes, not the checksum.
    let explicit = plan_with(sample(b"x"), Identity::from_bytes(ID));
    assert_eq!(
        explicit.checksum(ChecksumAlgorithm::Sha256).digest(),
        digest
    );
    let other = explicit.to_vec().unwrap();
    let differences: Vec<usize> = (0..bytes.len()).filter(|&i| bytes[i] != other[i]).collect();
    assert!(!differences.is_empty() && differences.len() <= 20);

    // A different value gives a different ID.
    let changed = sample(b"y").plan(IdPolicy::ContentDerived).unwrap();
    assert_ne!(changed.identity(), plan.identity());
}

/// Plan parts with an explicit ID.
fn plan_with(parts: PdbParts, identity: Identity) -> pdbrs::portable::write::WritePlan {
    parts.plan(IdPolicy::Explicit(identity)).unwrap()
}

#[test]
fn checksum_algorithms_have_their_names_and_sizes() {
    let plan = plan(sample(b"x"));
    for (algorithm, name, size) in [
        (ChecksumAlgorithm::Sha256, "SHA256", 32),
        (ChecksumAlgorithm::Sha384, "SHA384", 48),
        (ChecksumAlgorithm::Sha512, "SHA512", 64),
    ] {
        let checksum = plan.checksum(algorithm);
        assert_eq!(checksum.algorithm(), algorithm);
        assert_eq!(checksum.digest().len(), size);
        let data = checksum.entry_data();
        assert_eq!(&data[..name.len()], name.as_bytes());
        assert_eq!(data[name.len()], 0);
        assert_eq!(&data[name.len() + 1..], checksum.digest());
    }
}

#[test]
fn identical_parts_give_identical_bytes() {
    let first = sample(b"x").plan(IdPolicy::ContentDerived).unwrap();
    let second = sample(b"x").plan(IdPolicy::ContentDerived).unwrap();
    let bytes = first.to_vec().unwrap();
    assert_eq!(bytes, second.to_vec().unwrap());
    assert_eq!(first.identity(), second.identity());
    assert_eq!(u64::try_from(bytes.len()).unwrap(), first.file_size());
    let mut written = Vec::new();
    first.write_to(&mut written).unwrap();
    assert_eq!(written, bytes);
}

#[test]
fn embedded_output_decodes_to_the_pdb() {
    let plan = plan(sample(b"x"));
    let payload = plan.to_embedded().unwrap();
    assert_eq!(&payload[..4], b"MPDB");
    assert_eq!(
        decode_embedded(&payload, 1 << 20).unwrap(),
        plan.to_vec().unwrap()
    );
}

#[test]
fn output_has_the_system_reflection_metadata_layout() {
    let bytes = plan(sample(b"x")).to_vec().unwrap();
    let pdb = open(&bytes);
    let streams = pdb.metadata().streams();
    let names: Vec<&[u8]> = streams.iter().map(StreamHeader::name).collect();
    let expected: [&[u8]; 6] = [b"#Pdb", b"#~", b"#Strings", b"#US", b"#GUID", b"#Blob"];
    assert_eq!(names, expected);
    assert_eq!(pdb.metadata().version_string(), b"PDB v1.0");
    let tables = pdb.tables();
    let valid = (1 << 0x30) | (1 << 0x37);
    assert_eq!(tables.valid_mask(), valid);
    assert_eq!(tables.sorted_mask(), 1 << 0x37);
    assert_eq!(pdb.pdb_stream().referenced_tables(), 1 << 0x06);
    assert_eq!(pdb.pdb_stream().external_row_count(TableId::METHOD_DEF), 3);
}
