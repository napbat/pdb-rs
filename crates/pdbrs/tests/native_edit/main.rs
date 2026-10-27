//! Tests of `native::PdbEditor`: unchanged streams, named streams, symbol,
//! type and identity edits, and the rejections before output (#34).
//!
//! The source PDB is the synthetic program of the `native_synthetic`
//! example. `strip.rs` has the strip tests.

#[path = "../../examples/native_synthetic/program.rs"]
mod program;
mod strip;

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::id_record::{IdRecord, StringId};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    Data, Public, PublicFlags, S_GDATA32, S_PROCREF, S_PUB32, S_UDT, Symbol, SymbolData,
    UserDefinedType,
};
use pdbrs::native::codeview::type_record::{Class, ClassKind, ClassOptions, TypeLeaf};
use pdbrs::native::codeview::{Records, SymbolKind};
use pdbrs::native::msf::{MsfWriteOptions, StreamId};
use pdbrs::native::symbols::{HashTable, RecordOffsets, indexed_name};
use pdbrs::native::types::TypeHashes;
use pdbrs::native::{BuildOptions, EditOptions, Identity, IdentityPolicy, Pdb, PdbBuilder};
use pdbrs::{Bytes, ErrorKind};

/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);

/// Return an `S_PUB32` record with `flags` at `address`.
fn public(flags: PublicFlags, address: SectionOffset, name: &[u8]) -> Public<'_> {
    let mut public = Public::default();
    public.flags = flags;
    public.address = address;
    public.name = Bytes::new(name);
    public
}

/// Return an `S_UDT` record for `type_index`.
fn udt(type_index: TypeIndex, name: &[u8]) -> SymbolData<'_> {
    let mut udt = UserDefinedType::default();
    udt.type_index = type_index;
    udt.name = Bytes::new(name);
    SymbolData::UserDefinedType(udt)
}

/// Build the synthetic program with the changes of `extra`.
fn build(extra: impl FnOnce(&mut PdbBuilder<'static>) -> pdbrs::Result<()>) -> Vec<u8> {
    let mut builder = program::build().expect("the synthetic program builds");
    extra(&mut builder).expect("the extra records build");
    builder
        .plan(BuildOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("the synthetic program plans")
}

/// Build the synthetic program.
fn synthetic() -> Vec<u8> {
    build(|_| Ok(()))
}

/// Return the bytes of stream `id`, or `None` for a nil or absent stream.
fn stream(pdb: &Pdb<&[u8]>, id: u32) -> Option<Vec<u8>> {
    if id >= pdb.container().stream_count() {
        return None;
    }
    pdb.container()
        .stream(StreamId(id))
        .expect("stream directory")
        .map(|reader| reader.read_all(usize::MAX).expect("stream bytes"))
}

/// Return the IDs of the streams after stream 0 that differ between `left`
/// and `right`, with the streams that only one side has.
fn differing_streams(left: &[u8], right: &[u8]) -> Vec<u32> {
    let left = Pdb::parse(left).expect("left PDB");
    let right = Pdb::parse(right).expect("right PDB");
    let count = left
        .container()
        .stream_count()
        .max(right.container().stream_count());
    (1..count)
        .filter(|&id| stream(&left, id) != stream(&right, id))
        .collect()
}

/// Return the names of the records of a symbol index.
fn names(records: &[u8], table: HashTable<'_>) -> Vec<Vec<u8>> {
    table
        .records()
        .map(|record| {
            indexed_name(records, record.symbol)
                .expect("indexed name")
                .to_vec()
        })
        .collect()
}

/// The symbol record stream and the names of the global and public indexes.
struct Symbols {
    /// The global names.
    globals: Vec<Vec<u8>>,
    /// The public names.
    publics: Vec<Vec<u8>>,
}

/// Read the indexes and check that each hash record names a record start.
fn symbols(bytes: &[u8]) -> Symbols {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    let id = dbi.header().symbol_stream.expect("symbol record stream");
    let records = pdb.stream_bytes(id).expect("symbol records");
    let offsets = RecordOffsets::frame(&records, id).expect("records frame");
    let globals = pdb.global_symbols(&dbi).expect("GSI").expect("GSI present");
    let publics = pdb.public_symbols(&dbi).expect("PSI").expect("PSI present");
    globals
        .hash_table()
        .check_records(&offsets)
        .expect("global offsets");
    publics.check_records(&offsets).expect("public offsets");
    publics
        .check_address_order(|offset| {
            let symbol = Symbol::decode(offset.record(&records)?)?;
            match symbol.data() {
                SymbolData::Public(public) => Ok((public.address.section, public.address.offset)),
                _ => Err(pdbrs::Error::new(ErrorKind::InvalidFormat, None)),
            }
        })
        .expect("address order");
    Symbols {
        globals: names(&records, globals.hash_table()),
        publics: names(&records, publics.hash_table()),
    }
}

/// Return `true` if `list` has the name `name`.
fn has(list: &[Vec<u8>], name: &[u8]) -> bool {
    list.iter().any(|entry| entry == name)
}

#[test]
fn edit_without_changes_keeps_every_stream() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let plan = pdb
        .edit(IdentityPolicy::Preserve)
        .plan(EditOptions::default())
        .expect("plan");
    assert_eq!(plan.changed_streams(), []);
    let edited = plan.to_vec().expect("output");
    assert_eq!(differing_streams(&source, &edited), Vec::<u32>::new());
    let source_identity = pdb.info().identity();
    let edited = Pdb::parse(edited.as_slice()).expect("edited");
    assert_eq!(edited.info().identity(), source_identity);
}

#[test]
fn edit_options_select_the_page_size() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let page_size = |options: EditOptions| {
        let plan = pdb
            .edit(IdentityPolicy::Preserve)
            .plan(options)
            .expect("plan");
        let edited = plan.to_vec().expect("output");
        assert_eq!(plan.file_size(), u64::try_from(edited.len()).unwrap());
        let edited = Pdb::parse(edited.as_slice()).expect("edited");
        assert_eq!(plan.stream_count(), edited.container().stream_count());
        edited.container().msf().expect("MSF 7.00").block_size()
    };
    // The default keeps the page size of the source.
    assert_eq!(EditOptions::default().msf(), None);
    assert_eq!(page_size(EditOptions::default()), 4096);
    let options = EditOptions::default().with_msf(MsfWriteOptions::default().with_page_size(512));
    assert_eq!(options.msf().map(MsfWriteOptions::page_size), Some(512));
    assert_eq!(page_size(options), 512);
}

