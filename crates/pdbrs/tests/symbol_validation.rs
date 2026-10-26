//! Tests for the validation of module and shared symbol records.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::id_record::{LF_BUILDINFO, LF_FUNC_ID, LF_STRING_ID};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::symbol::validate::{
    ModuleRecord, Rule, ValidationContext, ValidationReport, validate_module_symbols,
    validate_shared_symbols,
};
use pdbrs::native::codeview::symbol::{
    S_BLOCK32, S_BUILDINFO, S_COMPILE3, S_DEFRANGE_FRAMEPOINTER_REL,
    S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, S_DEFRANGE_REGISTER, S_END, S_FILESTATIC, S_GMANPROC,
    S_GPROC32, S_GPROC32_ID, S_INLINESITE, S_INLINESITE_END, S_LABEL32, S_LOCAL, S_LPROC32,
    S_LPROCREF, S_PROC_ID_END, S_PROCREF, S_SKIP, S_THUNK32, S_UDT,
};
use pdbrs::native::msf::StreamId;

/// The module stream ID in the tests.
const STREAM: StreamId = StreamId(12);
/// The module stream offset of the first record, after the signature.
const BASE: u32 = 4;
/// The offset of the end field in each scope start payload.
const END: usize = 4;
/// The offset of the next field in procedure and thunk payloads.
const NEXT: usize = 8;
/// The x64 CPU value (`CV_CPU_TYPE_e`).
const X64: u16 = 0xd0;
/// The x64 register `RCX` (`CV_AMD64_RCX`).
const RCX: u16 = 330;
/// The local flag of a parameter.
const PARAMETER: u16 = 1;
/// The local flag of a variable without storage.
const OPTIMIZED_OUT: u16 = 1 << 8;
/// The start of the procedure code in section 1.
const CODE: u32 = 0x1000;

/// A builder for a symbol record region.
#[derive(Default)]
struct Builder {
    /// The serialized records.
    bytes: Vec<u8>,
}

impl Builder {
    /// Add a record with zero padding and return its stream offset.
    fn push(&mut self, kind: u16, payload: &[u8]) -> u32 {
        let offset = BASE + u32::try_from(self.bytes.len()).unwrap();
        let mut payload = payload.to_vec();
        while !payload.len().is_multiple_of(4) {
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

    /// Add an `S_COMPILE3` record for `machine`.
    fn compile(&mut self, machine: u16) -> u32 {
        let mut payload = vec![0; 4];
        payload.extend_from_slice(&machine.to_le_bytes());
        payload.extend_from_slice(&[0; 16]);
        payload.extend_from_slice(b"test\0");
        self.push(S_COMPILE3, &payload)
    }

    /// Add a procedure with an end link to patch later.
    fn procedure(&mut self, kind: u16, index: u32, start: u32, length: u32) -> u32 {
        let mut payload = Vec::new();
        for value in [0, 0, 0, length, 0, 0, index, start] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&[1, 0, 0]);
        payload.extend_from_slice(b"f\0");
        self.push(kind, &payload)
    }

    /// Add a block in `parent` with an end link to patch later.
    fn block(&mut self, parent: u32, start: u32, length: u32) -> u32 {
        let mut payload = Vec::new();
        for value in [parent, 0, length, start] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&[1, 0, 0]);
        self.push(S_BLOCK32, &payload)
    }

    /// Add an inline site in `parent` with `annotations`.
    fn inline_site(&mut self, parent: u32, inlinee: u32, annotations: &[u8]) -> u32 {
        let mut payload = Vec::new();
        for value in [parent, 0, inlinee] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(annotations);
        self.push(S_INLINESITE, &payload)
    }

