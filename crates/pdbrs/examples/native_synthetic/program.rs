//! The records of a small synthetic x64 program, built through the public API.
//!
//! The program has this source, `C:\synthetic\synthetic.c`:
//!
//! ```c
//! struct Point { int x; int y; };          // line 1
//! int g_value;                             // line 2
//! int sum(struct Point *p) {               // line 3
//!     return p->x + p->y;                  // line 4
//! }                                        // line 5
//!                                          // line 6
//! int main(void) {                         // line 7
//!     struct Point p = { 1, 2 };           // line 8
//!     g_value = sum(&p);                   // line 9
//!     return 0;                            // line 10
//! }                                        // line 11
//! ```
//!
//! The image has `.text` at RVA `0x1000` and `.data` at RVA `0x2000`.
//! `sum` is at `.text+0x0` (0x20 bytes), `main` at `.text+0x20` (0x30 bytes)
//! and `g_value` at `.data+0x0`.

use pdbrs::Result;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::field_list::{DataMember, Member, encode_field_list};
use pdbrs::native::codeview::id_record::{FunctionId, IdRecord};
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    Compile3, CompileFlags, Data, FrameProcedure, FrameProcedureFlags, ObjectName, Procedure,
    ProcedureFlags, ProcedureReference, ProcedureType, Public, PublicFlags, RegisterRelative,
    S_COMPILE3, S_END, S_FRAMEPROC, S_GDATA32, S_GPROC32, S_OBJNAME, S_PROCREF, S_PUB32,
    S_REGREL32, S_UDT, SymbolData, UserDefinedType,
};
use pdbrs::native::codeview::type_record::{
    ArgumentList, CallingConvention, Class, ClassKind, ClassOptions, FunctionOptions,
    MemberAttributes, Pointer, PointerAttributes, Procedure as ProcedureLeaf, TypeLeaf,
};
use pdbrs::native::dbi::{
    DbiHeader, DebugStream, OwnedModule, OwnedSectionMap, SectionContribution,
    SectionContributionVersion, SectionMapEntry, SegmentFlags,
};
use pdbrs::native::debug_stream::RecordStream;
use pdbrs::native::lines::{
    ChecksumKind, DEBUG_S_FILECHKSMS, DEBUG_S_LINES, LineEntry, LineSubsection, LinesHeader,
    encode_file_checksum, encode_line_block,
};
use pdbrs::native::section::SectionHeader;
use pdbrs::native::write::{ModuleSymbolHandle, PdbBuilder, TypeHandle};

/// The identity of the synthetic PDB.
pub const GUID: [u8; 16] = *b"pdbrs-synthetic!";
/// The signature of the synthetic PDB.
pub const SIGNATURE: u32 = 0x6533_3300;
/// The x64 machine value.
const MACHINE_AMD64: u16 = 0x8664;
/// The `CV_CFL_AMD64` compile machine value.
const CV_CFL_AMD64: u16 = 0xd0;
/// The x64 register number of `RSP` (`CV_AMD64_RSP`).
const CV_AMD64_RSP: u16 = 335;
/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);
/// The `.text` section characteristics: code, execute, read.
const TEXT_CHARACTERISTICS: u32 = 0x6000_0020;
/// The `.data` section characteristics: initialized data, read, write.
const DATA_CHARACTERISTICS: u32 = 0xc000_0040;
/// The source file name.
pub const SOURCE: &[u8] = b"C:\\synthetic\\synthetic.c";

/// A procedure of the program.
struct Function {
    /// The name.
    name: &'static [u8],
    /// The offset in `.text`.
    offset: u32,
    /// The code size.
    size: u32,
    /// The line entries: code offset and line.
    lines: &'static [(u32, u32)],
}

/// `sum`.
const SUM: Function = Function {
    name: b"sum",
    offset: 0,
    size: 0x20,
    lines: &[(0, 3), (0x8, 4), (0x1a, 5)],
};

/// `main`.
const MAIN: Function = Function {
    name: b"main",
    offset: 0x20,
    size: 0x30,
    lines: &[(0, 7), (0x8, 8), (0x14, 9), (0x24, 10), (0x2c, 11)],
};

/// Return an address in a section.
const fn at(section: u16, offset: u32) -> SectionOffset {
    SectionOffset { section, offset }
}

/// Return a numeric leaf with an unsigned value.
fn number(value: u32) -> Numeric<'static> {
    Numeric::from_integer(Integer::Unsigned(u128::from(value)))
}

