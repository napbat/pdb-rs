//! Native opening validates the PDB layer, not only the container signature.

use pdbrs::ErrorKind;
use pdbrs::native::Pdb;

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn container(named_target: Option<u32>, age: u32) -> Vec<u8> {
    let mut info = vec![0; 28];
    put(&mut info, 0, 20_000_404);
    put(&mut info, 8, age);
    let name: &[u8] = if named_target.is_some() {
        b"/test\0"
    } else {
        b""
    };
    info.extend_from_slice(&u32::try_from(name.len()).unwrap().to_le_bytes());
    info.extend_from_slice(name);
    // The hash table, then the `niMac` word.
    let values = if let Some(target) = named_target {
        vec![1, 2, 1, 1, 0, 0, target, 0]
    } else {
        vec![0, 1, 0, 0, 0]
    };
    for value in values {
        info.extend_from_slice(&u32::to_le_bytes(value));
    }

    let mut bytes = vec![0; 6 * 512];
    bytes[..32].copy_from_slice(b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0");
    for (offset, value) in [(32, 512), (36, 1), (40, 6), (44, 16), (52, 3)] {
        put(&mut bytes, offset, value);
    }
    put(&mut bytes, 3 * 512, 4);
    for (index, value) in [2, u32::MAX, u32::try_from(info.len()).unwrap(), 5]
        .into_iter()
        .enumerate()
    {
        put(&mut bytes, 4 * 512 + index * 4, value);
    }
    bytes[5 * 512..5 * 512 + info.len()].copy_from_slice(&info);
    bytes
}

#[test]
fn opens_info_without_requiring_optional_type_or_debug_streams() {
    let bytes = container(None, 11);
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.info().identity().age(), 11);
    assert!(pdb.types().unwrap().is_none());
    assert!(pdb.ids().unwrap().is_none());
    assert!(pdb.debug_info().unwrap().is_none());
}

#[test]
fn rejects_named_reference_outside_directory_or_to_nil_stream() {
    let error = Pdb::parse(&container(Some(9), 1)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidStreamIndex);
    let error = Pdb::parse(&container(Some(0), 1)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn applies_typed_stream_limit_before_loading_info() {
    let bytes = container(None, 1);
    let options = pdbrs::native::ReadOptions {
        max_stream_bytes: 4,
        ..pdbrs::native::ReadOptions::default()
    };
    let error = Pdb::with_options(bytes.as_slice(), options).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

fn with_debug_info(info_age: u32, dbi_age: u32, debug_target: u16) -> Vec<u8> {
    let mut bytes = container(None, info_age);
    bytes.resize(7 * 512, 0);
    put(&mut bytes, 40, 7);
    put(&mut bytes, 44, 28);
    for (index, value) in [4, u32::MAX, 52, u32::MAX, 66, 5, 6]
        .into_iter()
        .enumerate()
    {
        put(&mut bytes, 4 * 512 + index * 4, value);
    }
    let dbi = &mut bytes[6 * 512..];
    put(dbi, 0, u32::MAX);
    put(dbi, 4, 19_990_903);
    put(dbi, 8, dbi_age);
    for offset in [12, 16, 20] {
        dbi[offset..offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    }
    put(dbi, 48, 2);
    dbi[64..66].copy_from_slice(&debug_target.to_le_bytes());
    bytes
}

#[test]
fn rejects_inconsistent_info_and_dbi_ages() {
    let bytes = with_debug_info(1, 2, u16::MAX);
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(
        pdb.debug_info().unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn checks_optional_debug_references_against_the_directory() {
    for (target, expected) in [
        (8, ErrorKind::InvalidStreamIndex),
        (0, ErrorKind::InvalidReference),
    ] {
        let bytes = with_debug_info(1, 1, target);
        let pdb = Pdb::parse(&bytes).unwrap();
        assert_eq!(pdb.debug_info().unwrap_err().kind(), expected);
    }
}
