//! The records of a small synthetic x64 program, built with the public editor API.
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

use pdbrs::native::PdbEditor;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::field_list::{DataMember, Member, encode_field_list};
use pdbrs::native::codeview::id_record::{FunctionId, IdRecord};
use pdbrs::native::codeview::index::{
    ChecksumOffset, ModuleIndex, ModuleSymbolHandle, ModuleSymbolOffset, NameOffset, TypeIndex,
};
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    Compile3, CompileFlags, CpuType, Data, FrameProcedure, FrameProcedureFlags, ModuleSymbolTarget,
    ObjectName, Procedure, ProcedureFlags, ProcedureReference, ProcedureType, Public, PublicFlags,
    RegisterRelative, S_COMPILE3, S_END, S_FRAMEPROC, S_GDATA32, S_GPROC32, S_OBJNAME, S_PROCREF,
    S_REGREL32, S_UDT, SymbolData, UserDefinedType,
};
use pdbrs::native::codeview::type_record::{
    ArgumentList, CallingConvention, Class, ClassKind, ClassOptions, FunctionOptions,
    MemberAttributes, Pointer, PointerAttributes, Procedure as ProcedureLeaf, TypeLeaf,
};
use pdbrs::native::dbi::{
    DbiHeader, DebugStream, Module, SectionContribution, SectionContributionVersion, SectionMap,
    SectionMapEntry, SegmentFlags,
};
use pdbrs::native::debug_stream::RecordStream;
use pdbrs::native::editor::ModuleContents;
use pdbrs::native::lines::{
    ChecksumKind, DEBUG_S_FILECHKSMS, DEBUG_S_LINES, FileChecksum, LineBlock, LineEntry,
    LineSubsection, LinesHeader, SubsectionKind,
};
use pdbrs::native::section::{Machine, SectionHeader};
use pdbrs::{Error, ErrorKind, Result};

/// The identity of the synthetic PDB.
pub const GUID: [u8; 16] = *b"pdbrs-synthetic!";
/// The signature of the synthetic PDB.
pub const SIGNATURE: u32 = 0x6533_3300;
/// The `CV_CFL_AMD64` compile machine value.
const CV_CFL_AMD64: CpuType = CpuType::X64;
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

/// Return the `Point` structure record with `options`, `field_list`, a member count and a size.
fn point_class(
    options: ClassOptions,
    field_list: TypeIndex,
    member_count: u16,
    size: u32,
) -> Class<'static> {
    let mut class = Class::new(ClassKind::Structure);
    class.member_count = member_count;
    class.options = options;
    class.field_list = field_list;
    class.derivation_list = TypeIndex::NONE;
    class.vtable_shape = TypeIndex::NONE;
    class.size = number(size);
    class.name = b"Point".into();
    class.unique_name = None;
    class
}