/// Build the synthetic PDB.
///
/// # Errors
///
/// Returns the errors of the builder.
pub fn build() -> Result<PdbBuilder<'static>> {
    let mut builder = PdbBuilder::new(SIGNATURE, GUID, 1)?;
    // The handle of the function ID of `sum` exists before its record and
    // before its type: the module records can refer to it at once.
    let sum_id = builder.reserve_id()?;
    let point_forward = builder.add_type(TypeLeaf::Class(Class {
        kind: ClassKind::Structure,
        member_count: 0,
        options: ClassOptions(ClassOptions::FORWARD_REFERENCE),
        field_list: TypeIndex::NONE,
        derivation_list: TypeIndex::NONE,
        vtable_shape: TypeIndex::NONE,
        size: number(0),
        name: b"Point",
        unique_name: None,
        producer_padding: &[],
    }))?;
    let pointer = builder.add_type_with(move |resolver, out| {
        TypeLeaf::Pointer(Pointer {
            referent_type: resolver.type_index(point_forward)?,
            // A 64-bit pointer (kind 0x0c) of 8 bytes.
            attributes: PointerAttributes::from_raw(0x0c | (8 << 13))?,
            member: None,
            based: None,
        })
        .encode(out)
    })?;
    let sum_arguments = builder.add_type_with(move |resolver, out| {
        let index = resolver.type_index(pointer)?.0.to_le_bytes();
        TypeLeaf::ArgumentList(ArgumentList::from_bytes(&index)?).encode(out)
    })?;
    let sum_type = procedure_type(&mut builder, sum_arguments, 1)?;
    let fields = builder.add_type_with(|_, out| {
        let public = MemberAttributes::from_raw(3)?;
        let members = [
            Member::DataMember(DataMember {
                attributes: public,
                member_type: T_INT4,
                offset: number(0),
                name: b"x",
            }),
            Member::DataMember(DataMember {
                attributes: public,
                member_type: T_INT4,
                offset: number(4),
                name: b"y",
            }),
        ];
        encode_field_list(&members, TypeIndex(0x1000), 0xff00, out).map(|_| ())
    })?;
    // The definition comes after its field list. The pointer refers to the
    // forward reference, as compilers write it.
    let point = builder.add_type_with(move |resolver, out| {
        TypeLeaf::Class(Class {
            kind: ClassKind::Structure,
            member_count: 2,
            options: ClassOptions(0),
            field_list: resolver.type_index(fields)?,
            derivation_list: TypeIndex::NONE,
            vtable_shape: TypeIndex::NONE,
            size: number(8),
            name: b"Point",
            unique_name: None,
            producer_padding: &[],
        })
        .encode(out)
    })?;
    let main_arguments =
        builder.add_type(TypeLeaf::ArgumentList(ArgumentList::from_bytes(&[])?))?;
    let main_type = procedure_type(&mut builder, main_arguments, 0)?;
    builder.define_id_with(sum_id, move |resolver, out| {
        function_id(resolver.type_index(sum_type)?, SUM.name).encode(out)
    })?;
    builder.add_id_with(move |resolver, out| {
        function_id(resolver.type_index(main_type)?, MAIN.name).encode(out)
    })?;
    let file = builder.add_name(SOURCE)?;
    // As `link.exe` writes them, the procedure records name their type, not their ID.
    let procedures = add_module(
        &mut builder,
        file,
        [(sum_type, pointer), (main_type, point)],
    )?;
    add_globals(&mut builder, procedures, point)?;
    add_image_tables(&mut builder)?;
    Ok(builder)
}

/// Add an `LF_PROCEDURE` record that returns `int`.
fn procedure_type(
    builder: &mut PdbBuilder<'_>,
    arguments: TypeHandle,
    parameter_count: u16,
) -> Result<TypeHandle> {
    builder.add_type_with(move |resolver, out| {
        TypeLeaf::Procedure(ProcedureLeaf {
            return_type: T_INT4,
            calling_convention: CallingConvention::NearC,
            options: FunctionOptions(0),
            parameter_count,
            argument_list: resolver.type_index(arguments)?,
        })
        .encode(out)
    })
}

/// Return an `LF_FUNC_ID` record at the global scope.
fn function_id(function_type: TypeIndex, name: &[u8]) -> IdRecord<'_> {
    IdRecord::Function(FunctionId {
        scope: None,
        function_type,
        name,
        decorated_name_hash: None,
    })
}

