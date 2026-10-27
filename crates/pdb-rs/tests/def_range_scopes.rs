//! Tests for the def-range checks against inline sites and type sizes.

use pdb_rs::native::codeview::SymbolKind;
use pdb_rs::native::codeview::index::{IdIndex, ModuleSymbolOffset, TypeIndex};
use pdb_rs::native::codeview::symbol::validate::{
    Rule, ValidationContext, ValidationReport, validate_module_symbols,
};
use pdb_rs::native::codeview::symbol::{
    S_DEFRANGE_REGISTER, S_DEFRANGE_REGISTER_REL, S_DEFRANGE_SUBFIELD_REGISTER, S_END, S_GPROC32,
    S_INLINESITE, S_INLINESITE_END, S_LOCAL, S_SEPCODE,
};
use pdb_rs::native::msf::StreamId;

/// The module stream ID in the tests.
const STREAM: StreamId = StreamId(12);
/// The module stream offset of the first record, after the signature.
const BASE: u32 = 4;
/// The start of the procedure code in section 1.
const CODE: u32 = 0x1000;
/// The x64 register `RCX`.
const RCX: u16 = 330;
/// The type `int` (`T_INT4`), 4 bytes.
const INT: u32 = 0x0074;
/// A record type without a known size in the tests.
const UNKNOWN: u32 = 0x1000;

/// A builder for a symbol record region.
#[derive(Default)]
struct Builder {
    /// The serialized records.
    bytes: Vec<u8>,
}

impl Builder {
    /// Add a record with zero padding and return its stream offset.
    fn push(&mut self, kind: SymbolKind, payload: &[u8]) -> u32 {
        let offset = BASE + u32::try_from(self.bytes.len()).unwrap();
        let mut payload = payload.to_vec();
        while !payload.len().is_multiple_of(4) {
            payload.push(0);
        }
        let length = u16::try_from(payload.len() + 2).unwrap();
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(&kind.0.to_le_bytes());
        self.bytes.extend_from_slice(&payload);
        offset
    }

    /// Write the end link of the scope at `scope`.
    fn patch_end(&mut self, scope: u32, end: u32) {
        let start = usize::try_from(scope - BASE).unwrap() + 8;
        self.bytes[start..start + 4].copy_from_slice(&end.to_le_bytes());
    }

    /// Add an end record of `kind` for `scope`.
    fn close(&mut self, kind: SymbolKind, scope: u32) {
        let end = self.push(kind, &[]);
        self.patch_end(scope, end);
    }

    /// Add a procedure at [`CODE`] with 0x100 bytes.
    fn procedure(&mut self) -> u32 {
        let mut payload = Vec::new();
        for value in [0, 0, 0, 0x100, 0, 0, 0, CODE] {
            payload.extend_from_slice(&u32::to_le_bytes(value));
        }
        payload.extend_from_slice(&[1, 0, 0]);
        payload.extend_from_slice(b"f\0");
        self.push(S_GPROC32, &payload)
    }

    /// Add an inline site in `parent` with `annotations`.
    fn inline_site(&mut self, parent: u32, annotations: &[u8]) -> u32 {
        let mut payload = Vec::new();
        for value in [parent, 0, 0x1000] {
            payload.extend_from_slice(&u32::to_le_bytes(value));
        }
        payload.extend_from_slice(annotations);
        self.push(S_INLINESITE, &payload)
    }

    /// Add an `S_LOCAL` record of `type_index`.
    fn local(&mut self, type_index: u32) -> u32 {
        let mut payload = type_index.to_le_bytes().to_vec();
        payload.extend_from_slice(&[0, 0]);
        payload.extend_from_slice(b"v\0");
        self.push(S_LOCAL, &payload)
    }

    /// Add a def-range of `kind` with `head`, a range in section 1 and `gaps`.
    fn ranged(
        &mut self,
        kind: SymbolKind,
        head: &[u8],
        start: u32,
        length: u16,
        gaps: &[(u16, u16)],
    ) -> u32 {
        let mut payload = head.to_vec();
        payload.extend_from_slice(&start.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&length.to_le_bytes());
        for &(gap_start, gap_length) in gaps {
            payload.extend_from_slice(&gap_start.to_le_bytes());
            payload.extend_from_slice(&gap_length.to_le_bytes());
        }
        self.push(kind, &payload)
    }

