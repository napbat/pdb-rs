//! The native editor (#42): edits of a parsed PDB, a write plan and checks of its output.
//!
//! The input is a part with the PDB bytes ([`Input::part`]) and then the edit bytes. Each edit
//! byte selects one operation, and the operation reads its arguments from the input. An
//! accessor error skips the operation. A plan error ends the run.

use std::collections::BTreeMap;

use pdbrs::Bytes;
use pdbrs::io::ReadAt;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, ModuleIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{
    Data, Public, PublicFlags, S_GDATA32, S_LDATA32, S_UDT, SymbolData, UserDefinedType,
};
use pdbrs::native::codeview::type_record::{ArgumentList, TypeLeaf};
use pdbrs::native::editor::{ModuleContents, ModuleSymbol, NamedStreams, SymbolRecord};
use pdbrs::native::lines::{ColumnEntry, LineBlock, LineEntry, LineSubsection, LinesSubsection};
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor, PdbWritePlan};

use crate::input::Input;
use crate::ok;

/// The number of operations that an edit byte selects.
const OPERATIONS: u8 = 17;

/// The names of new named streams, besides names from the input.
///
/// The list has names with a decoder, names that other records refer to, and `/names`, which
/// is not in the named stream collection.
const STREAM_NAMES: [&[u8]; 8] = [
    b"fuzz",
    b"raw",
    b"sourcelink",
    b"srcsrv",
    b"/LinkInfo",
    b"/TMCache",
    b"/src/files/a.c",
    b"/names",
];

/// The largest line number of a line entry (24 bits).
const LINE_MASK: u32 = 0x00ff_ffff;

/// The offset of the age in the DBI header.
const DBI_AGE: usize = 8;

/// The state of one run.
struct Run {
    /// The options of the plan.
    options: EditOptions,
    /// The contents of each changed named stream, or `None` for a removed named stream.
    named: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

/// Edit a native PDB, plan the output and check the output.
///
/// # Panics
///
/// Panics if a plan of the edits succeeds and one of these checks fails:
///
/// - A second plan of the same model gives the same bytes.
/// - The output parses with [`Pdb::parse`].
/// - The Info identity of the output is [`PdbWritePlan::identity`], and the DBI age of the
///   output is its age.
/// - Each inserted named stream has its bytes, and each removed named stream is absent.
/// - The report has one entry for each output stream, in output stream order.
pub fn editor(data: &[u8]) {
    let mut input = Input::new(data);
    let Some(pdb) = ok(Pdb::parse(input.part())) else {
        return;
    };
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let mut run = Run {
        options: EditOptions::default(),
        named: BTreeMap::new(),
    };
    while !input.is_empty() {
        apply(&mut editor, &mut run, &mut input);
    }
    let Some(plan) = ok(editor.plan(run.options)) else {
        return;
    };
    let Some(bytes) = ok(plan.to_vec()) else {
        return;
    };
    check(&editor, &plan, &bytes, &run);
}

/// Read one edit byte and apply its operation.
///
/// An operation gives `None` if an accessor error or the model skips it.
fn apply<S: ReadAt>(editor: &mut PdbEditor<'_, S>, run: &mut Run, input: &mut Input<'_>) {
    let operation = input.u8() % OPERATIONS;
    let _ = match operation {
        0..=2 => named(editor, run, input, operation),
        3..=5 => public(editor, input, operation - 3),
        6..=8 => global(editor, input, operation - 6),
        9 => push_type(editor, input),
        10 => change_module_symbol(editor, input),
        11 => remove_module_symbol(editor, input),
        12 => change_line(editor, input),
        13 => ok(editor.decode_all()),
        14 => {
            run.options = run.options.with_strip_private(!run.options.strip_private());
            Some(())
        }
        15 => remove_type(editor, input),
        _ => remove_id(editor, input),
    };
}

/// Insert (0), replace (1) or remove (2) a named stream.
fn named<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    run: &mut Run,
    input: &mut Input<'_>,
    operation: u8,
) -> Option<()> {
    let streams = ok(editor.named_streams_mut())?;
    let name = match operation {
        0 => new_name(input),
        1 => existing_name(streams, input)?,
        _ => existing_name(streams, input).unwrap_or_else(|| new_name(input)),
    };
    if operation == 2 {
        ok(streams.remove(&name))?;
        run.named.insert(name, None);
    } else {
        let bytes = input.part().to_vec();
        ok(streams.insert(&name, bytes.clone()))?;
        run.named.insert(name, Some(bytes));
    }
    Some(())
}

