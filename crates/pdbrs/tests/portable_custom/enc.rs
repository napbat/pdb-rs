//! Tests for the Edit and Continue local slot, lambda and state machine maps.

use pdbrs::ErrorKind;
use pdbrs::portable::custom::{
    self, CustomDebugKind, CustomDebugValue, EncLambdaAndClosureMap, EncLocalSlotMap,
    EncStateMachineStateMap, Lambda, LambdaClosure, LocalSlot, MAX_LOCAL_SLOT_KIND,
    StateMachineState,
};

use super::{error, round_trip};

/// Return a variable slot.
const fn variable(kind: u8, syntax_offset: i32, ordinal: Option<u32>) -> LocalSlot {
    LocalSlot::Variable {
        kind,
        syntax_offset,
        ordinal,
    }
}

/// Decode a local slot map and return its slots.
fn slots(blob: &[u8]) -> Vec<LocalSlot> {
    let CustomDebugValue::EncLocalSlotMap(map) =
        round_trip(CustomDebugKind::ENC_LOCAL_SLOT_MAP, blob)
    else {
        panic!("expected a local slot map");
    };
    assert_eq!(map.as_bytes(), blob);
    assert_eq!(map.len(), map.iter().count());
    map.iter().collect()
}

#[test]
fn decodes_local_slot_maps() {
    // Without a stored baseline, the baseline is -1.
    assert_eq!(
        slots(&[0x01, 0x0f, 0x00, 0x1d, 0x54]),
        [
            variable(0, 14, None),
            LocalSlot::Temporary,
            variable(0x1c, 83, None)
        ]
    );
    // A stored baseline -5, an ordinal and the largest kind.
    let blob = [0xff, 0x05, 0x01, 0x00, 0x9d, 0x08, 0x01, 0x7e, 0x05];
    assert_eq!(
        slots(&blob),
        [
            variable(0, -5, None),
            variable(0x1c, 3, Some(1)),
            variable(MAX_LOCAL_SLOT_KIND, 0, None),
        ]
    );
    let map = EncLocalSlotMap::parse(&blob).unwrap();
    assert_eq!(map.syntax_offset_baseline(), -5);
    // A stored ordinal 0 encodes again without change.
    assert_eq!(slots(&[0x81, 0x01, 0x00]), [variable(0, 0, Some(0))]);
    assert_eq!(slots(&[]).len(), 0);
}

#[test]
fn encodes_local_slot_maps_as_roslyn() {
    // A baseline that is not necessary decodes, and encodes without it.
    let value = custom::decode(
        CustomDebugKind::ENC_LOCAL_SLOT_MAP,
        &[0xff, 0x01, 0x01, 0x00],
    )
    .unwrap();
    assert_eq!(value.encode().unwrap(), [0x01, 0x00]);
    assert_eq!(
        EncLocalSlotMap::encode([variable(0, -1, None)]).unwrap(),
        [0x01, 0x00]
    );
    for (slot, expected) in [
        (
            variable(MAX_LOCAL_SLOT_KIND + 1, 0, None),
            ErrorKind::InvalidFormat,
        ),
        (variable(0, i32::MIN, None), ErrorKind::LimitExceeded),
        (variable(0, i32::MAX, None), ErrorKind::LimitExceeded),
        (variable(0, 0, Some(0x2000_0000)), ErrorKind::LimitExceeded),
    ] {
        assert_eq!(
            EncLocalSlotMap::encode([slot]).unwrap_err().kind(),
            expected,
            "{slot:?}"
        );
    }
}

#[test]
fn rejects_malformed_local_slot_maps() {
    let kind = CustomDebugKind::ENC_LOCAL_SLOT_MAP;
    for blob in [&[0xff][..], &[0x01], &[0x81, 0x00], &[0x01, 0x80]] {
        assert_eq!(error(kind, blob), ErrorKind::UnexpectedEof, "{blob:x?}");
    }
    for blob in [
        &[0x7f, 0x00][..],
        &[0x80, 0x00, 0x00],
        &[0x01, 0x00, 0xff, 0x01],
        &[0x01, 0xe0],
    ] {
        assert_eq!(error(kind, blob), ErrorKind::InvalidFormat, "{blob:x?}");
    }
}

/// The blob of a lambda map: method ordinal -1, baseline -1, closures at 10
/// and 11, and three lambdas.
const LAMBDA_MAP: [u8; 11] = [0, 1, 2, 0x0b, 0x0c, 0x15, 0x00, 0x16, 0x01, 0x17, 0x03];

#[test]
fn decodes_lambda_and_closure_maps() {
    let CustomDebugValue::EncLambdaAndClosureMap(map) =
        round_trip(CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP, &LAMBDA_MAP)
    else {
        panic!("expected a lambda map");
    };
    assert_eq!(map.as_bytes(), LAMBDA_MAP);
    assert_eq!(map.method_ordinal(), -1);
    assert_eq!(map.syntax_offset_baseline(), -1);
    assert_eq!(map.closures().collect::<Vec<_>>(), [10, 11]);
    let lambda = |syntax_offset, closure| Lambda {
        syntax_offset,
        closure,
    };
    assert_eq!(
        map.lambdas().collect::<Vec<_>>(),
        [
            lambda(20, LambdaClosure::ThisOnly),
            lambda(21, LambdaClosure::Static),
            lambda(22, LambdaClosure::Closure(1)),
        ]
    );
    // Negative syntax offsets give a smaller baseline.
    let blob =
        EncLambdaAndClosureMap::encode(0, [-10], [lambda(-20, LambdaClosure::Closure(0))]).unwrap();
    assert_eq!(blob, [1, 20, 1, 10, 0, 2]);
    let map = EncLambdaAndClosureMap::parse(&blob).unwrap();
    assert_eq!(map.syntax_offset_baseline(), -20);
    assert_eq!(map.closures().collect::<Vec<_>>(), [-10]);
    assert_eq!(map.lambdas().next().unwrap().syntax_offset, -20);
    // No closures and no lambdas.
    assert_eq!(
        EncLambdaAndClosureMap::encode(5, [], []).unwrap(),
        [6, 1, 0]
    );
}

