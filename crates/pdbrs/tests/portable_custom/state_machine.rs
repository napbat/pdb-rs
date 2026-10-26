//! Tests for async method stepping information and hoisted local scopes.

use pdbrs::ErrorKind;
use pdbrs::portable::custom::{
    AsyncMethodSteppingInformation, AsyncStep, CustomDebugKind, CustomDebugValue,
    HoistedLocalScope, HoistedLocalScopes,
};

use super::{error, round_trip};

/// Return a step.
const fn step(yield_offset: u32, resume_offset: u32, move_next_method: u32) -> AsyncStep {
    AsyncStep {
        yield_offset,
        resume_offset,
        move_next_method,
    }
}

/// Return the blob of an async method stepping information.
fn stepping(catch: u32, steps: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let mut blob = catch.to_le_bytes().to_vec();
    for (yield_offset, resume_offset, method) in steps {
        blob.extend_from_slice(&yield_offset.to_le_bytes());
        blob.extend_from_slice(&resume_offset.to_le_bytes());
        blob.extend_from_slice(method);
    }
    blob
}

#[test]
fn decodes_async_method_stepping_information() {
    let blob = stepping(
        0x299,
        &[(0x19a, 0x1b8, &[0x4f]), (0x200, 0x220, &[0x81, 0x00])],
    );
    let CustomDebugValue::AsyncMethodSteppingInformation(info) =
        round_trip(CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION, &blob)
    else {
        panic!("expected async stepping information");
    };
    assert_eq!(info.catch_handler_offset(), Some(0x298));
    assert_eq!(info.len(), 2);
    assert_eq!(
        info.iter().collect::<Vec<_>>(),
        [step(0x19a, 0x1b8, 0x4f), step(0x200, 0x220, 0x100)]
    );

    // A catch handler offset of 0 means no handler. No step is valid.
    let info = AsyncMethodSteppingInformation::parse(&[0; 4]).unwrap();
    assert_eq!(info.catch_handler_offset(), None);
    assert!(info.is_empty());
    // The largest stored value is `u32::MAX`.
    let info = AsyncMethodSteppingInformation::parse(&[0xff; 4]).unwrap();
    assert_eq!(info.catch_handler_offset(), Some(u32::MAX - 1));
}

#[test]
fn rejects_malformed_async_method_stepping_information() {
    let kind = CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION;
    let blob = stepping(0, &[(1, 2, &[0x81, 0x00])]);
    for length in (0..4).chain(5..blob.len()) {
        assert_eq!(
            error(kind, &blob[..length]),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
    assert_eq!(
        error(kind, &stepping(0, &[(1, 2, &[0])])),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        error(kind, &stepping(0, &[(1, 2, &[0xe0])])),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn encodes_async_method_stepping_information() {
    assert_eq!(
        AsyncMethodSteppingInformation::encode(None, []).unwrap(),
        [0; 4]
    );
    assert_eq!(
        AsyncMethodSteppingInformation::encode(Some(u32::MAX - 1), [step(1, 2, 0x1fff_ffff)])
            .unwrap(),
        stepping(u32::MAX, &[(1, 2, &[0xdf, 0xff, 0xff, 0xff])])
    );
    for (catch, steps, expected) in [
        (Some(u32::MAX), vec![], ErrorKind::LimitExceeded),
        (None, vec![step(1, 2, 0)], ErrorKind::InvalidFormat),
        (
            None,
            vec![step(1, 2, 0x2000_0000)],
            ErrorKind::LimitExceeded,
        ),
    ] {
        assert_eq!(
            AsyncMethodSteppingInformation::encode(catch, steps)
                .unwrap_err()
                .kind(),
            expected
        );
    }
}

/// Return the blob of hoisted local scopes.
fn scopes_blob(scopes: &[(u32, u32)]) -> Vec<u8> {
    scopes
        .iter()
        .flat_map(|(start, length)| [start.to_le_bytes(), length.to_le_bytes()])
        .flatten()
        .collect()
}

#[test]
fn decodes_hoisted_local_scopes() {
    // `(0, 0)` marks a slot without a scope; the limits are `0x7fffffff`.
    let blob = scopes_blob(&[(0x10, 0x20), (0, 0), (0x7fff_ffff, 0x7fff_ffff)]);
    let CustomDebugValue::StateMachineHoistedLocalScopes(scopes) =
        round_trip(CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES, &blob)
    else {
        panic!("expected hoisted local scopes");
    };
    assert_eq!(scopes.len(), 3);
    assert_eq!(scopes.as_bytes(), blob);
    assert_eq!(
        scopes.iter().collect::<Vec<_>>(),
        [
            HoistedLocalScope {
                start: 0x10,
                length: 0x20
            },
            HoistedLocalScope::default(),
            HoistedLocalScope {
                start: 0x7fff_ffff,
                length: 0x7fff_ffff
            },
        ]
    );
    assert!(HoistedLocalScopes::parse(&[]).unwrap().is_empty());
}

#[test]
fn rejects_malformed_hoisted_local_scopes() {
    let kind = CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES;
    let blob = scopes_blob(&[(1, 2), (3, 4)]);
    for length in (1..8).chain(9..16) {
        assert_eq!(
            error(kind, &blob[..length]),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
    for scope in [(0x8000_0000, 1), (1, 0x8000_0000)] {
        assert_eq!(
            error(kind, &scopes_blob(&[scope])),
            ErrorKind::InvalidFormat
        );
        let scope = HoistedLocalScope {
            start: scope.0,
            length: scope.1,
        };
        assert_eq!(
            HoistedLocalScopes::encode([scope]).unwrap_err().kind(),
            ErrorKind::InvalidFormat
        );
    }
}