#[test]
fn named_streams_are_added_replaced_and_removed() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let count = pdb.container().stream_count();
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let first = b"SRCSRV: ini ----\r\nVERSION=2\r\nSRCSRV: end ----\r\n";
    let second = b"SRCSRV: ini ----\r\nVERSION=3\r\nSRCSRV: end ----\r\n";
    editor
        .set_named_stream(b"srcsrv", first.to_vec())
        .expect("srcsrv");
    editor
        .set_named_stream(b"raw", b"raw bytes".to_vec())
        .expect("raw");
    editor
        .set_named_stream(b"srcsrv", second.to_vec())
        .expect("srcsrv again");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    // The Info stream and the two new streams change.
    assert_eq!(
        plan.changed_streams(),
        [StreamId(1), StreamId(count), StreamId(count + 1)]
    );
    let added = plan.to_vec().expect("output");
    assert_eq!(differing_streams(&source, &added), [1, count, count + 1]);
    let edited = Pdb::parse(added.as_slice()).expect("edited");
    assert_eq!(
        edited.named_stream_bytes(b"srcsrv").expect("read"),
        Some(second.to_vec())
    );
    assert_eq!(
        edited.named_stream_bytes(b"raw").expect("read"),
        Some(b"raw bytes".to_vec())
    );
    // Remove the raw stream again: its stream becomes nil.
    let mut editor = edited.edit(IdentityPolicy::Preserve);
    editor.remove_named_stream(b"raw").expect("remove");
    let removed = editor
        .plan(EditOptions::default())
        .expect("plan")
        .to_vec()
        .expect("output");
    let removed = Pdb::parse(removed.as_slice()).expect("removed");
    assert_eq!(removed.info().named_stream(b"raw"), None);
    assert!(stream(&removed, count + 1).is_none());
    assert_eq!(stream(&removed, count), stream(&edited, count));
}

