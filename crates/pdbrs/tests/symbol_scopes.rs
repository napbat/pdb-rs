//! Tests for the scope checks of module symbol streams.

use pdbrs::native::codeview::symbol::{
    ModuleSymbols, S_BLOCK32, S_END, S_GPROC32, S_INLINESITE, S_INLINESITE_END, S_LABEL32, S_LOCAL,
    S_PROC_ID_END, S_THUNK32, ScopedSymbol,
};
use pdbrs::native::msf::StreamId;
use pdbrs::{Error, ErrorKind};

/// The module stream ID in the tests.
const STREAM: StreamId = StreamId(12);
/// The module stream offset of the first record, after the signature.
const BASE: u32 = 4;
/// The offset of the parent field in each scope start payload.
const PARENT: usize = 0;
/// The offset of the end field in each scope start payload.
const END: usize = 4;

/// A builder for a module symbol region.
#[derive(Default)]
struct Builder {
    /// The serialized records.
    bytes: Vec<u8>,
}

impl Builder {
    /// Add a record with zero padding and return its module stream offset.
    fn push(&mut self, kind: u16, payload: &[u8]) -> u32 {
        let offset = BASE + u32::try_from(self.bytes.len()).unwrap();
        let mut payload = payload.to_vec();
        while !(payload.len() + 4).is_multiple_of(4) {
            payload.push(0);
        }
        let length = u16::try_from(payload.len() + 2).unwrap();
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(&kind.to_le_bytes());
        self.bytes.extend_from_slice(&payload);
        offset
    }

    /// Write `value` into the payload of the record at `record`.
    fn patch(&mut self, record: u32, field: usize, value: u32) {
        let start = usize::try_from(record - BASE).unwrap() + 4 + field;
        self.bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Add a scope start with `parent` and an end link to patch later.
    fn open(&mut self, kind: u16, parent: u32) -> u32 {
        let mut payload = Vec::new();
        payload.extend_from_slice(&parent.to_le_bytes());
        payload.extend_from_slice(&0_u32.to_le_bytes());
        match kind {
            S_INLINESITE => payload.extend_from_slice(&0x1001_u32.to_le_bytes()),
            S_BLOCK32 => payload.extend_from_slice(&[0; 10]),
            S_THUNK32 => payload.extend_from_slice(&[0; 13]),
            _ => payload.extend_from_slice(&[0; 27]),
        }
        if kind != S_INLINESITE {
            payload.extend_from_slice(b"f\0");
        }
        self.push(kind, &payload)
    }

    /// Add an end record and patch the end link of `scope`.
    fn close(&mut self, kind: u16, scope: u32) -> u32 {
        let offset = self.push(kind, &[]);
        self.patch(scope, END, offset);
        offset
    }

    /// Add a label.
    fn label(&mut self) -> u32 {
        self.push(S_LABEL32, &[0, 0, 0, 0, 1, 0, 0, b'L', 0])
    }

    fn walk(&self, max_depth: usize) -> Vec<Result<ScopedSymbol<'_>, Error>> {
        ModuleSymbols::new(&self.bytes, STREAM, BASE, max_depth).collect()
    }
}

/// Return the error and the module stream offset of the failed walk.
fn failure(builder: &Builder, max_depth: usize) -> (ErrorKind, u64) {
    let results = builder.walk(max_depth);
    let error = results.last().unwrap().as_ref().unwrap_err();
    assert!(results[..results.len() - 1].iter().all(Result::is_ok));
    let (stream, offset) = error.stream_location().unwrap();
    assert_eq!(stream, STREAM.0);
    (error.kind(), offset)
}

/// Build `proc { label block { inline { inline { } } } } thunk { }`.
fn nested() -> (Builder, [u32; 11]) {
    let mut builder = Builder::default();
    let procedure = builder.open(S_GPROC32, 0);
    let label = builder.label();
    let block = builder.open(S_BLOCK32, procedure);
    let outer = builder.open(S_INLINESITE, block);
    let inner = builder.open(S_INLINESITE, outer);
    let inner_end = builder.close(S_INLINESITE_END, inner);
    let outer_end = builder.close(S_INLINESITE_END, outer);
    let block_end = builder.close(S_END, block);
    let procedure_end = builder.close(S_PROC_ID_END, procedure);
    let thunk = builder.open(S_THUNK32, 0);
    let thunk_end = builder.close(S_END, thunk);
    let offsets = [
        procedure,
        label,
        block,
        outer,
        inner,
        inner_end,
        outer_end,
        block_end,
        procedure_end,
        thunk,
        thunk_end,
    ];
    (builder, offsets)
}

