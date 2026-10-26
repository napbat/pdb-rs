//! Local constant signatures: every type code, enums, general values, custom modifiers and
//! the encoder.

use pdbrs::ErrorKind;
use pdbrs::portable::debug::{
    ConstantValue, CustomModifier, Decimal, GeneralValue, LocalConstantSignature, PrimitiveConstant,
};
use pdbrs::portable::metadata::{MetadataToken, TableId};

use crate::{every_prefix_fails, expect};

fn token(table: TableId, row: u32) -> MetadataToken {
    MetadataToken::new(table, row).unwrap()
}

/// Decode `blob`, check the value, encode it again and compare the bytes.
///
/// For a fixed-size value, each strict prefix of the blob is truncated. An enum or general
/// value has optional trailing parts, so a prefix can be a valid signature.
fn round_trip(blob: &[u8], value: ConstantValue<'_>) {
    let signature = LocalConstantSignature::decode(blob).unwrap();
    assert_eq!(signature.value(), value, "{blob:02x?}");
    let modifiers: Vec<_> = signature.modifiers().collect();
    assert_eq!(
        LocalConstantSignature::encode(&modifiers, &value).unwrap(),
        blob
    );
    if matches!(value, ConstantValue::Primitive(_) | ConstantValue::Object) {
        every_prefix_fails(blob, |blob| {
            LocalConstantSignature::decode(blob).map(|_| ())
        });
    }
}