#[test]
fn named_stream_errors_come_before_output() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    for name in [
        b"/names".as_slice(),
        b"/TMCache",
        b"/UDTSRCLINEUNDONE",
        b"/src/headerblock",
        b"/src/files/a.natvis",
    ] {
        let error = editor
            .set_named_stream(name, Vec::new())
            .expect_err("rejected name");
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    }
    let error = editor
        .set_named_stream(b"", Vec::new())
        .expect_err("empty name");
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    // The decoder of the kind checks the bytes when the plan runs.
    editor
        .set_named_stream(b"srcsrv", b"not a source server stream".to_vec())
        .expect("recorded");
    let error = editor
        .plan(EditOptions::default())
        .expect_err("invalid srcsrv");
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.remove_named_stream(b"absent").expect("recorded");
    let error = editor
        .plan(EditOptions::default())
        .expect_err("absent name");
    assert_eq!(error.kind(), ErrorKind::MissingStream);
}

#[test]
fn added_symbols_are_indexed_without_moving_records() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let Identity::Modern { age, .. } = pdb.info().identity() else {
        panic!("modern identity");
    };
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .add_public(&public(
            PublicFlags(0),
            SectionOffset {
                section: 2,
                offset: 0,
            },
            b"g_added",
        ))
        .expect("public");
    let mut data = Data::default();
    data.type_index = T_INT4;
    data.address = SectionOffset {
        section: 2,
        offset: 0,
    };
    data.name = Bytes::new(b"g_added");
    editor
        .add_global(S_GDATA32, &SymbolData::Data(data))
        .expect("global");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.identity().age(), age + 1);
    let edited = plan.to_vec().expect("output");
    let symbols = symbols(&edited);
    assert!(has(&symbols.publics, b"g_added"));
    assert!(has(&symbols.globals, b"g_added"));
    assert_eq!(symbols.publics.len(), 4);
    let edited_pdb = Pdb::parse(edited.as_slice()).expect("edited");
    let dbi = edited_pdb.debug_info().expect("DBI").expect("DBI present");
    // Both ages change; the module stream keeps its bytes.
    assert_eq!(dbi.header().age, age + 1);
    assert_eq!(edited_pdb.info().identity().age(), age + 1);
    let header = dbi.header();
    let mut expected = vec![1, 3];
    expected.extend(
        [
            header.global_stream,
            header.public_stream,
            header.symbol_stream,
        ]
        .map(|id| id.expect("index stream").0),
    );
    expected.sort_unstable();
    assert_eq!(differing_streams(&source, &edited), expected);
}

#[test]
fn renames_move_records_and_rewrite_the_indexes() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .rename_public(b"sum", b"sum_with_a_longer_name")
        .expect("public");
    editor
        .rename_global(b"g_value", b"g_value_with_a_longer_name")
        .expect("global");
    let edited = editor
        .plan(EditOptions::default())
        .expect("plan")
        .to_vec()
        .expect("output");
    let symbols = symbols(&edited);
    assert!(has(&symbols.publics, b"sum_with_a_longer_name"));
    assert!(!has(&symbols.publics, b"sum"));
    assert!(has(&symbols.globals, b"g_value_with_a_longer_name"));
    assert!(!has(&symbols.globals, b"g_value"));
    // The references still name the procedures in the module stream.
    assert!(has(&symbols.globals, b"sum"));
    assert!(has(&symbols.publics, b"main"));
}

#[test]
fn rename_errors_come_before_output() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let cases: [(&[u8], bool, ErrorKind); 3] = [
        (b"absent", false, ErrorKind::InvalidReference),
        // An S_PROCREF record: the module record has the same name.
        (b"sum", true, ErrorKind::UnsupportedEdit),
        (b"absent", true, ErrorKind::InvalidReference),
    ];
    for (name, global, kind) in cases {
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        if global {
            editor.rename_global(name, b"new").expect("recorded");
        } else {
            editor.rename_public(name, b"new").expect("recorded");
        }
        assert_eq!(
            editor
                .plan(EditOptions::default())
                .expect_err("rejected")
                .kind(),
            kind
        );
    }
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.rename_public(b"sum", b"first").expect("recorded");
    editor.rename_public(b"sum", b"second").expect("recorded");
    assert_eq!(
        editor
            .plan(EditOptions::default())
            .expect_err("two renames")
            .kind(),
        ErrorKind::UnsupportedEdit
    );
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let error = editor
        .add_global(S_PROCREF, &SymbolData::ScopeEnd)
        .expect_err("unsupported kind");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    assert_eq!(
        editor
            .rename_public(b"a\0b", b"c")
            .expect_err("NUL byte")
            .kind(),
        ErrorKind::InvalidFormat
    );
}