/// Return a name of [`STREAM_NAMES`] or a name from the input.
fn new_name(input: &mut Input<'_>) -> Vec<u8> {
    let selector = input.u8();
    if selector < 0x80 {
        STREAM_NAMES[usize::from(selector) % STREAM_NAMES.len()].to_vec()
    } else {
        input.bytes(usize::from(selector & 0x0f) + 1).to_vec()
    }
}

/// Return the name of a named stream of the collection, or `None` if it is empty.
fn existing_name(streams: &NamedStreams, input: &mut Input<'_>) -> Option<Vec<u8>> {
    let count = streams.len();
    if count == 0 {
        return None;
    }
    let position = usize::from(input.u16()) % count;
    streams.names().nth(position).map(<[u8]>::to_vec)
}

/// Return a symbol name from the input.
fn symbol_name(input: &mut Input<'_>) -> Bytes<'static> {
    let length = usize::from(input.u8() % 32);
    Bytes::from(input.bytes(length).to_vec())
}

/// Return a section offset from the input.
fn address(input: &mut Input<'_>) -> SectionOffset {
    SectionOffset {
        section: input.u16(),
        offset: input.u32(),
    }
}

/// Return the position of an entry of a list with `count` entries, or `None` for no entries.
fn position(count: usize, input: &mut Input<'_>) -> Option<usize> {
    (count != 0).then(|| usize::from(input.u16()) % count)
}

/// Give a record the name from the input, if its kind has a name.
fn rename(record: &mut SymbolRecord, input: &mut Input<'_>) {
    let name = symbol_name(input);
    if let Some(slot) = record.data_mut().name_mut() {
        *slot = name;
    }
}

/// Rename (0), remove (1) or push (2) a public symbol.
fn public<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    input: &mut Input<'_>,
    operation: u8,
) -> Option<()> {
    let mut publics = ok(editor.publics_mut())?;
    if operation == 2 {
        let mut public = Public::default();
        public.flags = PublicFlags(input.u32() & 0xf);
        public.address = address(input);
        public.name = symbol_name(input);
        ok(publics.push(public))?;
        return Some(());
    }
    let handle = publics.handles().nth(position(publics.len(), input)?)?;
    if operation == 0 {
        rename(publics.get_mut(handle)?, input);
    } else {
        ok(publics.remove(handle))?;
    }
    Some(())
}

/// Rename (0), remove (1) or push (2) a global symbol.
fn global<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    input: &mut Input<'_>,
    operation: u8,
) -> Option<()> {
    let mut globals = ok(editor.globals_mut())?;
    if operation == 2 {
        let selector = input.u8() % 3;
        let type_index = TypeIndex(input.u32());
        let name = symbol_name(input);
        let reference_count = u32::from(input.u8());
        let (kind, data) = if selector == 0 {
            let mut record = UserDefinedType::default();
            record.type_index = type_index;
            record.name = name;
            (S_UDT, SymbolData::UserDefinedType(record))
        } else {
            let mut record = Data::default();
            record.type_index = type_index;
            record.address = address(input);
            record.name = name;
            let kind = if selector == 1 { S_GDATA32 } else { S_LDATA32 };
            (kind, SymbolData::Data(record))
        };
        ok(globals.push(kind, data, reference_count))?;
        return Some(());
    }
    let handle = globals.handles().nth(position(globals.len(), input)?)?;
    if operation == 0 {
        rename(globals.get_mut(handle)?, input);
    } else {
        ok(globals.remove(handle))?;
    }
    Some(())
}

/// Push an `LF_ARGLIST` record of primitive types and earlier type indices.
fn push_type<S: ReadAt>(editor: &mut PdbEditor<'_, S>, input: &mut Input<'_>) -> Option<()> {
    let types = ok(editor.types_mut())?;
    let end = ok(types.end())?.0;
    let count = input.u8() % 8;
    let mut indices = Vec::new();
    for _ in 0..count {
        let selector = input.u8();
        let index = if selector & 1 == 0 || end <= FIRST_RECORD_INDEX {
            u32::from(selector >> 1)
        } else {
            FIRST_RECORD_INDEX + u32::from(input.u16()) % (end - FIRST_RECORD_INDEX)
        };
        indices.extend_from_slice(&index.to_le_bytes());
    }
    let leaf = TypeLeaf::ArgumentList(ok(ArgumentList::parse(indices))?);
    ok(types.push(leaf))?;
    Some(())
}