    /// Add a thunk at the top level with a `next` link.
    fn thunk(&mut self, next: u32) -> u32 {
        let mut payload = Vec::new();
        for value in [0, 0, next, CODE + 0x800] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&[1, 0, 6, 0, 0]);
        payload.extend_from_slice(b"t\0");
        self.push(S_THUNK32, &payload)
    }

    /// Add an end record and patch the end link of `scope`.
    fn close(&mut self, kind: u16, scope: u32) -> u32 {
        let offset = self.push(kind, &[]);
        self.patch(scope, END, offset);
        offset
    }

    /// Add an `S_LOCAL` record.
    fn local(&mut self, type_index: u32, flags: u16) -> u32 {
        let mut payload = type_index.to_le_bytes().to_vec();
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(b"v\0");
        self.push(S_LOCAL, &payload)
    }

    /// Add an `S_DEFRANGE_REGISTER` record with `gaps`.
    fn register_range(
        &mut self,
        register: u16,
        start: u32,
        length: u16,
        gaps: &[(u16, u16)],
    ) -> u32 {
        let mut payload = register.to_le_bytes().to_vec();
        payload.extend_from_slice(&[0, 0]);
        payload.extend_from_slice(&start.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&length.to_le_bytes());
        for &(gap_start, gap_length) in gaps {
            payload.extend_from_slice(&gap_start.to_le_bytes());
            payload.extend_from_slice(&gap_length.to_le_bytes());
        }
        self.push(S_DEFRANGE_REGISTER, &payload)
    }

    /// Add an `S_DEFRANGE_FRAMEPOINTER_REL` record without gaps.
    fn frame_range(&mut self, start: u32, length: u16) -> u32 {
        let mut payload = 8_i32.to_le_bytes().to_vec();
        payload.extend_from_slice(&start.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&length.to_le_bytes());
        self.push(S_DEFRANGE_FRAMEPOINTER_REL, &payload)
    }

    /// Add an `S_UDT` record.
    fn udt(&mut self, type_index: u32) -> u32 {
        let mut payload = type_index.to_le_bytes().to_vec();
        payload.extend_from_slice(b"u\0");
        self.push(S_UDT, &payload)
    }

    /// Add a procedure reference record.
    fn reference(&mut self, kind: u16, module: u16, offset: u32) -> u32 {
        let mut payload = vec![0; 4];
        payload.extend_from_slice(&offset.to_le_bytes());
        payload.extend_from_slice(&module.to_le_bytes());
        payload.extend_from_slice(b"r\0");
        self.push(kind, &payload)
    }
}

/// Return the IPI record kind of a test ID: 0x1000 is a function, 0x1001
/// is a build information record and 0x1002 is a string.
fn id_kind(id: IdIndex) -> Option<u16> {
    match id.0 {
        0x1000 => Some(LF_FUNC_ID),
        0x1001 => Some(LF_BUILDINFO),
        0x1002 => Some(LF_STRING_ID),
        _ => None,
    }
}

/// Return a context with TPI records up to 0x1010 and IPI records up to 0x1003.
fn context() -> ValidationContext<'static> {
    ValidationContext::new(TypeIndex(0x1010), IdIndex(0x1003)).with_id_kinds(&id_kind)
}

/// Validate a module stream with `context`.
fn module_with(builder: &Builder, context: &ValidationContext<'_>) -> ValidationReport {
    validate_module_symbols(&builder.bytes, STREAM, BASE, context).unwrap()
}

/// Validate a module stream with the default test context.
fn module(builder: &Builder) -> ValidationReport {
    module_with(builder, &context())
}

/// Return the broken rules and their record offsets.
fn rules(report: &ValidationReport) -> Vec<(Rule, u64)> {
    report
        .violations
        .iter()
        .map(|violation| (violation.rule, violation.offset))
        .collect()
}

/// Build a procedure at [`CODE`] with 0x100 bytes and an open end link.
fn procedure(builder: &mut Builder) -> u32 {
    builder.procedure(S_GPROC32_ID, 0x1000, CODE, 0x100)
}