#[test]
fn rejects_malformed_lambda_and_closure_maps() {
    let kind = CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP;
    for length in 0..5 {
        assert_eq!(
            error(kind, &LAMBDA_MAP[..length]),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
    for length in [6, 8, 10] {
        assert_eq!(
            error(kind, &LAMBDA_MAP[..length]),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
    // The closure index 2 is not below the closure count 2.
    let mut blob = LAMBDA_MAP;
    blob[10] = 4;
    assert_eq!(error(kind, &blob), ErrorKind::InvalidFormat);
    assert_eq!(error(kind, &[1, 1, 0, 0x0b, 2]), ErrorKind::InvalidFormat);
    assert_eq!(error(kind, &[0xe0]), ErrorKind::InvalidFormat);

    let lambda = Lambda {
        syntax_offset: 0,
        closure: LambdaClosure::Closure(1),
    };
    assert_eq!(
        EncLambdaAndClosureMap::encode(0, [0], [lambda])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        EncLambdaAndClosureMap::encode(-2, [], [])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        EncLambdaAndClosureMap::encode(0x1fff_ffff, [], [])
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
}

/// The state map of `ComputeAsync` in the `dotnet-csharp-full` fixture.
const STATE_MAP: [u8; 8] = [0x03, 0x00, 0x00, 0x2b, 0x02, 0x53, 0x04, 0x78];

/// Return a state.
const fn state(state_number: i32, syntax_offset: i32) -> StateMachineState {
    StateMachineState {
        state_number,
        syntax_offset,
    }
}

#[test]
fn decodes_state_machine_state_maps() {
    let CustomDebugValue::EncStateMachineStateMap(map) =
        round_trip(CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP, &STATE_MAP)
    else {
        panic!("expected a state machine state map");
    };
    assert_eq!(map.as_bytes(), STATE_MAP);
    assert_eq!(map.len(), 3);
    assert_eq!(map.syntax_offset_baseline(), 0);
    assert_eq!(
        map.iter().collect::<Vec<_>>(),
        [state(0, 43), state(1, 83), state(2, 120)]
    );
    assert_eq!(map.padding(), []);

    // Negative state numbers and syntax offsets, and equal syntax offsets.
    let states = [state(-3, -4), state(-4, -4), state(0x0fff_ffff, 7)];
    let blob = EncStateMachineStateMap::encode(states).unwrap();
    let map = EncStateMachineStateMap::parse(&blob).unwrap();
    assert_eq!(map.syntax_offset_baseline(), -4);
    assert_eq!(map.iter().collect::<Vec<_>>(), states);
    assert_eq!(map.into_iter().len(), 3);

    let empty = EncStateMachineStateMap::parse(&[0]).unwrap();
    assert!(empty.is_empty());
    assert_eq!(EncStateMachineStateMap::encode([]).unwrap(), [0]);
}

#[test]
fn accepts_pdb2pdb_padding_after_states() {
    for pad in 1..=3 {
        let mut blob = STATE_MAP.to_vec();
        blob.resize(STATE_MAP.len() + pad, 0);
        let map = EncStateMachineStateMap::parse(&blob).unwrap();
        assert_eq!(map.padding().len(), pad);
        assert_eq!(map.as_bytes(), blob);
        assert_eq!(map.len(), 3);
        let value = custom::decode(CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP, &blob).unwrap();
        // The encoder writes the form without padding.
        assert_eq!(value.encode().unwrap(), STATE_MAP);
    }
}

#[test]
fn rejects_malformed_state_machine_state_maps() {
    let kind = CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP;
    for length in 0..STATE_MAP.len() {
        assert_eq!(
            error(kind, &STATE_MAP[..length]),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
    let mut blob = STATE_MAP.to_vec();
    blob.extend_from_slice(&[0; 4]);
    assert_eq!(error(kind, &blob), ErrorKind::InvalidFormat);
    blob.truncate(STATE_MAP.len() + 1);
    blob[STATE_MAP.len()] = 1;
    assert_eq!(error(kind, &blob), ErrorKind::InvalidFormat);
    // The syntax offsets 10 and 5 decrease.
    assert_eq!(
        error(kind, &[2, 0, 0, 0x0a, 0, 0x05]),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        EncStateMachineStateMap::encode([state(0, 10), state(1, 5)])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn limits_states_with_the_same_syntax_offset() {
    let states = |count| vec![state(0, 1); count];
    let blob = EncStateMachineStateMap::encode(states(256)).unwrap();
    assert_eq!(EncStateMachineStateMap::parse(&blob).unwrap().len(), 256);
    assert_eq!(
        EncStateMachineStateMap::encode(states(257))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    // 257 states: the count needs two bytes.
    let mut blob = vec![0x81, 0x01, 0x00];
    for _ in 0..257 {
        blob.extend_from_slice(&[0, 1]);
    }
    assert_eq!(
        error(CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP, &blob),
        ErrorKind::InvalidFormat
    );
}
