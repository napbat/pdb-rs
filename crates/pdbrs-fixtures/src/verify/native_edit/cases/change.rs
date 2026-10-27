//! The changes of the new edit cases (S6c-1 of #42).
//!
//! [`prepare`] finds the first record of a case in the source PDB and changes it in the
//! editor. It returns the [`Target`]: the place and the new value that the read of the output
//! must find.

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::SymbolKind;
use pdbrs::native::codeview::field_list::{
    FieldList, LLVM_FIELD_LIST_LIMIT, Member, encode_field_list,
};
use pdbrs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    S_BPREL32, S_GDATA32, S_LABEL32, S_LDATA32, S_LOCAL, S_REGREL32, SymbolData,
};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::editor::{ModuleSymbol, SymbolRecord};
use pdbrs::native::lines::{
    ColumnEntry, LineBlock, LineEntry, LineSubsection, LinesSubsection, NEVER_STEP_INTO_LINE,
};
use pdbrs::native::{Pdb, PdbEditor};
use pdbrs::{Error, ErrorKind};

use super::super::edits::{
    Expectation, T_INT4, absent, first_public, first_rename_global, first_unique, indexes, lossy,
};
use super::Case;

/// The `T_UINT4` primitive type.
const T_UINT4: TypeIndex = TypeIndex(0x75);

/// The largest member offset that the member offset case changes. The new offset stays an
/// immediate numeric leaf, so the record keeps its size.
const MEMBER_OFFSET_LIMIT: u64 = 0x7FFF;

/// The kinds of the module symbols of the local type case.
const LOCAL_TYPE_KINDS: [SymbolKind; 4] = [S_REGREL32, S_BPREL32, S_LOCAL, S_LDATA32];

/// The kinds of the module symbols of the module symbol removal.
const REMOVED_KINDS: [SymbolKind; 3] = [S_LABEL32, S_REGREL32, S_BPREL32];

/// The place that a case changed, and the value that the output must have there.
#[derive(Debug)]
pub(super) enum Target {
    /// A public symbol with its new address.
    Public {
        /// The name of the symbol.
        name: Vec<u8>,
        /// The new address.
        address: SectionOffset,
    },
    /// A global data symbol with its new type.
    Global {
        /// The name of the symbol.
        name: Vec<u8>,
        /// The new type index.
        type_index: TypeIndex,
    },
    /// A module symbol with its new type.
    ModuleSymbol {
        /// The module.
        module: ModuleIndex,
        /// The position of the record in the pre-order of the module.
        position: usize,
        /// The kind of the record.
        kind: SymbolKind,
        /// The new type index.
        type_index: TypeIndex,
    },
    /// A line entry with its new line number.
    Line {
        /// The module.
        module: ModuleIndex,
        /// The position of the `DEBUG_S_LINES` subsection in the module.
        subsection: usize,
        /// The position of the block in the subsection.
        block: usize,
        /// The position of the line entry in the block.
        entry: usize,
        /// The new line number.
        line: u32,
    },
    /// An `LF_MEMBER` member with its new offset.
    Member {
        /// The index of the `LF_FIELDLIST` record.
        field_list: TypeIndex,
        /// The position of the member in the record.
        member: usize,
        /// The new offset.
        offset: u64,
    },
    /// A public symbol that the output must not have.
    PublicRemoved {
        /// The name of the symbol.
        name: Vec<u8>,
    },
    /// A global symbol that the output must not have.
    GlobalRemoved {
        /// The name of the symbol.
        name: Vec<u8>,
    },
    /// A module whose records must have these kinds in pre-order: the source kinds without
    /// the removed record.
    ModuleSymbolRemoved {
        /// The module.
        module: ModuleIndex,
        /// The kinds of the records in pre-order.
        kinds: Vec<SymbolKind>,
    },
}

/// The result of [`prepare`].
#[derive(Debug)]
pub(super) enum Prepared {
    /// The case does not apply to the PDB. The text tells why.
    NotApplied(String),
    /// The editor has the change.
    Changed(Box<(Target, Expectation)>),
}