#[test]
fn accepts_a_valid_module() {
    let mut builder = Builder::default();
    builder.compile(X64);
    builder.push(S_BUILDINFO, &0x1001_u32.to_le_bytes());
    let outer = procedure(&mut builder);
    builder.local(0x0074, PARAMETER);
    builder.register_range(RCX, CODE, 0x20, &[(2, 2), (4, 3), (10, 1)]);
    builder.push(S_SKIP, &[0; 8]);
    builder.frame_range(CODE + 0x20, 0x10);
    builder.local(0x1005, 0);
    builder.push(
        S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE,
        &16_i32.to_le_bytes(),
    );
    let mut file_static = 0x0074_u32.to_le_bytes().to_vec();
    file_static.extend_from_slice(&[0, 0, 0, 0, 0, 2]);
    file_static.extend_from_slice(b"s\0");
    builder.push(S_FILESTATIC, &file_static);
    builder.register_range(RCX, CODE + 0x30, 0x8, &[]);
    builder.local(0x0074, OPTIMIZED_OUT);
    let block = builder.block(outer, CODE + 0x40, 0x40);
    builder.local(0x0074, 0);
    builder.register_range(RCX, CODE + 0x38, 0x10, &[]);
    let site = builder.inline_site(block, 0x1000, &[]);
    builder.local(0x0074, PARAMETER);
    builder.close(S_INLINESITE_END, site);
    let empty = builder.block(block, CODE + 0x50, 0);
    builder.local(0x0074, 0);
    builder.frame_range(CODE + 0x50, 0);
    builder.close(S_END, empty);
    builder.close(S_END, block);
    builder.close(S_PROC_ID_END, outer);
    let thunk = builder.thunk(0);
    builder.close(S_END, thunk);
    let report = module(&builder);
    assert_eq!(rules(&report), []);
    assert!(report.is_valid());
    assert_eq!(report.records, 26);
    assert_eq!(report.stream, STREAM);
}

#[test]
fn accepts_an_empty_stream() {
    let report = module(&Builder::default());
    assert!(report.is_valid());
    assert_eq!(report.records, 0);
}

#[test]
fn reports_a_wrong_parent_as_a_walk_violation() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    let block = builder.block(outer + 4, CODE, 0x10);
    builder.close(S_END, block);
    builder.close(S_PROC_ID_END, outer);
    let report = module(&builder);
    assert_eq!(rules(&report), [(Rule::Walk, u64::from(block))]);
    let violation = report.violations[0];
    assert_eq!(violation.error_kind, ErrorKind::InvalidReference);
    assert_eq!(violation.record_kind, None);
    assert_eq!(report.records, 1);
}

#[test]
fn reports_an_end_link_to_the_wrong_record() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    let label = builder.push(S_LABEL32, &[0, 0, 0, 0, 1, 0, 0, b'L', 0]);
    builder.close(S_PROC_ID_END, outer);
    builder.patch(outer, END, label);
    let report = module(&builder);
    assert_eq!(rules(&report), [(Rule::Walk, u64::from(outer))]);
    assert_eq!(report.violations[0].error_kind, ErrorKind::InvalidReference);
}

#[test]
fn reports_end_kinds_that_do_not_match_the_procedure() {
    let mut builder = Builder::default();
    let typed = builder.procedure(S_GPROC32, 0x1005, CODE, 0x10);
    let typed_end = builder.close(S_PROC_ID_END, typed);
    let with_id = procedure(&mut builder);
    let id_end = builder.close(S_END, with_id);
    let report = module(&builder);
    assert_eq!(
        rules(&report),
        [
            (Rule::ProcedureIdEndKind, u64::from(typed_end)),
            (Rule::ProcedureIdEndMissing, u64::from(id_end)),
        ]
    );
}

#[test]
fn reports_a_block_at_the_top_level() {
    let mut builder = Builder::default();
    let block = builder.block(0, CODE, 0x10);
    builder.close(S_END, block);
    assert_eq!(
        rules(&module(&builder)),
        [(Rule::ScopeWithoutParent, u64::from(block))]
    );
}

#[test]
fn checks_next_links() {
    let mut builder = Builder::default();
    let first = builder.procedure(S_LPROC32, 0x1005, CODE, 0x10);
    builder.close(S_END, first);
    let second = builder.procedure(S_LPROC32, 0x1005, CODE + 0x10, 0x10);
    let label = builder.push(S_LABEL32, &[0, 0, 0, 0, 1, 0, 0, b'L', 0]);
    builder.close(S_END, second);
    let thunk = builder.thunk(first);
    builder.close(S_END, thunk);
    builder.patch(first, NEXT, second);
    builder.patch(second, NEXT, label);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::NextLink, u64::from(second)),
            (Rule::NextLink, u64::from(thunk)),
        ]
    );
}

#[test]
fn reports_a_block_outside_its_procedure() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    let block = builder.block(outer, CODE + 0xf0, 0x20);
    builder.close(S_END, block);
    builder.close(S_PROC_ID_END, outer);
    assert_eq!(
        rules(&module(&builder)),
        [(Rule::ScopeRange, u64::from(block))]
    );
}