/// Return a record index below `end` that the input selects, or `None` for an empty list.
fn record_index(end: u32, input: &mut Input<'_>) -> Option<u32> {
    let count = end.checked_sub(FIRST_RECORD_INDEX).filter(|&count| count > 0)?;
    Some(FIRST_RECORD_INDEX + u32::from(input.u16()) % count)
}

/// Remove a type record (#47). The plan rejects a removal of a referenced record.
fn remove_type<S: ReadAt>(editor: &mut PdbEditor<'_, S>, input: &mut Input<'_>) -> Option<()> {
    let types = ok(editor.types_mut())?;
    let index = record_index(ok(types.end())?.0, input)?;
    ok(types.remove(TypeIndex(index)))
}

/// Remove an ID record (#47). The plan rejects a removal of a referenced record.
fn remove_id<S: ReadAt>(editor: &mut PdbEditor<'_, S>, input: &mut Input<'_>) -> Option<()> {
    let ids = ok(editor.ids_mut())?;
    let index = record_index(ok(ids.end())?.0, input)?;
    ok(ids.remove(IdIndex(index)))
}

/// Return a module that the input selects.
fn module<'e, S: ReadAt>(
    editor: &'e mut PdbEditor<'_, S>,
    input: &mut Input<'_>,
) -> Option<&'e mut ModuleContents> {
    let index = position(ok(editor.module_count())?, input)?;
    ok(editor.module_mut(ModuleIndex(u32::try_from(index).ok()?)))
}

/// Return the number of records and scope openers of `nodes`.
fn record_count(nodes: &[ModuleSymbol]) -> usize {
    nodes
        .iter()
        .map(|node| match node {
            ModuleSymbol::Record(_) => 1,
            ModuleSymbol::Scope(scope) => 1 + record_count(&scope.children),
        })
        .sum()
}

/// Return the record or scope opener `n` of `nodes` in pre-order.
///
/// The function decreases `n` by the number of records before the end of `nodes`.
fn nth_record<'n>(nodes: &'n mut [ModuleSymbol], n: &mut usize) -> Option<&'n mut SymbolRecord> {
    for node in nodes {
        if *n == 0 {
            return Some(node.record_mut());
        }
        *n -= 1;
        if let ModuleSymbol::Scope(scope) = node
            && let Some(record) = nth_record(&mut scope.children, n)
        {
            return Some(record);
        }
    }
    None
}

/// Remove the node `n` of `nodes` in pre-order, with the children of a scope.
///
/// Return `false` and decrease `n` by the number of nodes if `nodes` has fewer nodes.
fn remove_nth(nodes: &mut Vec<ModuleSymbol>, n: &mut usize) -> bool {
    let mut index = 0;
    while index < nodes.len() {
        if *n == 0 {
            nodes.remove(index);
            return true;
        }
        *n -= 1;
        if let ModuleSymbol::Scope(scope) = &mut nodes[index]
            && remove_nth(&mut scope.children, n)
        {
            return true;
        }
        index += 1;
    }
    false
}

/// Rename a module symbol, or give it a new type index.
fn change_module_symbol<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    input: &mut Input<'_>,
) -> Option<()> {
    let symbols = ok(module(editor, input)?.symbols_mut())?;
    let mut n = position(record_count(symbols), input)?;
    let record = nth_record(symbols, &mut n)?;
    if input.u8() & 1 == 0 {
        rename(record, input);
        return Some(());
    }
    let index = TypeIndex(input.u32());
    match record.data_mut() {
        SymbolData::Data(value) | SymbolData::ThreadData(value) => value.type_index = index,
        SymbolData::UserDefinedType(value) => value.type_index = index,
        SymbolData::Constant(value) => value.type_index = index,
        SymbolData::Local(value) => value.type_index = index,
        SymbolData::RegisterRelative(value) => value.type_index = index,
        SymbolData::BasePointerRelative(value) => value.type_index = index,
        _ => {}
    }
    Some(())
}