/// Add the module with its symbols and lines. Return the procedure handles.
fn add_module(
    builder: &mut PdbBuilder<'_>,
    file: u32,
    functions: [(TypeHandle, TypeHandle); 2],
) -> Result<[ModuleSymbolHandle; 2]> {
    let module = builder.add_module(OwnedModule {
        unused1: 0,
        section_contribution: contribution(1, 0, 0x50, TEXT_CHARACTERISTICS),
        flags: 0,
        stream: None,
        symbol_bytes: 0,
        c11_bytes: 0,
        c13_bytes: 0,
        source_file_count: 0,
        padding: 0,
        file_name_offset: 0,
        source_file_name_index: 0,
        pdb_file_path_index: 0,
        name: b"C:\\synthetic\\synthetic.obj".to_vec(),
        object_file: b"C:\\synthetic\\synthetic.obj".to_vec(),
        name_padding: [0; 3],
    })?;
    module.push_file(SOURCE)?;
    module.push_symbol_fields(
        S_OBJNAME,
        SymbolData::ObjectName(ObjectName {
            signature: 0,
            name: b"C:\\synthetic\\synthetic.obj",
        }),
    )?;
    module.push_symbol_fields(
        S_COMPILE3,
        SymbolData::Compile3(Compile3 {
            flags: CompileFlags(0),
            machine: CV_CFL_AMD64,
            frontend: [19, 51, 0, 0],
            backend: [19, 51, 0, 0],
            version: b"pdbrs synthetic",
            trailing_zeros: 0,
        }),
    )?;
    let mut handles = [None, None];
    for ((function, (procedure_type, local_type)), local) in [SUM, MAIN]
        .iter()
        .zip(functions)
        .zip([b"p".as_slice(), b"p"])
    {
        let procedure = module.push_symbol_with(move |resolver, out| {
            SymbolData::Procedure(Procedure {
                parent: 0,
                end: 0,
                next: 0,
                code_length: function.size,
                debug_start: 0,
                debug_end: function.size - 1,
                signature: ProcedureType::Type(resolver.type_index(procedure_type)?),
                address: at(1, function.offset),
                flags: ProcedureFlags(0),
                extended_flags: None,
                name: function.name,
            })
            .encode(S_GPROC32, out)
        })?;
        module.push_symbol_fields(
            S_FRAMEPROC,
            SymbolData::FrameProcedure(FrameProcedure {
                frame_size: 0x28,
                padding_size: 0,
                padding_offset: 0,
                callee_saved_size: 0,
                exception_handler: at(0, 0),
                // The local and parameter base registers are RSP.
                flags: FrameProcedureFlags((1 << 14) | (1 << 16)),
            }),
        )?;
        module.push_symbol_with(move |resolver, out| {
            SymbolData::RegisterRelative(RegisterRelative {
                offset: 0x30,
                type_index: resolver.type_index(local_type)?,
                register: CV_AMD64_RSP,
                name: local,
            })
            .encode(S_REGREL32, out)
        })?;
        module.push_symbol_fields(S_END, SymbolData::ScopeEnd)?;
        let slot = usize::from(function.offset != 0);
        handles[slot] = Some(procedure);
    }
    module.push_subsection_with(move |_, out| {
        let mut data = Vec::new();
        encode_file_checksum(file, ChecksumKind::None, &[], &mut data)?;
        frame(DEBUG_S_FILECHKSMS, &data, out)
    })?;
    for function in [SUM, MAIN] {
        module.push_subsection_with(move |_, out| {
            let mut data = Vec::new();
            LinesHeader::new(function.offset, 1, 0, function.size).encode(&mut data)?;
            let lines: Vec<LineEntry> = function
                .lines
                .iter()
                .map(|&(offset, line)| LineEntry::new(offset, line, 0, true))
                .collect();
            // The file checksum entry of the source is at offset 0.
            encode_line_block(0, &lines, None, &mut data)?;
            frame(DEBUG_S_LINES, &data, out)
        })?;
    }
    let [Some(sum), Some(main)] = handles else {
        unreachable!("both procedures have a handle")
    };
    Ok([sum, main])
}

/// Append a C13 subsection of `kind` with `data`, its header and its padding.
///
/// The subsection codec decodes the data and frames it.
fn frame(kind: u32, data: &[u8], out: &mut Vec<u8>) -> Result<()> {
    LineSubsection::decode(kind, data)?.encode_framed(out)
}