    /// Add an `S_DEFRANGE_REGISTER` record with range `attributes`.
    fn register(&mut self, attributes: u16, start: u32, length: u16, gaps: &[(u16, u16)]) -> u32 {
        let mut head = RCX.to_le_bytes().to_vec();
        head.extend_from_slice(&attributes.to_le_bytes());
        self.ranged(S_DEFRANGE_REGISTER, &head, start, length, gaps)
    }

    /// Add an `S_SEPCODE` record of the procedure at [`CODE`].
    fn separated(&mut self, start: u32, length: u32) -> u32 {
        let mut payload = Vec::new();
        for value in [0, 0, length, 0, start, CODE] {
            payload.extend_from_slice(&u32::to_le_bytes(value));
        }
        payload.extend_from_slice(&[1, 0, 1, 0]);
        self.push(S_SEPCODE, &payload)
    }
}

/// Return the size of the test types: `int` has 4 bytes.
fn type_size(index: TypeIndex) -> Option<u64> {
    (index.0 == INT).then_some(4)
}

/// Validate a module stream with type sizes.
fn validate(builder: &Builder) -> ValidationReport {
    let context =
        ValidationContext::new(TypeIndex(0x1010), IdIndex(0x1003)).with_type_sizes(&type_size);
    validate_module_symbols(&builder.bytes, STREAM, ModuleSymbolOffset(BASE), &context).unwrap()
}

/// Return the broken rules and their record offsets.
fn rules(report: &ValidationReport) -> Vec<(Rule, u64)> {
    report
        .violations
        .iter()
        .map(|violation| (violation.rule, violation.offset))
        .collect()
}

/// Annotations with one range at 0x10 with 0x10 bytes
/// (`BA_OP_ChangeCodeLengthAndCodeOffset`).
const SITE_10_20: [u8; 3] = [0x0c, 0x10, 0x10];

#[test]
fn accepts_def_ranges_that_overlap_or_touch_the_inline_site() {
    let mut builder = Builder::default();
    let outer = builder.procedure();
    let site = builder.inline_site(outer, &SITE_10_20);
    builder.local(INT);
    builder.register(0, CODE + 0x12, 4, &[]);
    // Starts at the end of the site code, as MSVC writes for a result.
    builder.register(0, CODE + 0x20, 4, &[]);
    // Ends at the start of the site code.
    builder.register(0, CODE + 0x0c, 4, &[]);
    // Overlaps the site code with its first byte.
    builder.register(0, CODE + 0x1f, 8, &[]);
    builder.close(S_INLINESITE_END, site);
    builder.close(S_END, outer);
    assert_eq!(rules(&validate(&builder)), []);
}

#[test]
fn reports_def_ranges_that_miss_the_inline_site() {
    let mut builder = Builder::default();
    let outer = builder.procedure();
    let site = builder.inline_site(outer, &SITE_10_20);
    builder.local(INT);
    let after = builder.register(0, CODE + 0x21, 4, &[]);
    // The gap removes the only part that touches the site code.
    let gap = builder.register(0, CODE + 0x18, 0x20, &[(0, 0x10)]);
    // The "may have no user name" bit turns the check off.
    builder.register(1, CODE + 0x40, 4, &[]);
    builder.close(S_INLINESITE_END, site);
    // Outside an inline site, only the procedure range applies.
    builder.local(INT);
    builder.register(0, CODE + 0x40, 4, &[]);
    builder.close(S_END, outer);
    assert_eq!(
        rules(&validate(&builder)),
        [
            (Rule::DefRangeOutsideInlineSite, u64::from(after)),
            (Rule::DefRangeOutsideInlineSite, u64::from(gap)),
        ]
    );
}

#[test]
fn accepts_def_ranges_in_the_code_of_an_enclosing_inline_site() {
    let mut builder = Builder::default();
    let outer = builder.procedure();
    let site = builder.inline_site(outer, &[0x0c, 0x10, 0x30]);
    let inner = builder.inline_site(site, &SITE_10_20);
    builder.local(INT);
    builder.register(0, CODE + 0x32, 4, &[]);
    let outside = builder.register(0, CODE + 0x60, 4, &[]);
    builder.close(S_INLINESITE_END, inner);
    builder.close(S_INLINESITE_END, site);
    builder.close(S_END, outer);
    assert_eq!(
        rules(&validate(&builder)),
        [(Rule::DefRangeOutsideInlineSite, u64::from(outside))]
    );
}

