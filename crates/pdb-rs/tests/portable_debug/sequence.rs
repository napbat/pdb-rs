//! Sequence point blobs: hidden points, document changes, signed deltas, bounds and the
//! Roslyn encoding.

use pdb_rs::ErrorKind;
use pdb_rs::portable::Pdb;
use pdb_rs::portable::debug::{
    HIDDEN_LINE, LocalSignatureForm, MethodDebugInformation, SequencePoint, SequencePoints,
    SourceSpan,
};
use pdb_rs::portable::metadata::{DocumentId, IlOffset, MethodDebugInformationId, StandAloneSigId};

use crate::synthetic::Synthetic;
use crate::{every_prefix_fails, expect};

/// The blob of method 1 of the synthetic PDB: a visible point, a hidden point and a visible
/// point with negative line and column deltas.
const SINGLE: [u8; 14] = [
    0x01, 0x00, 0x00, 0x05, 0x0a, 0x03, 0x02, 0x00, 0x00, 0x03, 0x01, 0x7d, 0x77, 0x02,
];

/// The blob of method 2: a point in document 1, a document record, a point in document 2.
const MULTI: [u8; 14] = [
    0x00, 0x01, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x02, 0x04, 0x00, 0x02, 0x02, 0x00,
];

fn point(il: u32, document: u32, span: Option<(u32, u16, u32, u16)>) -> SequencePoint {
    let mut point = SequencePoint::default();
    point.il_offset = IlOffset(il);
    point.document = DocumentId(document);
    point.span = span.map(|(start_line, start_column, end_line, end_column)| {
        let mut span = SourceSpan::default();
        span.start_line = start_line;
        span.start_column = start_column;
        span.end_line = end_line;
        span.end_column = end_column;
        span
    });
    point
}

fn decode(blob: &[u8], document: u32) -> pdb_rs::Result<Vec<SequencePoint>> {
    SequencePoints::parse(blob, DocumentId(document))?.collect()
}

/// Encode with the `LocalSignature` row form of the specification.
fn encode(
    signature: u32,
    points: &[SequencePoint],
) -> pdb_rs::Result<MethodDebugInformation<'static>> {
    SequencePoints::encode_parts(StandAloneSigId(signature), LocalSignatureForm::Row, points)
}

#[test]
fn hidden_points_and_signed_deltas_decode() {
    let decoded = SequencePoints::parse(&SINGLE, DocumentId(1)).unwrap();
    assert_eq!(decoded.local_signature(), StandAloneSigId(1));
    assert_eq!(decoded.local_signature_form(), LocalSignatureForm::Row);
    let points: Vec<_> = decoded.collect::<Result<_, _>>().unwrap();
    assert_eq!(
        points,
        [
            point(0, 1, Some((10, 3, 10, 8))),
            point(2, 1, None),
            // The start is relative to the previous visible point, not the hidden point:
            // line 10 - 5 and column 3 + 1.
            point(5, 1, Some((5, 4, 6, 2))),
        ]
    );
    assert!(points[1].is_hidden());
}

#[test]
fn document_records_change_the_document() {
    let decoded = SequencePoints::parse(&MULTI, DocumentId(0)).unwrap();
    assert_eq!(decoded.initial_document(), DocumentId(1));
    let points: Vec<_> = decoded.collect::<Result<_, _>>().unwrap();
    assert_eq!(
        points,
        [
            point(0, 1, Some((1, 1, 1, 2))),
            point(4, 2, Some((2, 1, 2, 3)))
        ]
    );
}

#[test]
fn the_encoder_writes_the_roslyn_form() {
    let single = decode(&SINGLE, 1).unwrap();
    let encoded = encode(1, &single).unwrap();
    assert_eq!(encoded.document, DocumentId(1));
    assert_eq!(encoded.sequence_points, SINGLE);

    let multi = decode(&MULTI, 0).unwrap();
    let encoded = encode(0, &multi).unwrap();
    assert_eq!(encoded.document, DocumentId(0));
    assert_eq!(encoded.sequence_points, MULTI);

    let empty = encode(3, &[]).unwrap();
    assert_eq!(empty.document, DocumentId(0));
    assert!(empty.sequence_points.is_empty());
}

#[test]
fn large_values_and_deltas_round_trip() {
    let points = [
        point(0x1000, 3, Some((0x1fff_fffe, 0xfffe, 0x1fff_ffff, 0))),
        point(0x1001, 3, None),
        point(0x1fff_fffe, 1, Some((0x1000_0000, 0xffff, 0x1000_0001, 0))),
        point(0x1fff_fffe, 1, None),
    ];
    // The last two points have the same IL offset, so the encoder rejects them.
    let error = encode(0, &points).unwrap_err();
    expect(&error, ErrorKind::InvalidFormat, "do not increase");
    let points = &points[..3];
    let encoded = encode(2, points).unwrap();
    assert_eq!(encoded.document, DocumentId(0));
    assert_eq!(decode(&encoded.sequence_points, 0).unwrap(), points);
}