/// Return a [`Prepared::Changed`] value.
fn changed(target: Target, expect: Expectation) -> Prepared {
    Prepared::Changed(Box::new((target, expect)))
}

/// Return the error for an unexpected record.
fn unexpected(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// Make the change of `case` in `editor`. `pdb` is the source of the editor.
pub(super) fn prepare(
    case: Case,
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
) -> pdbrs::Result<Prepared> {
    match case {
        Case::PublicAddress => public(pdb, editor, false),
        Case::RemovePublic => public(pdb, editor, true),
        Case::GlobalType => global_type(pdb, editor),
        Case::RemoveGlobal => remove_global(pdb, editor),
        Case::LocalType => local_type(editor),
        Case::LineNumber => line_number(editor),
        Case::MemberOffset => member_offset(editor),
        Case::RemoveModuleSymbol => remove_module_symbol(editor),
    }
}

/// Return the type that the type cases write in place of `old`.
const fn new_type(old: TypeIndex) -> TypeIndex {
    if old.0 == T_INT4.0 { T_UINT4 } else { T_INT4 }
}

/// Return the type index of a record of the type cases.
pub(super) const fn type_of(data: &SymbolData<'_>) -> Option<TypeIndex> {
    match data {
        SymbolData::RegisterRelative(value) => Some(value.type_index),
        SymbolData::BasePointerRelative(value) => Some(value.type_index),
        SymbolData::Local(value) => Some(value.type_index),
        SymbolData::Data(value) => Some(value.type_index),
        _ => None,
    }
}

/// Return the type index of a record of the type cases, for a change.
const fn type_slot<'d>(data: &'d mut SymbolData<'_>) -> Option<&'d mut TypeIndex> {
    match data {
        SymbolData::RegisterRelative(value) => Some(&mut value.type_index),
        SymbolData::BasePointerRelative(value) => Some(&mut value.type_index),
        SymbolData::Local(value) => Some(&mut value.type_index),
        SymbolData::Data(value) => Some(&mut value.type_index),
        _ => None,
    }
}

/// Change the address of the first public symbol with a unique name, or remove the symbol.
fn public(
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
    remove: bool,
) -> pdbrs::Result<Prepared> {
    let indexes = indexes(pdb)?;
    let Some((name, _)) = first_public(&indexes)? else {
        return Ok(Prepared::NotApplied(
            "no public symbol with a unique name".into(),
        ));
    };
    let mut publics = editor.publics_mut()?;
    let handle = publics.find(name)?.ok_or_else(absent)?;
    let mut expect = Expectation::default();
    if remove {
        publics.remove(handle)?;
        expect.publics_absent.push(lossy(name));
        return Ok(changed(
            Target::PublicRemoved {
                name: name.to_vec(),
            },
            expect,
        ));
    }
    let record = publics.get_mut(handle).ok_or_else(absent)?;
    let SymbolData::Public(public) = record.data_mut() else {
        return Err(unexpected("the public symbol is not an S_PUB32 record"));
    };
    public.address.offset = public
        .address
        .offset
        .checked_add(1)
        .ok_or_else(|| unexpected("the public symbol address overflows"))?;
    let address = public.address;
    Ok(changed(
        Target::Public {
            name: name.to_vec(),
            address,
        },
        expect,
    ))
}

/// Change the type of the first `S_GDATA32` or `S_LDATA32` global with a unique name.
fn global_type(pdb: &Pdb<&[u8]>, editor: &mut PdbEditor<'_, &[u8]>) -> pdbrs::Result<Prepared> {
    let indexes = indexes(pdb)?;
    let global = match indexes.globals {
        Some(globals) => first_unique(indexes.records, globals.hash_table(), |kind| {
            matches!(kind, S_GDATA32 | S_LDATA32)
        })?,
        None => None,
    };
    let Some((name, _)) = global else {
        return Ok(Prepared::NotApplied(
            "no S_GDATA32 or S_LDATA32 global with a unique name".into(),
        ));
    };
    let mut globals = editor.globals_mut()?;
    let handle = globals.find(name)?.ok_or_else(absent)?;
    let record = globals.get_mut(handle).ok_or_else(absent)?;
    let SymbolData::Data(data) = record.data_mut() else {
        return Err(unexpected("the global symbol is not a data record"));
    };
    data.type_index = new_type(data.type_index);
    let type_index = data.type_index;
    Ok(changed(
        Target::Global {
            name: name.to_vec(),
            type_index,
        },
        Expectation::default(),
    ))
}

/// Remove the global symbol of the rename edit.
fn remove_global(pdb: &Pdb<&[u8]>, editor: &mut PdbEditor<'_, &[u8]>) -> pdbrs::Result<Prepared> {
    let indexes = indexes(pdb)?;
    let Some((name, _)) = first_rename_global(&indexes)? else {
        return Ok(Prepared::NotApplied(
            "no global data, constant or UDT symbol with a unique name".into(),
        ));
    };
    let mut globals = editor.globals_mut()?;
    let handle = globals.find(name)?.ok_or_else(absent)?;
    globals.remove(handle)?;
    let mut expect = Expectation::default();
    expect.globals_absent.push(lossy(name));
    Ok(changed(
        Target::GlobalRemoved {
            name: name.to_vec(),
        },
        expect,
    ))
}

/// Return the records of `nodes` in pre-order: a record, or the opener of a scope, then its
/// children, then its closer.
pub(super) fn preorder<'n>(nodes: &'n [ModuleSymbol], out: &mut Vec<&'n SymbolRecord>) {
    for node in nodes {
        match node {
            ModuleSymbol::Record(record) => out.push(record),
            ModuleSymbol::Scope(scope) => {
                out.push(&scope.opener);
                preorder(&scope.children, out);
                out.push(&scope.closer);
            }
        }
    }
}