/// Return a path in the temporary directory for this process and `name`.
#[cfg(feature = "std")]
fn temporary(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pdbrs-native-edit-{}-{name}", std::process::id()))
}

#[test]
fn opaque_stream_rejects_moved_records_and_writes_no_file() {
    // A stream that no other stream names: its role is not known.
    let source = build(|builder| builder.add_stream(vec![1, 2, 3, 4]).map(|_| ()));
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .rename_public(b"sum", b"sum_with_a_longer_name")
        .expect("recorded");
    let error = editor
        .plan(EditOptions::default())
        .expect_err("opaque stream");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    assert!(error.to_string().contains("no other stream names"));
    #[cfg(feature = "std")]
    {
        let path = temporary("opaque.pdb");
        let result = editor
            .plan(EditOptions::default())
            .and_then(|plan| plan.write_file(&path));
        assert_eq!(
            result.expect_err("opaque stream").kind(),
            ErrorKind::UnsupportedEdit
        );
        assert!(!path.exists());
    }
    // A rename that keeps the record size moves no record.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.rename_public(b"sum", b"add").expect("recorded");
    editor.plan(EditOptions::default()).expect("same size");
    // An added record moves no record either.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .add_public(&public(
            PublicFlags(0),
            SectionOffset {
                section: 2,
                offset: 0,
            },
            b"g_added",
        ))
        .expect("recorded");
    editor
        .plan(EditOptions::default())
        .expect("appended record");
}

#[test]
fn unknown_symbol_kind_rejects_moved_records() {
    // A record of a kind that the decoder does not know (0x11fe).
    let source = build(|builder| {
        builder
            .add_symbol_with(|_, out| {
                out.extend_from_slice(&[6, 0, 0xfe, 0x11, 0, 0, 0, 0]);
                Ok(())
            })
            .map(|_| ())
    });
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .rename_public(b"sum", b"sum_with_a_longer_name")
        .expect("recorded");
    let error = editor
        .plan(EditOptions::default())
        .expect_err("unknown kind");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    assert!(error.to_string().contains("unknown kind"));
    // An added record moves no record, so the edit needs no check.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .add_public(&public(
            PublicFlags(0),
            SectionOffset {
                section: 2,
                offset: 0,
            },
            b"g_added",
        ))
        .expect("recorded");
    editor
        .plan(EditOptions::default())
        .expect("appended record");
}

/// Return a numeric leaf with an unsigned value.
fn number(value: u32) -> Numeric<'static> {
    Numeric::from_integer(Integer::Unsigned(u128::from(value)))
}

/// Return a forward reference to a structure with the name `name`.
fn forward(name: &[u8]) -> TypeLeaf<'_> {
    let mut class = Class::new(ClassKind::Structure);
    class.member_count = 0;
    class.options = ClassOptions::FORWARD_REFERENCE;
    class.field_list = TypeIndex::NONE;
    class.derivation_list = TypeIndex::NONE;
    class.vtable_shape = TypeIndex::NONE;
    class.size = number(0);
    class.name = Bytes::new(name);
    class.unique_name = None;
    TypeLeaf::Class(class)
}

#[test]
fn added_types_get_the_next_indices_and_hash_values() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let tpi = pdb.types().expect("TPI").expect("TPI present");
    let ipi = pdb.ids().expect("IPI").expect("IPI present");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let added = editor.add_type(&forward(b"Added")).expect("type");
    assert_eq!(added, TypeIndex(tpi.header().index_end));
    let id = editor
        .add_id(&IdRecord::String({
            let mut string = StringId::default();
            string.substrings = None;
            string.string = Bytes::new(b"added string");
            string
        }))
        .expect("ID");
    assert_eq!(id, IdIndex(ipi.header().index_end));
    editor
        .add_global(S_UDT, &udt(added, b"Added"))
        .expect("UDT");
    let edited = editor
        .plan(EditOptions::default())
        .expect("plan")
        .to_vec()
        .expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("edited");
    for (stream, end) in [
        (edited.types(), tpi.header().index_end + 1),
        (edited.ids(), ipi.header().index_end + 1),
    ] {
        let stream = stream.expect("stream").expect("present");
        assert_eq!(stream.header().index_end, end);
        let id = stream.header().hash_stream.expect("hash stream");
        let hash = edited.stream_bytes(id).expect("hash bytes");
        TypeHashes::parse(stream.header(), &hash)
            .and_then(|hashes| hashes.check(&stream))
            .expect("hash values");
    }
    let types = edited.types().expect("TPI").expect("TPI present");
    let last = types.records().last().expect("record").expect("framed");
    assert_eq!(
        TypeLeaf::decode(last.record).expect("decoded"),
        forward(b"Added")
    );
    // The source records keep their bytes.
    assert!(types.record_bytes().starts_with(tpi.record_bytes()));
}