#[test]
fn bounds_and_record_rules_are_checked() {
    // A visible point on the hidden line 0xFEEFEE.
    let hidden_line = [0x00, 0x00, 0x00, 0x01, 0xc0, 0xfe, 0xef, 0xee, 0x00];
    expect(
        &decode(&hidden_line, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "hidden line",
    );
    // A start column of 0x10000 does not fit.
    let column = [0x00, 0x00, 0x00, 0x01, 0x01, 0xc0, 0x01, 0x00, 0x00];
    expect(
        &decode(&column, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "column",
    );
    // A negative start line after a delta.
    let negative = [
        0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x02, 0x00, 0x01, 0x05, 0x00,
    ];
    expect(
        &decode(&negative, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "line",
    );
    // On a later line, a negative column delta can make the end column negative.
    let end_column = [0x00, 0x00, 0x01, 0x7f, 0x01, 0x00];
    expect(
        &decode(&end_column, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "column",
    );
    // The IL offset 0x1FFFFFFF plus 1 is out of range.
    let offset = [0x00, 0xdf, 0xff, 0xff, 0xff, 0x00, 0x00, 0x01, 0x00, 0x00];
    expect(
        &decode(&offset, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "IL offset",
    );
    // A document record at the end of the blob.
    let end = [0x00, 0x00, 0x00, 0x00, 0x00, 0x02];
    expect(
        &decode(&end, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "document record",
    );
    // A nil document in a document record and in the header.
    let nil = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00];
    expect(
        &decode(&nil, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "nil",
    );
    expect(
        &decode(&[0x00, 0x00, 0x00], 0).unwrap_err(),
        ErrorKind::InvalidFormat,
        "nil",
    );
    // A header without records, and a LocalSignature token of another table.
    expect(
        &decode(&[0x00], 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "no sequence point",
    );
    let token = [0xc1, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
    expect(
        &decode(&token, 1).unwrap_err(),
        ErrorKind::InvalidFormat,
        "LocalSignature",
    );
}

#[test]
fn the_fsharp_token_form_of_the_local_signature_round_trips() {
    let blob = [0xd1, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00];
    let decoded = SequencePoints::parse(&blob, DocumentId(1)).unwrap();
    assert_eq!(decoded.local_signature(), StandAloneSigId(4));
    let form = decoded.local_signature_form();
    assert_eq!(form, LocalSignatureForm::Token);
    let points: Vec<_> = decoded.collect::<Result<_, _>>().unwrap();
    let encoded = SequencePoints::encode_parts(StandAloneSigId(4), form, &points).unwrap();
    assert_eq!(encoded.sequence_points, blob);
    // The row form writes the row; a nil row is 0 in both forms.
    assert_eq!(encode(4, &points).unwrap().sequence_points[0], 0x04);
    let nil = SequencePoints::encode_parts(StandAloneSigId(0), LocalSignatureForm::Token, &points);
    assert_eq!(nil.unwrap().sequence_points[0], 0x00);
}

#[test]
fn a_nil_local_signature_token_has_the_row_form() {
    // Regression (fuzz target `portable_blobs`): the header `0x11000000` is a
    // `StandAloneSig` token with the nil row. The decoder gave the token form, but the encoder
    // writes a nil row as 0 in both forms, so the encoded blob decoded with the row form.
    let blob = [0xd1, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00];
    let decoded = SequencePoints::parse(&blob, DocumentId(1)).unwrap();
    assert_eq!(decoded.local_signature(), StandAloneSigId(0));
    let form = decoded.local_signature_form();
    assert_eq!(form, LocalSignatureForm::Row);
    let points: Vec<_> = decoded.collect::<Result<_, _>>().unwrap();
    let encoded = SequencePoints::encode_parts(StandAloneSigId(0), form, &points).unwrap();
    let again = SequencePoints::parse(&encoded.sequence_points, encoded.document).unwrap();
    assert_eq!(again.local_signature_form(), form);
    assert_eq!(again.collect::<Result<Vec<_>, _>>().unwrap(), points);
}

#[test]
fn the_encoder_checks_its_input() {
    let nil = [point(0, 0, None)];
    expect(
        &encode(0, &nil).unwrap_err(),
        ErrorKind::InvalidFormat,
        "nil",
    );
    let backwards = [point(0, 1, Some((2, 5, 2, 5)))];
    expect(
        &encode(0, &backwards).unwrap_err(),
        ErrorKind::InvalidFormat,
        "ends before",
    );
    let hidden = [point(0, 1, Some((HIDDEN_LINE, 0, HIDDEN_LINE, 1)))];
    expect(
        &encode(0, &hidden).unwrap_err(),
        ErrorKind::InvalidFormat,
        "hidden line",
    );
}

#[test]
fn truncated_blobs_give_errors() {
    // A prefix that ends after a complete sequence point record is a valid blob.
    for (blob, document, complete) in [(&SINGLE[..], 1, &[6, 9][..]), (&MULTI[..], 0, &[7][..])] {
        for end in 0..blob.len() {
            let result = decode(&blob[..end], document);
            assert_eq!(
                result.is_ok(),
                complete.contains(&end),
                "prefix of {end} bytes"
            );
        }
    }
    every_prefix_fails(&SINGLE[..6], |blob| decode(blob, 1).map(|_| ()));
}

#[test]
fn the_typed_row_reads_the_blob() {
    let bytes = Synthetic::valid().bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let method = pdb
        .method_debug_information(MethodDebugInformationId(2))
        .unwrap();
    assert_eq!(method.document, DocumentId(0));
    let points: Vec<_> = method
        .decode()
        .unwrap()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(points.len(), 2);
    let none = pdb
        .method_debug_information(MethodDebugInformationId(3))
        .unwrap();
    assert!(none.decode().unwrap().is_none());
    assert_eq!(pdb.method_debug_informations().len(), 4);
}