/// Return the record at pre-order position `left` of `nodes`. Decrement `left` for each
/// record before it.
fn nth_mut<'n>(nodes: &'n mut [ModuleSymbol], left: &mut usize) -> Option<&'n mut SymbolRecord> {
    for node in nodes {
        match node {
            ModuleSymbol::Record(record) => {
                if *left == 0 {
                    return Some(record);
                }
                *left -= 1;
            }
            ModuleSymbol::Scope(scope) => {
                if *left == 0 {
                    return Some(&mut scope.opener);
                }
                *left -= 1;
                if let Some(found) = nth_mut(&mut scope.children, left) {
                    return Some(found);
                }
                if *left == 0 {
                    return Some(&mut scope.closer);
                }
                *left -= 1;
            }
        }
    }
    None
}

/// Remove the record node at pre-order position `left` of `nodes`. Return `true` if a node
/// was removed. The node must not be a scope.
fn remove_nth(nodes: &mut Vec<ModuleSymbol>, left: &mut usize) -> bool {
    let mut index = 0;
    while index < nodes.len() {
        match &mut nodes[index] {
            ModuleSymbol::Record(_) => {
                if *left == 0 {
                    nodes.remove(index);
                    return true;
                }
                *left -= 1;
            }
            ModuleSymbol::Scope(scope) => {
                if *left == 0 {
                    return false;
                }
                *left -= 1;
                if remove_nth(&mut scope.children, left) {
                    return true;
                }
                if *left == 0 {
                    return false;
                }
                *left -= 1;
            }
        }
        index += 1;
    }
    false
}

/// Return the module index of position `index`.
fn module_index(index: usize) -> pdbrs::Result<ModuleIndex> {
    u32::try_from(index)
        .map(ModuleIndex)
        .map_err(|_| Error::new(ErrorKind::LimitExceeded, None).with_context("too many modules"))
}

/// Return the first module, in module order, with a record in pre-order that `select`
/// accepts: the module, the position of the record and the kinds of all records in
/// pre-order. Skip the modules whose symbols stay opaque.
fn first_module_symbol(
    editor: &mut PdbEditor<'_, &[u8]>,
    select: impl Fn(&SymbolRecord) -> bool,
) -> pdbrs::Result<Option<(ModuleIndex, usize, Vec<SymbolKind>)>> {
    for index in 0..editor.module_count()? {
        let module = module_index(index)?;
        let contents = editor.module_mut(module)?;
        let nodes = match contents.symbols() {
            Ok(nodes) => nodes,
            Err(error) if error.kind() == ErrorKind::UnsupportedEdit => continue,
            Err(error) => return Err(error),
        };
        let mut records = Vec::new();
        preorder(nodes, &mut records);
        if let Some(position) = records.iter().position(|record| select(record)) {
            let kinds = records.iter().map(|record| record.kind()).collect();
            return Ok(Some((module, position, kinds)));
        }
    }
    Ok(None)
}