#[test]
fn invalid_type_references_come_before_output() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let end = pdb.types().expect("TPI").expect("TPI").header().index_end;
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let TypeLeaf::Class(mut broken) = forward(b"Broken") else {
        unreachable!()
    };
    broken.options = ClassOptions(0);
    broken.field_list = TypeIndex(end + 10);
    editor.add_type(&TypeLeaf::Class(broken)).expect("recorded");
    assert_eq!(
        editor
            .plan(EditOptions::default())
            .expect_err("reference")
            .kind(),
        ErrorKind::InvalidReference
    );
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .add_global(S_UDT, &udt(TypeIndex(end), b"Missing"))
        .expect("recorded");
    assert_eq!(
        editor
            .plan(EditOptions::default())
            .expect_err("global reference")
            .kind(),
        ErrorKind::InvalidReference
    );
}

#[test]
fn identity_replace_writes_both_ages() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let guid = *b"edited-identity!";
    let plan = pdb
        .edit(IdentityPolicy::Replace {
            signature: 0x1234,
            guid,
            age: 9,
        })
        .plan(EditOptions::default())
        .expect("plan");
    assert_eq!(plan.identity(), Identity::Modern { guid, age: 9 });
    assert_eq!(plan.changed_streams(), [StreamId(1), StreamId(3)]);
    let edited = plan.to_vec().expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("edited");
    assert_eq!(edited.info().identity(), Identity::Modern { guid, age: 9 });
    assert_eq!(edited.info().signature(), 0x1234);
    let dbi = edited.debug_info().expect("DBI").expect("DBI present");
    assert_eq!(dbi.header().age, 9);
}

#[test]
fn added_public_records_decode_as_public_symbols() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let public = public(
        PublicFlags(PublicFlags::FUNCTION.0),
        SectionOffset {
            section: 1,
            offset: 0x10,
        },
        b"inner",
    );
    editor.add_public(&public).expect("public");
    let edited = editor
        .plan(EditOptions::default())
        .expect("plan")
        .to_vec()
        .expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("edited");
    let dbi = edited.debug_info().expect("DBI").expect("DBI present");
    let records = edited
        .stream_bytes(dbi.header().symbol_stream.expect("symbols"))
        .expect("bytes");
    let last = Records::new(&records)
        .last()
        .expect("record")
        .expect("framed");
    assert_eq!(SymbolKind(last.kind), S_PUB32);
    assert_eq!(
        *Symbol::decode(last).expect("decoded").data(),
        SymbolData::Public(public)
    );
}

/// A type map stream (version 2) that no other stream names: the bytes of
/// stream 61 of the `msvc-x64-incremental` fixture.
const UNOWNED_TYPE_MAP: [u8; 40] = [
    2, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 2, 0, 0, 0, 0, 0x10, 0, 0, 3, 0x10, 0,
    0, 0, 0, 0, 0, 8, 0, 0, 0, 0x18, 0, 0, 0,
];

#[test]
fn unowned_type_map_does_not_block_moved_records() {
    let source = build(|builder| builder.add_stream(UNOWNED_TYPE_MAP.to_vec()).map(|_| ()));
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .rename_public(b"sum", b"sum_with_a_longer_name")
        .expect("recorded");
    let edited = editor
        .plan(EditOptions::default())
        .expect("type map")
        .to_vec()
        .expect("output");
    let symbols = symbols(&edited);
    assert!(has(&symbols.publics, b"sum_with_a_longer_name"));
    // The type map keeps its bytes.
    let count = pdb.container().stream_count();
    let edited = Pdb::parse(edited.as_slice()).expect("edited");
    assert_eq!(stream(&edited, count - 1), Some(UNOWNED_TYPE_MAP.to_vec()));
}
