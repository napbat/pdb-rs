//! Matching rules for Portable PDB debug references.

use pdbrs::portable::Identity;

#[test]
fn portable_matching_uses_stamp_and_requires_age_one() {
    let guid = [0x29; 16];
    let stamp = 0x1234_5678;
    let identity = Identity::new(guid, stamp);

    assert!(identity.matches_codeview(&guid, stamp, 1));
    assert!(!identity.matches_codeview(&guid, 0, 1));
    assert!(!identity.matches_codeview(&guid, stamp, 2));
    let mut other_guid = guid;
    other_guid[15] ^= 1;
    assert!(!identity.matches_codeview(&other_guid, stamp, 1));
}