/// Remove a module symbol: a record, or a scope with its children.
fn remove_module_symbol<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    input: &mut Input<'_>,
) -> Option<()> {
    let symbols = ok(module(editor, input)?.symbols_mut())?;
    let mut n = position(record_count(symbols), input)?;
    remove_nth(symbols, &mut n).then_some(())
}

/// Return the number of line entries of a `DEBUG_S_LINES` subsection, or 0 for another kind.
fn line_count(subsection: &LineSubsection<'_>) -> usize {
    match subsection {
        LineSubsection::Lines(lines) => lines.blocks().map(|block| block.lines.len()).sum(),
        _ => 0,
    }
}

/// Give a line entry of a module a line number from the input.
fn change_line<S: ReadAt>(editor: &mut PdbEditor<'_, S>, input: &mut Input<'_>) -> Option<()> {
    let lines = module(editor, input)?.lines_mut();
    let mut n = position(lines.iter().map(line_count).sum(), input)?;
    let line = input.u32() & LINE_MASK;
    for subsection in lines.iter_mut() {
        let count = line_count(subsection);
        if n >= count {
            n -= count;
            continue;
        }
        let LineSubsection::Lines(old) = &*subsection else {
            return None;
        };
        *subsection = LineSubsection::Lines(with_line(old, n, line)?);
        return Some(());
    }
    None
}

/// Return a copy of `old` in which the line entry `n` has the line number `line`.
fn with_line(old: &LinesSubsection<'_>, n: usize, line: u32) -> Option<LinesSubsection<'static>> {
    let mut data = Vec::new();
    ok(old.header().encode(&mut data))?;
    let mut target = Some(n);
    for block in old.blocks() {
        let mut entries: Vec<LineEntry> = block.lines.iter().collect();
        if let Some(rest) = target {
            if let Some(entry) = entries.get_mut(rest) {
                entry.line_start = line;
                target = None;
            } else {
                target = Some(rest - entries.len());
            }
        }
        let columns: Option<Vec<ColumnEntry>> = block
            .columns
            .as_ref()
            .map(|columns| columns.iter().collect());
        ok(LineBlock::encode_parts(
            block.checksum_offset,
            &entries,
            columns.as_deref(),
            &mut data,
        ))?;
    }
    Some(ok(LinesSubsection::parse(&data))?.into_owned())
}

/// Check the invariants of a plan and of its output `bytes`.
fn check<S: ReadAt>(
    editor: &PdbEditor<'_, S>,
    plan: &PdbWritePlan<'_, S>,
    bytes: &[u8],
    run: &Run,
) {
    let again = editor
        .plan(run.options)
        .expect("a second plan of the same model succeeds");
    let again = again
        .to_vec()
        .expect("a second plan of the same model writes");
    assert!(
        again == bytes,
        "a second plan of the same model gives other bytes"
    );

    let output = Pdb::parse(bytes).expect("the output parses");
    let identity = plan.identity().expect("the plan gives the identity");
    assert_eq!(
        output.info().identity(),
        identity,
        "the Info identity of the output is not the plan identity"
    );
    if let Some(age) = ok(output.stream_bytes(StreamId(3)))
        .and_then(|dbi| dbi.get(DBI_AGE..DBI_AGE + 4)?.try_into().ok())
        .map(u32::from_le_bytes)
    {
        assert_eq!(
            age,
            identity.age(),
            "the DBI age of the output is not the age of the plan identity"
        );
    }

    for (name, expected) in &run.named {
        let name_text = String::from_utf8_lossy(name);
        match expected {
            Some(expected) => {
                let actual = output
                    .named_stream_bytes(name)
                    .expect("an inserted named stream reads");
                assert!(
                    actual.as_ref() == Some(expected),
                    "the named stream {name_text:?} does not have its bytes"
                );
            }
            None => assert!(
                output.info().named_stream(name).is_none(),
                "the removed named stream {name_text:?} is in the output"
            ),
        }
    }

    let count = plan.stream_count();
    assert_eq!(
        output.container().stream_count(),
        count,
        "the output does not have the stream count of the plan"
    );
    let outputs = plan
        .report()
        .streams()
        .iter()
        .filter_map(|entry| entry.output);
    assert!(
        outputs.map(|id| id.0).eq(0..count),
        "the report does not have one entry for each output stream in output stream order"
    );
}