#[test]
fn uses_separated_code_chunks_of_the_procedure() {
    let mut builder = Builder::default();
    let outer = builder.procedure();
    // `BA_OP_ChangeCodeOffsetBase` 1, then a range at offset 4 with 8 bytes
    // of the first separated chunk, then a range without a length.
    let site = builder.inline_site(outer, &[0x02, 0x01, 0x0c, 0x08, 0x04, 0x03, 0x20]);
    builder.local(INT);
    builder.register(0, CODE + 0x506, 2, &[]);
    // The last range ends with its chunk.
    builder.register(0, CODE + 0x530, 4, &[]);
    let outside = builder.register(0, CODE + 0x14, 4, &[]);
    builder.close(S_INLINESITE_END, site);
    builder.close(S_END, outer);
    let chunk = builder.separated(CODE + 0x500, 0x40);
    builder.close(S_END, chunk);
    assert_eq!(
        rules(&validate(&builder)),
        [(Rule::DefRangeOutsideInlineSite, u64::from(outside))]
    );
}

#[test]
fn skips_sites_in_a_missing_chunk_and_reports_bad_annotations() {
    let mut builder = Builder::default();
    let outer = builder.procedure();
    let site = builder.inline_site(outer, &[0x02, 0x02, 0x0c, 0x08, 0x04]);
    builder.local(INT);
    builder.register(0, CODE + 0x80, 4, &[]);
    builder.close(S_INLINESITE_END, site);
    // A range without a length, then a range that starts before it.
    let bad = builder.inline_site(outer, &[0x03, 0x10, 0x01, 0x00, 0x03, 0x00]);
    builder.local(INT);
    builder.register(0, CODE + 0x80, 4, &[]);
    builder.close(S_INLINESITE_END, bad);
    builder.close(S_END, outer);
    assert_eq!(
        rules(&validate(&builder)),
        [(Rule::InlineSiteAnnotations, u64::from(bad))]
    );
}

#[test]
fn checks_subfield_offsets_against_the_type_size() {
    let mut builder = Builder::default();
    let outer = builder.procedure();
    builder.local(INT);
    let subfield = |offset: u32| {
        let mut head = RCX.to_le_bytes().to_vec();
        head.extend_from_slice(&[0, 0]);
        head.extend_from_slice(&offset.to_le_bytes());
        head
    };
    builder.ranged(S_DEFRANGE_SUBFIELD_REGISTER, &subfield(3), CODE, 4, &[]);
    let past = builder.ranged(S_DEFRANGE_SUBFIELD_REGISTER, &subfield(4), CODE, 4, &[]);
    // `S_DEFRANGE_REGISTER_REL` with the spilled-member bit and offset 4.
    let relative = |flags: u16| {
        let mut head = RCX.to_le_bytes().to_vec();
        head.extend_from_slice(&flags.to_le_bytes());
        head.extend_from_slice(&8_i32.to_le_bytes());
        head
    };
    let spilled = builder.ranged(
        S_DEFRANGE_REGISTER_REL,
        &relative(1 | (4 << 4)),
        CODE,
        4,
        &[],
    );
    // Without the bit, the field is not an offset.
    builder.ranged(S_DEFRANGE_REGISTER_REL, &relative(4 << 4), CODE, 4, &[]);
    // A type without a known size is not checked.
    builder.local(UNKNOWN);
    builder.ranged(S_DEFRANGE_SUBFIELD_REGISTER, &subfield(0x100), CODE, 4, &[]);
    builder.close(S_END, outer);
    assert_eq!(
        rules(&validate(&builder)),
        [
            (Rule::SubfieldOutsideType, u64::from(past)),
            (Rule::SubfieldOutsideType, u64::from(spilled)),
        ]
    );
    // Without type sizes, the check is not done.
    let context = ValidationContext::new(TypeIndex(0x1010), IdIndex(0x1003));
    let report =
        validate_module_symbols(&builder.bytes, STREAM, ModuleSymbolOffset(BASE), &context)
            .unwrap();
    assert_eq!(rules(&report), []);
}