/// Return an `int` data member of `Point`.
fn int_member(attributes: MemberAttributes, offset: u32, name: &'static [u8]) -> Member<'static> {
    let mut member = DataMember::default();
    member.attributes = attributes;
    member.member_type = T_INT4;
    member.offset = number(offset);
    member.name = name.into();
    Member::DataMember(member)
}

/// Build the synthetic PDB.
///
/// The type and ID records go in reference order: a record refers only to
/// earlier records. The indices are the indices of the old builder program.
///
/// # Errors
///
/// Returns the errors of the editor.
pub fn build() -> Result<PdbEditor<'static>> {
    let mut editor = PdbEditor::new(SIGNATURE, GUID, 1)?;
    let types = editor.types_mut()?;
    // 0x1000.
    let point_forward = types.push(TypeLeaf::Class(point_class(
        ClassOptions::FORWARD_REFERENCE,
        TypeIndex::NONE,
        0,
        0,
    )))?;
    let mut pointer = Pointer::default();
    pointer.referent_type = point_forward;
    // A 64-bit pointer (kind 0x0c) of 8 bytes.
    pointer.attributes = PointerAttributes::from_raw(0x0c | (8 << 13))?;
    pointer.member = None;
    pointer.based = None;
    pointer.trailing_zeros = 0;
    // 0x1001.
    let pointer = types.push(TypeLeaf::Pointer(pointer))?;
    // 0x1002.
    let sum_arguments = types.push(TypeLeaf::ArgumentList(ArgumentList::parse(
        &pointer.0.to_le_bytes(),
    )?))?;
    // 0x1003.
    let sum_type = types.push(procedure_type(sum_arguments, 1))?;
    let public = MemberAttributes::from_raw(3)?;
    let members = [int_member(public, 0, b"x"), int_member(public, 4, b"y")];
    let mut field_list = Vec::new();
    encode_field_list(&members, types.end()?, 0xff00, &mut field_list)?;
    let field_list = Records::new(&field_list).next().ok_or_else(|| {
        Error::new(ErrorKind::InvalidFormat, None).with_context("field list has no record")
    })??;
    // 0x1004.
    let fields = types.push(TypeLeaf::decode(field_list)?)?;
    // The definition comes after its field list. The pointer refers to the
    // forward reference, as compilers write it. 0x1005.
    let point = types.push(TypeLeaf::Class(point_class(ClassOptions(0), fields, 2, 8)))?;
    // 0x1006.
    let main_arguments = types.push(TypeLeaf::ArgumentList(ArgumentList::parse(&[])?))?;
    // 0x1007.
    let main_type = types.push(procedure_type(main_arguments, 0))?;
    let ids = editor.ids_mut()?;
    // 0x1000 and 0x1001.
    ids.push(function_id(sum_type, SUM.name))?;
    ids.push(function_id(main_type, MAIN.name))?;
    let file = editor.names_mut()?.insert(SOURCE)?;
    // As `link.exe` writes them, the procedure records name their type, not their ID.
    let procedures = add_module(&mut editor, file, [(sum_type, pointer), (main_type, point)])?;
    add_globals(&mut editor, procedures, point)?;
    add_image_tables(&mut editor)?;
    Ok(editor)
}

/// Return an `LF_PROCEDURE` record that returns `int`.
fn procedure_type(arguments: TypeIndex, parameter_count: u16) -> TypeLeaf<'static> {
    let mut procedure = ProcedureLeaf::new(CallingConvention::NearC);
    procedure.return_type = T_INT4;
    procedure.options = FunctionOptions(0);
    procedure.parameter_count = parameter_count;
    procedure.argument_list = arguments;
    TypeLeaf::Procedure(procedure)
}

/// Return an `LF_FUNC_ID` record at the global scope.
fn function_id(function_type: TypeIndex, name: &[u8]) -> IdRecord<'_> {
    let mut function = FunctionId::default();
    function.scope = None;
    function.function_type = function_type;
    function.name = name.into();
    function.decorated_name_hash = None;
    IdRecord::Function(function)
}