#[test]
fn reports_def_ranges_outside_the_procedure_and_overflows() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    let before = builder.register_range(RCX, CODE - 1, 4, &[]);
    let after = builder.register_range(RCX, CODE + 0xff, 2, &[]);
    let wrapped = builder.register_range(RCX, u32::MAX - 1, 4, &[]);
    let inside = builder.block(outer, CODE + 0x40, 0x10);
    builder.local(0x0074, 0);
    // A def-range can start before its block; LLVM writes such ranges.
    builder.register_range(RCX, CODE + 0x30, 0x20, &[]);
    builder.close(S_END, inside);
    builder.close(S_PROC_ID_END, outer);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::DefRangeOutsideProcedure, u64::from(before)),
            (Rule::DefRangeOutsideProcedure, u64::from(after)),
            (Rule::CodeRangeOverflow, u64::from(wrapped)),
        ]
    );
}

#[test]
fn checks_def_range_gaps() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    let outside = builder.register_range(RCX, CODE, 0x10, &[(0xc, 5)]);
    let at_end = builder.register_range(RCX, CODE, 0x10, &[(0xc, 4)]);
    let overlap = builder.register_range(RCX, CODE, 0x10, &[(2, 4), (5, 1)]);
    let unsorted = builder.register_range(RCX, CODE, 0x10, &[(8, 1), (2, 1)]);
    let empty = builder.register_range(RCX, CODE, 0x10, &[(3, 0)]);
    builder.close(S_PROC_ID_END, outer);
    let report = module(&builder);
    assert_eq!(
        rules(&report),
        [
            (Rule::GapOutsideRange, u64::from(outside)),
            (Rule::GapOrder, u64::from(overlap)),
            (Rule::GapOrder, u64::from(unsorted)),
            (Rule::GapEmpty, u64::from(empty)),
        ]
    );
    assert!(
        report
            .violations
            .iter()
            .all(|v| v.offset != u64::from(at_end))
    );
}

#[test]
fn reports_def_ranges_without_a_local() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    builder.udt(0x0074);
    let orphan = builder.register_range(RCX, CODE, 0x10, &[]);
    let full_scope = builder.push(S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, &[0; 4]);
    builder.close(S_PROC_ID_END, outer);
    let first = builder.push(S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, &[0; 4]);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::DefRangeWithoutLocal, u64::from(orphan)),
            (Rule::DefRangeWithoutLocal, u64::from(full_scope)),
            (Rule::DefRangeWithoutLocal, u64::from(first)),
        ]
    );
}

#[test]
fn reports_an_empty_def_range_in_code() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    let empty = builder.frame_range(CODE, 0);
    let site = builder.inline_site(outer, 0x1000, &[0x04, 0x00]);
    builder.local(0x0074, 0);
    builder.frame_range(CODE, 0);
    builder.close(S_INLINESITE_END, site);
    let covered = builder.inline_site(outer, 0x1000, &[0x04, 0x02]);
    builder.local(0x0074, 0);
    let in_code = builder.frame_range(CODE, 0);
    builder.close(S_INLINESITE_END, covered);
    builder.close(S_PROC_ID_END, outer);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::DefRangeEmpty, u64::from(empty)),
            (Rule::DefRangeEmpty, u64::from(in_code)),
        ]
    );
}

#[test]
fn checks_type_indices() {
    let mut builder = Builder::default();
    builder.udt(0x0074);
    builder.udt(0x100f);
    let past_end = builder.udt(0x1010);
    let unknown = builder.udt(0x0f03);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::TypeIndexRange, u64::from(past_end)),
            (Rule::PrimitiveType, u64::from(unknown)),
        ]
    );
}

#[test]
fn checks_id_indices_and_kinds() {
    let mut builder = Builder::default();
    let wrong_build = builder.push(S_BUILDINFO, &0x1000_u32.to_le_bytes());
    let outside = builder.push(S_BUILDINFO, &0x1003_u32.to_le_bytes());
    let primitive = builder.push(S_BUILDINFO, &0x0074_u32.to_le_bytes());
    let wrong_function = builder.procedure(S_GPROC32_ID, 0x1002, CODE, 0x10);
    let site = builder.inline_site(wrong_function, 0x1001, &[]);
    builder.close(S_INLINESITE_END, site);
    builder.close(S_PROC_ID_END, wrong_function);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::IdKind, u64::from(wrong_build)),
            (Rule::IdIndexRange, u64::from(outside)),
            (Rule::IdIndexRange, u64::from(primitive)),
            (Rule::IdKind, u64::from(wrong_function)),
            (Rule::IdKind, u64::from(site)),
        ]
    );
}