/// Change the type of the first module symbol of a [`LOCAL_TYPE_KINDS`] kind.
fn local_type(editor: &mut PdbEditor<'_, &[u8]>) -> pdbrs::Result<Prepared> {
    let found = first_module_symbol(editor, |record| {
        LOCAL_TYPE_KINDS.contains(&record.kind()) && type_of(record.data()).is_some()
    })?;
    let Some((module, position, kinds)) = found else {
        return Ok(Prepared::NotApplied(
            "no module symbol of kind S_REGREL32, S_BPREL32, S_LOCAL or S_LDATA32".into(),
        ));
    };
    let nodes = editor.module_mut(module)?.symbols_mut()?;
    let mut left = position;
    let record = nth_mut(nodes, &mut left).ok_or_else(absent)?;
    let slot = type_slot(record.data_mut())
        .ok_or_else(|| unexpected("the module symbol has no type index"))?;
    *slot = new_type(*slot);
    let type_index = *slot;
    Ok(changed(
        Target::ModuleSymbol {
            module,
            position,
            kind: kinds[position],
            type_index,
        },
        Expectation::default(),
    ))
}

/// Remove the first module symbol of a [`REMOVED_KINDS`] kind.
fn remove_module_symbol(editor: &mut PdbEditor<'_, &[u8]>) -> pdbrs::Result<Prepared> {
    let found = first_module_symbol(editor, |record| REMOVED_KINDS.contains(&record.kind()))?;
    let Some((module, position, mut kinds)) = found else {
        return Ok(Prepared::NotApplied(
            "no module symbol of kind S_LABEL32, S_REGREL32 or S_BPREL32".into(),
        ));
    };
    let nodes = editor.module_mut(module)?.symbols_mut()?;
    let mut left = position;
    if !remove_nth(nodes, &mut left) {
        return Err(unexpected("the module symbol to remove opens a scope"));
    }
    kinds.remove(position);
    Ok(changed(
        Target::ModuleSymbolRemoved { module, kinds },
        Expectation::default(),
    ))
}

/// Return the first line entry of a `DEBUG_S_LINES` subsection with a line number below
/// [`NEVER_STEP_INTO_LINE`]: the positions of the subsection, the block and the entry, and
/// the line number.
fn first_line(lines: &[LineSubsection<'static>]) -> Option<(usize, usize, usize, u32)> {
    for (subsection, data) in lines.iter().enumerate() {
        let LineSubsection::Lines(data) = data else {
            continue;
        };
        for (block, entries) in data.blocks().enumerate() {
            for (entry, line) in entries.lines.iter().enumerate() {
                if line.line_start < NEVER_STEP_INTO_LINE {
                    return Some((subsection, block, entry, line.line_start));
                }
            }
        }
    }
    None
}

/// Return `data` with the line number + 1 in entry `entry` of block `block`.
fn next_line(
    data: &LinesSubsection<'_>,
    block: usize,
    entry: usize,
) -> pdbrs::Result<LineSubsection<'static>> {
    let mut bytes = Vec::new();
    data.header().encode(&mut bytes)?;
    for (position, current) in data.blocks().enumerate() {
        if position == block {
            let mut lines: Vec<LineEntry> = current.lines.iter().collect();
            let line = lines.get_mut(entry).ok_or_else(absent)?;
            line.line_start += 1;
            let columns: Option<Vec<ColumnEntry>> = current
                .columns
                .as_ref()
                .map(|columns| columns.iter().collect());
            LineBlock::encode_parts(
                current.checksum_offset,
                &lines,
                columns.as_deref(),
                &mut bytes,
            )?;
        } else {
            current.encode(&mut bytes)?;
        }
    }
    Ok(LineSubsection::Lines(
        LinesSubsection::parse(&bytes)?.into_owned(),
    ))
}