/// Add the module with its symbols and lines. Return the procedure handles.
fn add_module(
    editor: &mut PdbEditor<'_>,
    file: NameOffset,
    functions: [(TypeIndex, TypeIndex); 2],
) -> Result<[ModuleSymbolHandle; 2]> {
    let mut record = Module::default();
    record.section_contribution = contribution(1, 0, 0x50, TEXT_CHARACTERISTICS);
    record.name = b"C:\\synthetic\\synthetic.obj".into();
    record.object_file = b"C:\\synthetic\\synthetic.obj".into();
    let index = editor.add_module(record)?;
    let module = editor.module_mut(index)?;
    module.files_mut().push(SOURCE.into());
    let mut object = ObjectName::default();
    object.signature = 0;
    object.name = b"C:\\synthetic\\synthetic.obj".into();
    module.push(S_OBJNAME, SymbolData::ObjectName(object))?;
    let mut compile = Compile3::default();
    compile.flags = CompileFlags(0);
    compile.machine = CV_CFL_AMD64;
    compile.frontend = [19, 51, 0, 0];
    compile.backend = [19, 51, 0, 0];
    compile.version = b"pdbrs synthetic".into();
    compile.trailing_zeros = 0;
    module.push(S_COMPILE3, SymbolData::Compile3(compile))?;
    let [(sum_type, sum_local), (main_type, main_local)] = functions;
    let sum = push_procedure(module, &SUM, sum_type, sum_local)?;
    let main = push_procedure(module, &MAIN, main_type, main_local)?;
    let mut data = Vec::new();
    FileChecksum::encode_parts(file, ChecksumKind::None, &[], &mut data)?;
    push_subsection(module, DEBUG_S_FILECHKSMS, &data)?;
    for function in [SUM, MAIN] {
        let mut data = Vec::new();
        let mut header = LinesHeader::default();
        header.code_offset = function.offset;
        header.segment = 1;
        header.flags = 0;
        header.code_size = function.size;
        header.encode(&mut data)?;
        let lines: Vec<LineEntry> = function
            .lines
            .iter()
            .map(|&(offset, line)| {
                let mut entry = LineEntry::default();
                entry.offset = offset;
                entry.line_start = line;
                entry.end_delta = 0;
                entry.is_statement = true;
                entry
            })
            .collect();
        // The file checksum entry of the source is at offset 0.
        LineBlock::encode_parts(ChecksumOffset(0), &lines, None, &mut data)?;
        push_subsection(module, DEBUG_S_LINES, &data)?;
    }
    Ok([sum, main])
}

/// Add the scope of `function` to `module`: the procedure record of
/// `procedure_type`, the frame record, the local `p` of `local_type` and the
/// scope end. Return the handle of the procedure record.
fn push_procedure(
    module: &mut ModuleContents,
    function: &Function,
    procedure_type: TypeIndex,
    local_type: TypeIndex,
) -> Result<ModuleSymbolHandle> {
    let mut procedure = Procedure::new(ProcedureType::Type(procedure_type));
    procedure.next = ModuleSymbolOffset(0);
    procedure.code_length = function.size;
    procedure.debug_start = 0;
    procedure.debug_end = function.size - 1;
    procedure.address = at(1, function.offset);
    procedure.flags = ProcedureFlags(0);
    procedure.extended_flags = None;
    procedure.name = function.name.into();
    // The plan computes the scope links from the tree.
    let handle = module.push(S_GPROC32, SymbolData::Procedure(procedure))?;
    let mut frame = FrameProcedure::default();
    frame.frame_size = 0x28;
    frame.padding_size = 0;
    frame.padding_offset = 0;
    frame.callee_saved_size = 0;
    frame.exception_handler = at(0, 0);
    // The local and parameter base registers are RSP.
    frame.flags = FrameProcedureFlags((1 << 14) | (1 << 16));
    module.push(S_FRAMEPROC, SymbolData::FrameProcedure(frame))?;
    let mut relative = RegisterRelative::default();
    relative.offset = 0x30;
    relative.type_index = local_type;
    relative.register = CV_AMD64_RSP;
    relative.name = b"p".into();
    module.push(S_REGREL32, SymbolData::RegisterRelative(relative))?;
    module.push(S_END, SymbolData::ScopeEnd)?;
    Ok(handle)
}

/// Add a C13 subsection of `kind` with `data` to `module`.
///
/// The subsection codec decodes the data. The plan frames it.
fn push_subsection(module: &mut ModuleContents, kind: SubsectionKind, data: &[u8]) -> Result<()> {
    let subsection = LineSubsection::decode(kind, data)?.into_owned();
    let lines = module.lines_mut();
    lines.try_reserve(1).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None).with_context("no memory for a subsection")
    })?;
    lines.push(subsection);
    Ok(())
}