#[test]
fn walks_valid_nested_scopes() {
    let (builder, offsets) = nested();
    let symbols: Vec<ScopedSymbol<'_>> = builder.walk(4).into_iter().map(Result::unwrap).collect();
    let [procedure, _, block, outer, inner, _, _, _, _, thunk, _] = offsets;

    let places: Vec<(u32, usize, Option<u32>)> = symbols
        .iter()
        .map(|symbol| (symbol.offset, symbol.depth, symbol.scope))
        .collect();
    let expected = [
        (offsets[0], 0, None),
        (offsets[1], 1, Some(procedure)),
        (offsets[2], 1, Some(procedure)),
        (offsets[3], 2, Some(block)),
        (offsets[4], 3, Some(outer)),
        (offsets[5], 4, Some(inner)),
        (offsets[6], 3, Some(outer)),
        (offsets[7], 2, Some(block)),
        (offsets[8], 1, Some(procedure)),
        (offsets[9], 0, None),
        (offsets[10], 1, Some(thunk)),
    ];
    assert_eq!(places, expected);
    assert_eq!(symbols[1].symbol.kind(), S_LABEL32);
    assert_eq!(symbols[0].symbol.record().offset + BASE, procedure);
}

#[test]
fn walks_empty_stream_and_flat_records() {
    assert!(Builder::default().walk(0).is_empty());
    let mut builder = Builder::default();
    builder.push(S_LOCAL, &[0x10, 0x10, 0, 0, 0, 0, b'x', 0]);
    builder.label();
    let symbols = builder.walk(0);
    assert_eq!(symbols.len(), 2);
    assert!(symbols.iter().all(|symbol| {
        let symbol = symbol.as_ref().unwrap();
        (symbol.depth, symbol.scope) == (0, None)
    }));
}

#[test]
fn rejects_parent_that_is_not_the_enclosing_scope() {
    let (mut builder, offsets) = nested();
    // The inner inline site claims the block as its parent.
    builder.patch(offsets[4], PARENT, offsets[2]);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidReference, u64::from(offsets[4]))
    );

    let (mut builder, offsets) = nested();
    // A top-level thunk must have parent 0.
    builder.patch(offsets[9], PARENT, offsets[0]);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidReference, u64::from(offsets[9]))
    );
}

#[test]
fn rejects_end_link_that_misses_the_end_record() {
    let (mut builder, offsets) = nested();
    builder.patch(offsets[2], END, offsets[8]);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidReference, u64::from(offsets[2]))
    );
}

#[test]
fn rejects_end_link_before_the_start() {
    let (mut builder, offsets) = nested();
    builder.patch(offsets[3], END, offsets[3]);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidReference, u64::from(offsets[3]))
    );
}

#[test]
fn rejects_end_kind_that_does_not_match_the_scope() {
    let mut builder = Builder::default();
    let procedure = builder.open(S_GPROC32, 0);
    let end = builder.close(S_INLINESITE_END, procedure);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidFormat, u64::from(end))
    );

    let mut builder = Builder::default();
    let procedure = builder.open(S_GPROC32, 0);
    let site = builder.open(S_INLINESITE, procedure);
    let end = builder.close(S_END, site);
    builder.patch(procedure, END, 0x1000);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidFormat, u64::from(end))
    );
}

#[test]
fn rejects_end_without_open_scope() {
    let mut builder = Builder::default();
    builder.label();
    let end = builder.push(S_END, &[]);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidFormat, u64::from(end))
    );
}

#[test]
fn rejects_scope_without_end_record() {
    let mut builder = Builder::default();
    let procedure = builder.open(S_GPROC32, 0);
    let block = builder.open(S_BLOCK32, procedure);
    builder.patch(procedure, END, 0x1000);
    builder.patch(block, END, 0x1000);
    builder.label();
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidFormat, u64::from(block))
    );
}

#[test]
fn enforces_the_depth_limit() {
    let (builder, offsets) = nested();
    assert!(builder.walk(4).iter().all(Result::is_ok));
    assert_eq!(
        failure(&builder, 3),
        (ErrorKind::LimitExceeded, u64::from(offsets[4]))
    );
    assert_eq!(
        failure(&builder, 0),
        (ErrorKind::LimitExceeded, u64::from(offsets[0]))
    );
}

#[test]
fn rejects_scope_kinds_that_are_not_decoded() {
    let mut builder = Builder::default();
    builder.label();
    // S_SEPCODE starts a scope, but this crate does not decode it.
    let separated = builder.push(0x1132, &[0; 24]);
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::UnsupportedVersion, u64::from(separated))
    );
}

#[test]
fn adds_location_to_record_errors() {
    let mut builder = Builder::default();
    builder.label();
    // An S_LABEL32 payload of 12 bytes without a name terminator.
    let label = builder.push(
        S_LABEL32,
        &[0, 0, 0, 0, 1, 0, 0, b'L', b'L', b'L', b'L', b'L'],
    );
    assert_eq!(
        failure(&builder, 4),
        (ErrorKind::InvalidFormat, u64::from(label))
    );

    let mut builder = Builder::default();
    let label = builder.label();
    builder.bytes.extend_from_slice(&[8, 0, 5]);
    let truncated = u64::from(label) + 16;
    assert_eq!(failure(&builder, 4), (ErrorKind::UnexpectedEof, truncated));
}

#[test]
fn stops_after_an_error() {
    let mut builder = Builder::default();
    builder.push(S_END, &[]);
    builder.label();
    let mut symbols = ModuleSymbols::new(&builder.bytes, STREAM, BASE, 4);
    assert!(symbols.next().unwrap().is_err());
    assert!(symbols.next().is_none());
    assert!(symbols.next().is_none());
}