#[test]
fn checks_id_ranges_without_kind_lookup() {
    let mut builder = Builder::default();
    builder.push(S_BUILDINFO, &0x1000_u32.to_le_bytes());
    let outside = builder.push(S_BUILDINFO, &0x1003_u32.to_le_bytes());
    let context = ValidationContext::new(TypeIndex(0x1010), IdIndex(0x1003));
    assert_eq!(
        rules(&module_with(&builder, &context)),
        [(Rule::IdIndexRange, u64::from(outside))]
    );
}

#[test]
fn checks_registers_for_the_module_cpu() {
    let mut builder = Builder::default();
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    builder.register_range(31, CODE, 4, &[]);
    builder.close(S_PROC_ID_END, outer);
    assert!(module(&builder).is_valid(), "no compile record: no check");

    let mut builder = Builder::default();
    builder.compile(X64);
    builder.compile(0x60);
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    let unknown = builder.register_range(31, CODE, 4, &[]);
    builder.register_range(30_000, CODE, 4, &[]);
    builder.close(S_PROC_ID_END, outer);
    assert_eq!(
        rules(&module(&builder)),
        [(Rule::RegisterUnknown, u64::from(unknown))]
    );

    let mut builder = Builder::default();
    builder.compile(0xff);
    let outer = procedure(&mut builder);
    builder.local(0x0074, 0);
    builder.register_range(31, CODE, 4, &[]);
    builder.close(S_PROC_ID_END, outer);
    assert!(module(&builder).is_valid(), "unknown CPU: no check");
}

#[test]
fn checks_local_flags_and_scopes() {
    let mut builder = Builder::default();
    let top = builder.local(0x0074, 0);
    let outer = procedure(&mut builder);
    let reserved = builder.local(0x0074, 1 << 11);
    let optimized = builder.local(0x0074, OPTIMIZED_OUT);
    let range = builder.register_range(RCX, CODE, 4, &[]);
    let block = builder.block(outer, CODE, 0x10);
    let parameter = builder.local(0x0074, PARAMETER);
    builder.close(S_END, block);
    builder.close(S_PROC_ID_END, outer);
    assert_eq!(
        rules(&module(&builder)),
        [
            (Rule::LocalOutsideScope, u64::from(top)),
            (Rule::LocalFlagsReserved, u64::from(reserved)),
            (Rule::OptimizedOutWithRange, u64::from(range)),
            (Rule::ParameterScope, u64::from(parameter)),
        ]
    );
    assert!(optimized < range);
}

#[test]
fn checks_code_ranges_against_sections() {
    let mut builder = Builder::default();
    let fits = builder.procedure(S_GPROC32, 0x0003, 0, 0x100);
    builder.close(S_END, fits);
    let past_end = builder.procedure(S_GPROC32, 0x0003, 0x80, 0x81);
    builder.local(0x0074, 0);
    builder.register_range(RCX, 0x80, 4, &[]);
    builder.close(S_END, past_end);
    let lengths = [0x100];
    let context = context().with_section_lengths(&lengths);
    assert_eq!(
        rules(&module_with(&builder, &context)),
        [(Rule::SectionRange, u64::from(past_end))]
    );
    let none: [u32; 0] = [];
    let context = self::context().with_section_lengths(&none);
    let report = module_with(&builder, &context);
    assert_eq!(report.violations.len(), 3, "section 1 is missing");
}

