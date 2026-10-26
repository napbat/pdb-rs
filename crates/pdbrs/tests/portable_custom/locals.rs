//! Tests for dynamic local variables and tuple element names.

use pdbrs::ErrorKind;
use pdbrs::portable::custom::{
    self, CustomDebugKind, CustomDebugValue, DynamicLocalVariables, TupleElementNames,
};

use super::{error, round_trip};

#[test]
fn decodes_dynamic_local_variables() {
    let blob = [0b0000_0101, 0x80];
    let CustomDebugValue::DynamicLocalVariables(flags) =
        round_trip(CustomDebugKind::DYNAMIC_LOCAL_VARIABLES, &blob)
    else {
        panic!("expected dynamic local variables");
    };
    assert_eq!(flags.len(), 16);
    assert_eq!(flags.as_bytes(), blob);
    assert!(flags.flag(0) && !flags.flag(1) && flags.flag(2) && flags.flag(15));
    // A flag after the end of the blob is clear.
    assert!(!flags.flag(16) && !flags.flag(usize::MAX));
    let set: Vec<_> = flags
        .iter()
        .enumerate()
        .filter_map(|(index, flag)| flag.then_some(index))
        .collect();
    assert_eq!(set, [0, 2, 15]);
    assert_eq!(flags.iter().next_back(), Some(true));
    assert_eq!(flags.iter().len(), 16);
    assert!(DynamicLocalVariables::new(&[]).is_empty());
}

#[test]
fn encodes_dynamic_local_variables_without_trailing_zero_bytes() {
    assert_eq!(
        DynamicLocalVariables::encode([true, false, true]).unwrap(),
        [0b101]
    );
    let mut flags = [false; 17];
    flags[0] = true;
    assert_eq!(DynamicLocalVariables::encode(flags).unwrap(), [1]);
    flags[16] = true;
    assert_eq!(DynamicLocalVariables::encode(flags).unwrap(), [1, 0, 1]);
    assert_eq!(DynamicLocalVariables::encode([false; 9]).unwrap(), []);
    // A blob with a trailing zero byte decodes, and encodes without it.
    let value = custom::decode(CustomDebugKind::DYNAMIC_LOCAL_VARIABLES, &[1, 0]).unwrap();
    assert_eq!(value.encode().unwrap(), [1]);
}

#[test]
fn decodes_tuple_element_names() {
    let blob = b"a\0\0bc\0";
    let CustomDebugValue::TupleElementNames(names) =
        round_trip(CustomDebugKind::TUPLE_ELEMENT_NAMES, blob)
    else {
        panic!("expected tuple element names");
    };
    assert_eq!(names.len(), 3);
    assert_eq!(names.as_bytes(), blob);
    let names: Vec<_> = names.iter().map(|name| name.as_bytes()).collect();
    // An empty name is an element without a name.
    assert_eq!(names, [&b"a"[..], b"", b"bc"]);
    assert!(TupleElementNames::parse(&[]).unwrap().is_empty());
}

#[test]
fn rejects_malformed_tuple_element_names() {
    let kind = CustomDebugKind::TUPLE_ELEMENT_NAMES;
    assert_eq!(error(kind, b"a"), ErrorKind::UnexpectedEof);
    assert_eq!(error(kind, b"a\0b"), ErrorKind::UnexpectedEof);
    assert_eq!(
        TupleElementNames::encode([&b"a\0b"[..]])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        TupleElementNames::encode([&b"x"[..], b""]).unwrap(),
        b"x\0\0"
    );
}