/// Add the symbol record stream with its global and public index entries.
fn add_globals(
    editor: &mut PdbEditor<'_>,
    procedures: [ModuleSymbolHandle; 2],
    point: TypeIndex,
) -> Result<()> {
    let mut globals = editor.globals_mut()?;
    for (procedure, name) in procedures.into_iter().zip([SUM.name, MAIN.name]) {
        let mut reference = ProcedureReference::default();
        reference.name_checksum = 0;
        // The plan writes the module index and the offset of the procedure.
        reference.target = ModuleSymbolTarget::Handle(procedure);
        reference.name = name.into();
        globals.push(S_PROCREF, SymbolData::ProcedureReference(reference), 1)?;
    }
    let mut global = Data::default();
    global.type_index = T_INT4;
    global.address = at(2, 0);
    global.name = b"g_value".into();
    globals.push(S_GDATA32, SymbolData::Data(global), 1)?;
    let mut udt = UserDefinedType::default();
    udt.type_index = point;
    udt.name = b"Point".into();
    globals.push(S_UDT, SymbolData::UserDefinedType(udt), 1)?;
    let mut publics = editor.publics_mut()?;
    let function = PublicFlags(PublicFlags::FUNCTION.0 | PublicFlags::CODE.0);
    for (name, flags, address) in [
        (SUM.name, function, at(1, SUM.offset)),
        (MAIN.name, function, at(1, MAIN.offset)),
        (b"g_value".as_slice(), PublicFlags(0), at(2, 0)),
    ] {
        let mut symbol = Public::default();
        symbol.flags = flags;
        symbol.address = address;
        symbol.name = name.into();
        publics.push(symbol)?;
    }
    Ok(())
}

/// Return a V60 section contribution of module 0.
fn contribution(section: u16, offset: i32, size: i32, characteristics: u32) -> SectionContribution {
    let mut contribution = SectionContribution::default();
    contribution.section = section;
    contribution.offset = offset;
    contribution.size = size;
    contribution.characteristics = characteristics;
    contribution.module = ModuleIndex(0);
    contribution.data_crc = Some(0);
    contribution.reloc_crc = Some(0);
    contribution.coff_section = None;
    contribution
}

/// Add the DBI header, the section contributions, the section map and the
/// section headers.
fn add_image_tables(editor: &mut PdbEditor<'_>) -> Result<()> {
    let mut header = DbiHeader::default();
    // The new version format (bit 15), major version 14 and minor version 51 (0x33).
    header.build_number = 0x8000 | (14 << 8) | 0x33;
    header.machine = Machine::AMD64;
    let info = editor.debug_info_mut()?;
    *info.header_mut() = header;
    info.set_section_contributions(
        SectionContributionVersion::V60,
        vec![
            contribution(1, 0, 0x50, TEXT_CHARACTERISTICS),
            contribution(2, 0, 4, DATA_CHARACTERISTICS),
        ],
    );
    let segment = |flags: u16, frame: u16, length: u32| {
        let mut entry = SectionMapEntry::default();
        entry.flags = SegmentFlags(flags);
        entry.frame = frame;
        entry.section_name = 0xffff;
        entry.class_name = 0xffff;
        entry.length = length;
        entry
    };
    let selector = SegmentFlags::IS_SELECTOR.0 | SegmentFlags::ADDRESS_IS_32_BIT.0;
    let mut map = SectionMap::default();
    map.logical_count = 3;
    map.entries = vec![
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
    ];
    info.set_section_map(Some(map));
    info.set_debug_slot_count(11);
    let mut text = SectionHeader::new(*b".text\0\0\0", 0x100, 0x1000);
    text.characteristics = TEXT_CHARACTERISTICS;
    let mut data = SectionHeader::new(*b".data\0\0\0", 0x100, 0x2000);
    data.characteristics = DATA_CHARACTERISTICS;
    editor.debug_streams_mut()?.set(
        DebugStream::SectionHeader,
        RecordStream::from_records(&[text, data])?.to_bytes()?,
    )
}