#[test]
fn checks_module_references() {
    let mut builder = Builder::default();
    builder.reference(S_PROCREF, 1, 4);
    builder.reference(S_PROCREF, 1, 40);
    builder.reference(S_LPROCREF, 2, 8);
    let module_zero = builder.reference(S_PROCREF, 0, 4);
    let past_last = builder.reference(S_PROCREF, 3, 4);
    let missing = builder.reference(S_PROCREF, 1, 12);
    let wrong_kind = builder.reference(S_LPROCREF, 1, 4);
    builder.reference(S_PROCREF, 2, 4);
    let unknown = builder.reference(S_PROCREF, 2, 100);
    let records = |module: u16, offset: u32| match (module, offset) {
        (0, 4) => ModuleRecord::Kind(S_GPROC32),
        (0, 40) => ModuleRecord::Kind(S_GMANPROC),
        (1, 8) => ModuleRecord::Kind(S_LPROC32),
        (1, 4) => ModuleRecord::Kind(S_GPROC32_ID),
        (1, _) => ModuleRecord::Unknown,
        _ => ModuleRecord::Missing,
    };
    let context = context().with_module_count(2).with_module_records(&records);
    let report = validate_shared_symbols(&builder.bytes, StreamId(7), &context).unwrap();
    // The shared stream has no signature, so offsets start at 0.
    let base = u64::from(BASE);
    assert_eq!(
        rules(&report),
        [
            (Rule::ModuleIndex, u64::from(module_zero) - base),
            (Rule::ModuleIndex, u64::from(past_last) - base),
            (Rule::ModuleRecordMissing, u64::from(missing) - base),
            (Rule::ModuleRecordKind, u64::from(wrong_kind) - base),
        ]
    );
    assert_eq!(report.records, 9);
    assert_eq!(report.stream, StreamId(7));
    assert!(unknown > wrong_kind);
}

#[test]
fn checks_shared_type_and_id_references() {
    let mut builder = Builder::default();
    let past_end = builder.udt(0x2000);
    let wrong = builder.push(S_BUILDINFO, &0x1002_u32.to_le_bytes());
    let report = validate_shared_symbols(&builder.bytes, StreamId(7), &context()).unwrap();
    let base = u64::from(BASE);
    assert_eq!(
        rules(&report),
        [
            (Rule::TypeIndexRange, u64::from(past_end) - base),
            (Rule::IdKind, u64::from(wrong) - base),
        ]
    );
}

#[test]
fn gives_violations_as_errors_with_locations() {
    let mut builder = Builder::default();
    let past_end = builder.udt(0x1010);
    let report = module(&builder);
    let violation = report.violations[0];
    assert_eq!(violation.record_kind, Some(S_UDT));
    assert_eq!(violation.context, Rule::TypeIndexRange.description());
    let error = violation.error();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(error.context(), Some(Rule::TypeIndexRange.description()));
    assert_eq!(
        error.stream_location(),
        Some((STREAM.0, u64::from(past_end)))
    );
}

#[test]
fn reports_malformed_records_without_panic() {
    let mut builder = Builder::default();
    builder.udt(0x0074);
    let bad = builder.push(S_LOCAL, &[1]);
    builder.udt(0x0074);
    let report = module(&builder);
    assert_eq!(rules(&report), [(Rule::Walk, u64::from(bad))]);
    assert_eq!(report.records, 1);
    let report = validate_shared_symbols(&builder.bytes, StreamId(7), &context()).unwrap();
    assert_eq!(rules(&report), [(Rule::Walk, u64::from(bad - BASE))]);

    let truncated = &builder.bytes[..6];
    let report = validate_shared_symbols(truncated, StreamId(7), &context()).unwrap();
    assert_eq!(rules(&report), [(Rule::Walk, 0)]);
}

#[test]
fn does_not_panic_on_corrupt_bytes() {
    let mut builder = Builder::default();
    builder.compile(X64);
    let outer = procedure(&mut builder);
    builder.local(0x0074, PARAMETER);
    builder.register_range(RCX, CODE, 0x20, &[(2, 2)]);
    let block = builder.block(outer, CODE, 0x10);
    builder.close(S_END, block);
    builder.close(S_PROC_ID_END, outer);
    builder.reference(S_PROCREF, 1, 4);
    let records = |_: u16, _: u32| ModuleRecord::Missing;
    let lengths = [0x10];
    let context = context()
        .with_module_count(1)
        .with_module_records(&records)
        .with_section_lengths(&lengths)
        .with_max_depth(1);
    for index in 0..builder.bytes.len() {
        for value in [0x00, 0x01, 0x7f, 0xff] {
            let mut bytes = builder.bytes.clone();
            bytes[index] = value;
            // Only the absence of a panic matters for the module walk.
            let _ = validate_module_symbols(&bytes, STREAM, BASE, &context);
            let shared = validate_shared_symbols(&bytes, STREAM, &context);
            assert!(shared.is_ok());
            let _ = validate_module_symbols(&bytes[..index], STREAM, BASE, &context);
        }
    }
}