/// Add the symbol record stream with its global and public index entries.
fn add_globals(
    builder: &mut PdbBuilder<'_>,
    procedures: [ModuleSymbolHandle; 2],
    point: TypeHandle,
) -> Result<()> {
    for (procedure, name) in procedures.into_iter().zip([SUM.name, MAIN.name]) {
        let reference = builder.add_symbol_with(move |resolver, out| {
            SymbolData::ProcedureReference(ProcedureReference {
                name_checksum: 0,
                offset: resolver.module_symbol_offset(procedure)?,
                module: resolver.module_number(procedure.module())?,
                name,
            })
            .encode(S_PROCREF, out)
        })?;
        builder.index_global(reference, 1)?;
    }
    let data = builder.add_symbol_fields(
        S_GDATA32,
        SymbolData::Data(Data {
            type_index: T_INT4,
            address: at(2, 0),
            name: b"g_value",
        }),
    )?;
    builder.index_global(data, 1)?;
    let udt = builder.add_symbol_with(move |resolver, out| {
        SymbolData::UserDefinedType(UserDefinedType {
            type_index: resolver.type_index(point)?,
            name: b"Point",
        })
        .encode(S_UDT, out)
    })?;
    builder.index_global(udt, 1)?;
    let function = PublicFlags(PublicFlags::FUNCTION.0 | PublicFlags::CODE.0);
    for (name, flags, address) in [
        (SUM.name, function, at(1, SUM.offset)),
        (MAIN.name, function, at(1, MAIN.offset)),
        (b"g_value".as_slice(), PublicFlags(0), at(2, 0)),
    ] {
        let public = builder.add_symbol_fields(
            S_PUB32,
            SymbolData::Public(Public {
                flags,
                address,
                name,
            }),
        )?;
        builder.index_public(public)?;
    }
    Ok(())
}

/// Return a V60 section contribution of module 0.
const fn contribution(
    section: u16,
    offset: i32,
    size: i32,
    characteristics: u32,
) -> SectionContribution {
    SectionContribution {
        section,
        padding1: 0,
        offset,
        size,
        characteristics,
        module: 0,
        padding2: 0,
        data_crc: Some(0),
        reloc_crc: Some(0),
        coff_section: None,
    }
}

/// Add the DBI header, the section contributions, the section map and the
/// section headers.
fn add_image_tables(builder: &mut PdbBuilder<'_>) -> Result<()> {
    builder.set_dbi_header(DbiHeader {
        // The new version format (bit 15), major version 14 and minor version 51 (0x33).
        build_number: 0x8000 | (14 << 8) | 0x33,
        machine: MACHINE_AMD64,
        ..DbiHeader::default()
    });
    builder.set_section_contributions(
        SectionContributionVersion::V60,
        vec![
            contribution(1, 0, 0x50, TEXT_CHARACTERISTICS),
            contribution(2, 0, 4, DATA_CHARACTERISTICS),
        ],
    );
    let segment = |flags: u16, frame: u16, length: u32| SectionMapEntry {
        flags: SegmentFlags(flags),
        overlay: 0,
        group: 0,
        frame,
        section_name: 0xffff,
        class_name: 0xffff,
        offset: 0,
        length,
    };
    let selector = SegmentFlags::IS_SELECTOR.0 | SegmentFlags::ADDRESS_IS_32_BIT.0;
    builder.set_section_map(Some(OwnedSectionMap {
        logical_count: 3,
        entries: vec![
            segment(
                selector | SegmentFlags::READ.0 | SegmentFlags::EXECUTE.0,
                1,
                0x100,
            ),
            segment(
                selector | SegmentFlags::READ.0 | SegmentFlags::WRITE.0,
                2,
                0x100,
            ),
            segment(
                SegmentFlags::IS_ABSOLUTE_ADDRESS.0 | SegmentFlags::ADDRESS_IS_32_BIT.0,
                0,
                u32::MAX,
            ),
        ],
    }));
    let mut text = SectionHeader::new(*b".text\0\0\0", 0x100, 0x1000);
    text.characteristics = TEXT_CHARACTERISTICS;
    let mut data = SectionHeader::new(*b".data\0\0\0", 0x100, 0x2000);
    data.characteristics = DATA_CHARACTERISTICS;
    builder.set_debug_slot_count(11);
    builder.set_debug_stream(
        DebugStream::SectionHeader,
        RecordStream::from_records(&[text, data])?.to_bytes()?,
    )
}