#[test]
fn every_primitive_type_code_round_trips() {
    let cases: [(&[u8], PrimitiveConstant<'_>); 12] = [
        (&[0x02, 0x01], PrimitiveConstant::Boolean(true)),
        (&[0x03, 0x41, 0x00], PrimitiveConstant::Char(0x41)),
        (&[0x04, 0xfe], PrimitiveConstant::I1(-2)),
        (&[0x05, 0xfe], PrimitiveConstant::U1(0xfe)),
        (&[0x06, 0x00, 0x80], PrimitiveConstant::I2(i16::MIN)),
        (&[0x07, 0xff, 0xff], PrimitiveConstant::U2(u16::MAX)),
        (&[0x08, 0xff, 0xff, 0xff, 0xff], PrimitiveConstant::I4(-1)),
        (
            &[0x09, 0x01, 0x02, 0x03, 0x04],
            PrimitiveConstant::U4(0x0403_0201),
        ),
        (
            &[0x0a, 0, 0, 0, 0, 0, 0, 0, 0x80],
            PrimitiveConstant::I8(i64::MIN),
        ),
        (&[0x0b, 1, 0, 0, 0, 0, 0, 0, 0], PrimitiveConstant::U8(1)),
        (&[0x0c, 0x00, 0x00, 0x80, 0x3f], PrimitiveConstant::R4(1.0)),
        (
            &[0x0d, 0, 0, 0, 0, 0, 0, 0xe0, 0x3f],
            PrimitiveConstant::R8(0.5),
        ),
    ];
    for (blob, value) in cases {
        round_trip(blob, ConstantValue::Primitive(value));
    }
}

#[test]
fn strings_round_trip_with_null_and_empty_values() {
    let null = LocalConstantSignature::decode(&[0x0e, 0xff]).unwrap();
    assert_eq!(
        null.value(),
        ConstantValue::Primitive(PrimitiveConstant::String(None))
    );
    let blob = [0x0e, b'h', 0, b'i', 0];
    let text = LocalConstantSignature::decode(&blob).unwrap();
    assert_eq!(
        text.value(),
        ConstantValue::Primitive(PrimitiveConstant::String(Some(&blob[1..])))
    );
    assert_eq!(
        LocalConstantSignature::encode(&[], &text.value()).unwrap(),
        blob
    );
    let empty = LocalConstantSignature::decode(&[0x0e]).unwrap();
    assert_eq!(
        empty.value(),
        ConstantValue::Primitive(PrimitiveConstant::String(Some(&[])))
    );
    assert_eq!(
        LocalConstantSignature::encode(&[], &null.value()).unwrap(),
        [0x0e, 0xff]
    );
    expect(
        &LocalConstantSignature::decode(&[0x0e, b'h', 0, b'i']).unwrap_err(),
        ErrorKind::InvalidFormat,
        "odd size",
    );
}

#[test]
fn enum_constants_have_an_external_type() {
    // I4 5 with the enum type TypeRef 7 (7 << 2 | 1 = 0x1d).
    round_trip(
        &[0x08, 0x05, 0x00, 0x00, 0x00, 0x1d],
        ConstantValue::Enum {
            value: PrimitiveConstant::I4(5),
            enum_type: token(TableId::TYPE_REF, 7),
        },
    );
    // U1 with the enum type TypeDef 0x100 (0x100 << 2 = 0x400: 0x84 0x00).
    round_trip(
        &[0x05, 0x09, 0x84, 0x00],
        ConstantValue::Enum {
            value: PrimitiveConstant::U1(9),
            enum_type: token(TableId::TYPE_DEF, 0x100),
        },
    );
    // R8 and STRING cannot be enum types.
    expect(
        &LocalConstantSignature::decode(&[0x0c, 0, 0, 0x80, 0x3f, 0x1d]).unwrap_err(),
        ErrorKind::InvalidFormat,
        "after the value",
    );
    let value = ConstantValue::Enum {
        value: PrimitiveConstant::R4(1.0),
        enum_type: token(TableId::TYPE_REF, 1),
    };
    expect(
        &LocalConstantSignature::encode(&[], &value).unwrap_err(),
        ErrorKind::InvalidFormat,
        "R4",
    );
}

#[test]
fn general_constants_decode_by_value_size() {
    // CLASS TypeRef 2 with no value: a null reference.
    round_trip(
        &[0x12, 0x09],
        ConstantValue::General {
            value_type: false,
            constant_type: token(TableId::TYPE_REF, 2),
            value: GeneralValue::Default,
        },
    );
    // VALUETYPE TypeSpec 1 with DateTime ticks.
    let mut blob = vec![0x11, 0x06];
    blob.extend_from_slice(&638_397_504_000_000_000_i64.to_le_bytes());
    round_trip(
        &blob,
        ConstantValue::General {
            value_type: true,
            constant_type: token(TableId::TYPE_SPEC, 1),
            value: GeneralValue::DateTime(638_397_504_000_000_000),
        },
    );
    // VALUETYPE TypeRef 3 with the Decimal -12.75 (sign bit, scale 2, 1275).
    let blob = [0x11, 0x0d, 0x82, 0xfb, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    round_trip(
        &blob,
        ConstantValue::General {
            value_type: true,
            constant_type: token(TableId::TYPE_REF, 3),
            value: GeneralValue::Decimal(Decimal {
                negative: true,
                scale: 2,
                low: 1275,
                mid: 0,
                high: 0,
            }),
        },
    );
    let mut scale = blob;
    scale[2] = 29;
    expect(
        &LocalConstantSignature::decode(&scale).unwrap_err(),
        ErrorKind::InvalidFormat,
        "scale",
    );
    expect(
        &LocalConstantSignature::decode(&[0x12, 0x09, 0x01, 0x02]).unwrap_err(),
        ErrorKind::InvalidFormat,
        "general value",
    );
    round_trip(&[0x1c], ConstantValue::Object);
}

#[test]
fn custom_modifiers_come_before_the_value() {
    // CMOD_REQD TypeRef 1, CMOD_OPT TypeDef 2, then I2 7.
    let blob = [0x1f, 0x05, 0x20, 0x08, 0x06, 0x07, 0x00];
    let signature = LocalConstantSignature::decode(&blob).unwrap();
    let modifiers: Vec<_> = signature.modifiers().collect();
    assert_eq!(
        modifiers,
        [
            CustomModifier {
                required: true,
                modifier: token(TableId::TYPE_REF, 1),
            },
            CustomModifier {
                required: false,
                modifier: token(TableId::TYPE_DEF, 2),
            },
        ]
    );
    round_trip(&blob, ConstantValue::Primitive(PrimitiveConstant::I2(7)));
}

#[test]
fn invalid_signatures_give_errors() {
    for (blob, kind, text) in [
        (&[][..], ErrorKind::UnexpectedEof, "no type code"),
        (&[0x01], ErrorKind::InvalidFormat, "unknown type code"),
        (&[0x13, 0x00], ErrorKind::InvalidFormat, "unknown type code"),
        (&[0x02, 0x02], ErrorKind::InvalidFormat, "BOOLEAN"),
        (&[0x12, 0x03], ErrorKind::InvalidFormat, "tag 3"),
        (&[0x12, 0x01], ErrorKind::InvalidFormat, "nil row"),
        (
            &[0x1f, 0x00, 0x08, 0, 0, 0, 0],
            ErrorKind::InvalidFormat,
            "nil row",
        ),
        (&[0x1c, 0x00], ErrorKind::InvalidFormat, "after the value"),
        (
            &[0x08, 0x05, 0x00, 0x00, 0x00, 0x1d, 0x00],
            ErrorKind::InvalidFormat,
            "after the value",
        ),
        (&[0x08, 0x05, 0x00], ErrorKind::UnexpectedEof, "truncated"),
    ] {
        expect(
            &LocalConstantSignature::decode(blob).unwrap_err(),
            kind,
            text,
        );
    }
    let modifier = [CustomModifier {
        required: true,
        modifier: token(TableId::METHOD_DEF, 1),
    }];
    expect(
        &LocalConstantSignature::encode(&modifier, &ConstantValue::Object).unwrap_err(),
        ErrorKind::InvalidFormat,
        "TypeDef, TypeRef or TypeSpec",
    );
}
