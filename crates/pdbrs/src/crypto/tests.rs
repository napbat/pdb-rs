//! NIST test vectors for the SHA-2 digests.
//!
//! The expected digests come from the NIST examples for FIPS 180-4
//! ("Cryptographic Standards and Guidelines: Examples with Intermediate
//! Values"). Python `hashlib` gives the same values.

use alloc::vec;

use super::{Sha256Hasher, Sha384Hasher, Sha512Hasher, sha256};
use crate::test_support::hex;

/// The 448-bit message of the NIST two-block SHA-256 example.
const TWO_BLOCK: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";

/// The 896-bit message of the NIST two-block SHA-384 and SHA-512 examples.
const LONG: &[u8] = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmn\
hijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";

/// The size of the one-million-'a' message.
const MILLION: usize = 1_000_000;

/// Return the SHA-384 digest of `message`, given to the hasher in pieces of 7 bytes. The
/// pieces cross the 128-byte block boundaries.
fn sha384(message: &[u8]) -> [u8; 48] {
    let mut hasher = Sha384Hasher::new();
    for piece in message.chunks(7) {
        hasher.update(piece);
    }
    hasher.finalize()
}

/// Return the SHA-512 digest of `message`, given to the hasher in pieces of 7 bytes.
fn sha512(message: &[u8]) -> [u8; 64] {
    let mut hasher = Sha512Hasher::new();
    for piece in message.chunks(7) {
        hasher.update(piece);
    }
    hasher.finalize()
}

#[test]
fn sha256_matches_nist_vectors() {
    let cases: [(&[u8], &str); 4] = [
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            TWO_BLOCK,
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            LONG,
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1",
        ),
    ];
    for (message, digest) in cases {
        assert_eq!(sha256(message).as_slice(), hex(digest));
    }
    assert_eq!(
        sha256(&vec![b'a'; MILLION]).as_slice(),
        hex("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
    );
}

#[test]
fn sha384_matches_nist_vectors() {
    let cases: [(&[u8], &str); 4] = [
        (
            b"abc",
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
             8086072ba1e7cc2358baeca134c825a7",
        ),
        (
            b"",
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da\
             274edebfe76f65fbd51ad2f14898b95b",
        ),
        (
            TWO_BLOCK,
            "3391fdddfc8dc7393707a65b1b4709397cf8b1d162af05abfe8f450de5f36bc6\
             b0455a8520bc4e6f5fe95b1fe3c8452b",
        ),
        (
            LONG,
            "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712\
             fcc7c71a557e2db966c3e9fa91746039",
        ),
    ];
    for (message, digest) in cases {
        assert_eq!(sha384(message).as_slice(), hex(digest));
    }
    assert_eq!(
        sha384(&vec![b'a'; MILLION]).as_slice(),
        hex(
            "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b\
             07b8b3dc38ecc4ebae97ddd87f3d8985"
        )
    );
}

#[test]
fn sha512_matches_nist_vectors() {
    let cases: [(&[u8], &str); 4] = [
        (
            b"abc",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            b"",
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
             47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
        ),
        (
            TWO_BLOCK,
            "204a8fc6dda82f0a0ced7beb8e08a41657c16ef468b228a8279be331a703c335\
             96fd15c13b1b07f9aa1d3bea57789ca031ad85c7a71dd70354ec631238ca3445",
        ),
        (
            LONG,
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018\
             501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
        ),
    ];
    for (message, digest) in cases {
        assert_eq!(sha512(message).as_slice(), hex(digest));
    }
    assert_eq!(
        sha512(&vec![b'a'; MILLION]).as_slice(),
        hex(
            "e718483d0ce769644e2e42c7bc15b4638e1f98b13b2044285632a803afa973eb\
             de0ff244877ea60a4cb0432ce577c31beb009c5c2c49aa2e4eadb217ad8cc09b"
        )
    );
}

#[test]
fn sha256_hasher_matches_one_shot_digest() {
    // Pieces of different sizes cross the 64-byte block boundaries.
    let mut hasher = Sha256Hasher::new();
    for piece in LONG.chunks(7) {
        hasher.update(piece);
    }
    assert_eq!(hasher.finalize(), sha256(LONG));

    // The NIST one-million-'a' vector, in pieces of 1000 bytes.
    let mut hasher = Sha256Hasher::new();
    let piece = [b'a'; 1000];
    for _ in 0..MILLION / piece.len() {
        hasher.update(&piece);
    }
    assert_eq!(
        hasher.finalize().as_slice(),
        hex("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
    );

    assert_eq!(Sha256Hasher::new().finalize(), sha256(b""));
}