/// Add 1 to the line number of the first line entry (see [`first_line`]), in module order.
fn line_number(editor: &mut PdbEditor<'_, &[u8]>) -> pdbrs::Result<Prepared> {
    for index in 0..editor.module_count()? {
        let module = module_index(index)?;
        let contents = editor.module_mut(module)?;
        let Some((subsection, block, entry, line)) = first_line(contents.lines()) else {
            continue;
        };
        let LineSubsection::Lines(data) = &contents.lines()[subsection] else {
            return Err(unexpected(
                "the subsection is not a DEBUG_S_LINES subsection",
            ));
        };
        let new = next_line(data, block, entry)?;
        contents.lines_mut()[subsection] = new;
        return Ok(changed(
            Target::Line {
                module,
                subsection,
                block,
                entry,
                line: line + 1,
            },
            Expectation::default(),
        ));
    }
    Ok(Prepared::NotApplied(
        "no DEBUG_S_LINES line entry with a line number below 0xF00F00".into(),
    ))
}

/// Return the offset of an `LF_MEMBER` member that the member offset case changes.
fn member_offset_value(member: &Member<'_>) -> Option<u64> {
    let Member::DataMember(data) = member else {
        return None;
    };
    data.offset
        .integer()
        .and_then(Integer::to_u64)
        .filter(|&offset| offset < MEMBER_OFFSET_LIMIT)
}

/// Return the first `LF_FIELDLIST` record, in index order, with an `LF_MEMBER` member whose
/// offset is below [`MEMBER_OFFSET_LIMIT`]: its index, a copy of the list and the position
/// of the member.
fn first_member(
    editor: &mut PdbEditor<'_, &[u8]>,
) -> pdbrs::Result<Option<(TypeIndex, FieldList<'static>, usize)>> {
    for (index, leaf) in editor.types_mut()?.iter() {
        let TypeLeaf::FieldList(list) = leaf else {
            continue;
        };
        for (position, member) in list.members().enumerate() {
            if member_offset_value(&member?).is_some() {
                return Ok(Some((index, list.clone(), position)));
            }
        }
    }
    Ok(None)
}

/// Add 1 to the offset of the first `LF_MEMBER` member (see [`first_member`]) and encode the
/// field list again. The case does not apply if the encoder splits the list.
fn member_offset(editor: &mut PdbEditor<'_, &[u8]>) -> pdbrs::Result<Prepared> {
    let Some((index, list, position)) = first_member(editor)? else {
        return Ok(Prepared::NotApplied(
            "no LF_FIELDLIST with an LF_MEMBER offset below 0x7FFF".into(),
        ));
    };
    let mut members = list.members().collect::<pdbrs::Result<Vec<_>>>()?;
    let offset = members
        .get(position)
        .and_then(member_offset_value)
        .ok_or_else(absent)?
        + 1;
    if let Some(Member::DataMember(data)) = members.get_mut(position) {
        data.offset = Numeric::from_integer(Integer::Unsigned(u128::from(offset)));
    }
    let mut out = Vec::new();
    let records = encode_field_list(
        members
            .iter()
            .filter(|member| !matches!(member, Member::Continuation(_))),
        TypeIndex(index.0),
        LLVM_FIELD_LIST_LIMIT,
        &mut out,
    )?;
    if records.count != 1 {
        return Ok(Prepared::NotApplied(format!(
            "the encoder splits the field list {:#x} into {} records",
            index.0, records.count
        )));
    }
    let record = Records::new(&out)
        .next()
        .ok_or_else(|| unexpected("the field list has no record"))??;
    let mut payload = record.data.to_vec();
    if let Some(continuation) = list.continuation() {
        Member::Continuation(continuation).encode(&mut payload)?;
    }
    let new = FieldList::parse(payload)?.into_owned();
    let leaf = editor.types_mut()?.get_mut(index).ok_or_else(absent)?;
    *leaf = TypeLeaf::FieldList(new);
    Ok(changed(
        Target::Member {
            field_list: index,
            member: position,
            offset,
        },
        Expectation::default(),
    ))
}
