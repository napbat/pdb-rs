//! The edits that `verify-native-edit` applies to each native PDB.
//!
//! Each [`Edit`] uses the `pdbrs::native::PdbEditor` API and returns the edited PDB, the
//! streams that the plan changed and the values that the consumer lookups must find.

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::field_list::{
    DataMember, LLVM_FIELD_LIST_LIMIT, Member, encode_field_list,
};
use pdbrs::native::codeview::id_record::{IdRecord, StringId};
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    Data, Public, S_CONSTANT, S_GDATA32, S_GTHREAD32, S_LDATA32, S_LTHREAD32, S_UDT, SymbolData,
    UserDefinedType,
};
use pdbrs::native::codeview::type_record::{
    Class, ClassKind, ClassOptions, MemberAttributes, TypeLeaf,
};
use pdbrs::native::symbols::{HashTable, indexed_name};
use pdbrs::native::{EditOptions, Identity, IdentityPolicy, Pdb, PdbEditor};

/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);

/// The suffix that the rename edit appends to a name. The record gets longer, so the offsets
/// of the later records move.
pub(crate) const RENAME_SUFFIX: &str = "_pdbrs_renamed";

/// The name of the public symbol that the symbol edit adds.
pub(crate) const ADDED_PUBLIC: &str = "pdbrs_edit_public";

/// The name of the global data symbol that the symbol edit adds.
pub(crate) const ADDED_GLOBAL: &str = "pdbrs_edit_global";

/// The name of the structure that the type edit adds.
pub(crate) const ADDED_TYPE: &str = "PdbrsEditType";

/// The string of the `LF_STRING_ID` record that the type edit adds.
pub(crate) const ADDED_ID: &str = "pdbrs edit string";

/// The name of the raw named stream that the named stream edit adds.
pub(crate) const RAW_STREAM: &str = "pdbrs-edit";

/// The `srcsrv` stream that the named stream edit writes.
pub(crate) const SOURCE_SERVER: &str = "SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
VERCTRL=pdbrs\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=%targ%\\%var2%\r\n\
SRCSRV: source files ---------------------------------------\r\n\
C:\\pdbrs\\edit.cpp*edit.cpp\r\n\
SRCSRV: end ------------------------------------------------\r\n";

/// The `sourcelink` stream that the named stream edit writes.
pub(crate) const SOURCE_LINK: &str =
    r#"{"documents":{"C:\\pdbrs\\*":"https://example.invalid/pdbrs/*"}}"#;

/// The identity that the identity edit writes.
pub(crate) const NEW_SIGNATURE: u32 = 0x5244_4250;
/// The GUID that the identity edit writes.
pub(crate) const NEW_GUID: [u8; 16] = *b"pdbrs-edit-guid!";
/// The age that the identity edit writes.
pub(crate) const NEW_AGE: u32 = 7;

/// One edit of the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edit {
    /// Add `srcsrv`, `sourcelink` and a raw stream (or replace them), and remove `/LinkInfo`.
    Named,
    /// Add a public symbol and a global data symbol.
    AddSymbols,
    /// Rename a public symbol and a global symbol. The later records move.
    RenameSymbols,
    /// Add a structure, its field list, an `S_UDT` global and an `LF_STRING_ID` record.
    Types,
    /// Replace the identity.
    Identity,
    /// Strip the private symbols.
    Strip,
}

impl Edit {
    /// All edits in the order of the command.
    pub(crate) const ALL: [Self; 6] = [
        Self::Named,
        Self::AddSymbols,
        Self::RenameSymbols,
        Self::Types,
        Self::Identity,
        Self::Strip,
    ];

    /// The name of the edit in the output and in file names.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::AddSymbols => "add-symbols",
            Self::RenameSymbols => "rename-symbols",
            Self::Types => "types",
            Self::Identity => "identity",
            Self::Strip => "strip",
        }
    }
}

/// The values that the consumer lookups must find in the edited PDB.
#[derive(Debug, Default)]
pub(crate) struct Expectation {
    /// The named streams that must be present.
    pub named_present: Vec<String>,
    /// The named streams that must be absent.
    pub named_absent: Vec<String>,
    /// The `srcsrv` stream that `pdbstr -r` must read.
    pub source_server: Option<String>,
    /// The public symbols that must be present.
    pub publics: Vec<String>,
    /// The public symbols that must be absent.
    pub publics_absent: Vec<String>,
    /// The global symbols that must be present.
    pub globals: Vec<String>,
    /// The global symbols that must be absent.
    pub globals_absent: Vec<String>,
    /// The type names that must be present.
    pub types: Vec<String>,
    /// The `LF_STRING_ID` strings that must be present.
    pub ids: Vec<String>,
    /// The identity that the summary must show: signature, GUID and age.
    pub identity: Option<(u32, [u8; 16], u32)>,
}

/// An edited PDB.
#[derive(Debug)]
pub(crate) struct Edited {
    /// The bytes of the edited PDB.
    pub bytes: Vec<u8>,
    /// The streams that the plan changed, removed or added.
    pub changed: Vec<u32>,
    /// The identity for the image.
    pub identity: Identity,
    /// The values that the lookups must find.
    pub expect: Expectation,
    /// The parts of the edit that the PDB does not allow, for example a rename without a
    /// global record of a supported kind.
    pub notes: Vec<String>,
}

/// The result of one edit.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// The plan accepted the edit.
    Edited(Box<Edited>),
    /// The plan rejected the edit before output.
    Rejected(pdbrs::Error),
}

/// Apply `edit` to the PDB `source`.
///
/// A plan error gives [`Outcome::Rejected`]. Other errors, for example an error that reads
/// the source before the edit, give `Err`.
pub(crate) fn apply(edit: Edit, source: &[u8]) -> Result<Outcome, String> {
    let pdb = Pdb::parse(source).map_err(|error| format!("cannot read the PDB: {error}"))?;
    let mut expect = Expectation::default();
    let mut notes = Vec::new();
    let identity = match edit {
        Edit::Named | Edit::Strip => IdentityPolicy::Preserve,
        Edit::Identity => IdentityPolicy::Replace {
            signature: NEW_SIGNATURE,
            guid: NEW_GUID,
            age: NEW_AGE,
        },
        Edit::AddSymbols | Edit::RenameSymbols | Edit::Types => IdentityPolicy::IncrementAge,
    };
    let mut editor = pdb.edit(identity);
    let prepared = match edit {
        Edit::Named => named(&pdb, &mut editor, &mut expect),
        Edit::AddSymbols => add_symbols(&pdb, &mut editor, &mut expect),
        Edit::RenameSymbols => rename_symbols(&pdb, &mut editor, &mut expect, &mut notes),
        Edit::Types => types(&pdb, &mut editor, &mut expect, &mut notes),
        Edit::Identity => {
            expect.identity = Some((NEW_SIGNATURE, NEW_GUID, NEW_AGE));
            Ok(())
        }
        Edit::Strip => {
            editor.strip_private();
            Ok(())
        }
    };
    prepared.map_err(|error| format!("cannot prepare the edit: {error}"))?;
    let plan = match editor.plan(EditOptions::default()) {
        Ok(plan) => plan,
        Err(error) => return Ok(Outcome::Rejected(error)),
    };
    let bytes = plan
        .to_vec()
        .map_err(|error| format!("cannot write the edited PDB: {error}"))?;
    Ok(Outcome::Edited(Box::new(Edited {
        bytes,
        changed: plan.changed_streams().iter().map(|id| id.0).collect(),
        identity: plan.identity(),
        expect,
        notes,
    })))
}

/// Add or replace `srcsrv`, `sourcelink` and a raw stream, and remove `/LinkInfo`.
fn named(
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
    expect: &mut Expectation,
) -> pdbrs::Result<()> {
    editor.set_named_stream(b"srcsrv", SOURCE_SERVER.as_bytes().to_vec())?;
    editor.set_named_stream(b"sourcelink", SOURCE_LINK.as_bytes().to_vec())?;
    editor.set_named_stream(RAW_STREAM.as_bytes(), b"pdbrs raw stream".to_vec())?;
    expect.named_present = vec!["srcsrv".into(), "sourcelink".into(), RAW_STREAM.into()];
    expect.source_server = Some(SOURCE_SERVER.to_owned());
    if pdb.info().named_stream(b"/LinkInfo").is_some() {
        editor.remove_named_stream(b"/LinkInfo")?;
        expect.named_absent.push("/LinkInfo".into());
    }
    Ok(())
}

/// The symbol record stream and the indexes of a PDB.
struct Indexes {
    /// The symbol record stream.
    records: Vec<u8>,
    /// The global index stream bytes.
    globals: Option<pdbrs::native::symbols::GlobalSymbolIndex>,
    /// The public index stream bytes.
    publics: Option<pdbrs::native::symbols::PublicSymbolIndex>,
}

/// Read the symbol record stream and the indexes.
fn indexes(pdb: &Pdb<&[u8]>) -> pdbrs::Result<Indexes> {
    let dbi = pdb.debug_info()?.ok_or_else(|| {
        pdbrs::Error::new(pdbrs::ErrorKind::MissingStream, None).with_context("PDB has no DBI")
    })?;
    let records = match dbi.header().symbol_stream {
        Some(id) => pdb.stream_bytes(id)?,
        None => Vec::new(),
    };
    Ok(Indexes {
        records,
        globals: pdb.global_symbols(&dbi)?,
        publics: pdb.public_symbols(&dbi)?,
    })
}

/// Return the record with the smallest offset in `table` that `select` accepts and whose
/// name is unique in the table: its name and its data.
///
/// A short name of letters, digits and `_` comes before other names, so that the output
/// stays readable.
fn first_unique<'r>(
    records: &'r [u8],
    table: HashTable<'_>,
    select: impl Fn(pdbrs::native::codeview::SymbolKind) -> bool,
) -> pdbrs::Result<Option<(&'r [u8], SymbolData<'r>)>> {
    let mut candidates = Vec::new();
    let mut names = std::collections::BTreeMap::<&[u8], usize>::new();
    for record in table.records() {
        let name = indexed_name(records, record.symbol)?;
        *names.entry(name).or_default() += 1;
        let symbol =
            pdbrs::native::codeview::symbol::Symbol::decode(record.symbol.record(records)?)?;
        if select(symbol.kind()) && !name.is_empty() && name.is_ascii() {
            let plain = name.len() <= 40
                && name
                    .iter()
                    .all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_');
            candidates.push((!plain, record.symbol.0, name, *symbol.data()));
        }
    }
    candidates.sort_by_key(|(other, offset, _, _)| (*other, *offset));
    Ok(candidates
        .into_iter()
        .find(|(_, _, name, _)| names.get(name) == Some(&1))
        .map(|(_, _, name, data)| (name, data)))
}

/// Add a public symbol and a global data symbol at the address of the first public symbol.
fn add_symbols(
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
    expect: &mut Expectation,
) -> pdbrs::Result<()> {
    let indexes = indexes(pdb)?;
    let mut address = SectionOffset {
        section: 1,
        offset: 0,
    };
    let mut flags = pdbrs::native::codeview::symbol::PublicFlags(0);
    if let Some(publics) = &indexes.publics
        && let Some((_, SymbolData::Public(public))) =
            first_unique(&indexes.records, publics.hash_table(), |_| true)?
    {
        address = public.address;
        flags = public.flags;
    }
    let mut public = Public::default();
    public.flags = flags;
    public.address = address;
    public.name = ADDED_PUBLIC.as_bytes();
    editor.add_public(&public)?;
    let mut data = Data::default();
    data.type_index = T_INT4;
    data.address = address;
    data.name = ADDED_GLOBAL.as_bytes();
    editor.add_global(S_GDATA32, &SymbolData::Data(data))?;
    expect.publics.push(ADDED_PUBLIC.into());
    expect.globals.push(ADDED_GLOBAL.into());
    Ok(())
}

/// Rename the first unique public symbol and the first unique global data, constant or UDT
/// symbol.
fn rename_symbols(
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
    expect: &mut Expectation,
    notes: &mut Vec<String>,
) -> pdbrs::Result<()> {
    let indexes = indexes(pdb)?;
    let public = match &indexes.publics {
        Some(publics) => first_unique(&indexes.records, publics.hash_table(), |_| true)?,
        None => None,
    };
    if let Some((name, _)) = public {
        let new = [name, RENAME_SUFFIX.as_bytes()].concat();
        editor.rename_public(name, &new)?;
        expect.publics_absent.push(lossy(name));
        expect.publics.push(lossy(&new));
    } else {
        notes.push("no public symbol to rename".into());
    }
    let global = match &indexes.globals {
        Some(globals) => first_unique(&indexes.records, globals.hash_table(), |kind| {
            matches!(
                kind,
                S_GDATA32 | S_LDATA32 | S_GTHREAD32 | S_LTHREAD32 | S_CONSTANT | S_UDT
            )
        })?,
        None => None,
    };
    if let Some((name, _)) = global {
        let new = [name, RENAME_SUFFIX.as_bytes()].concat();
        editor.rename_global(name, &new)?;
        expect.globals_absent.push(lossy(name));
        expect.globals.push(lossy(&new));
    } else {
        notes.push("no global data, constant or UDT symbol to rename".into());
    }
    Ok(())
}

/// Return the text of a name.
fn lossy(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// Return a numeric leaf with an unsigned value.
fn number(value: u32) -> Numeric<'static> {
    Numeric::from_integer(Integer::Unsigned(u128::from(value)))
}

/// Add a field list, a structure, an `S_UDT` global and an `LF_STRING_ID` record.
fn types(
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
    expect: &mut Expectation,
    notes: &mut Vec<String>,
) -> pdbrs::Result<()> {
    let mut member = DataMember::default();
    member.attributes = MemberAttributes::from_raw(3)?;
    member.member_type = T_INT4;
    member.offset = number(0);
    member.name = b"value";
    let members = [Member::DataMember(member)];
    let mut bytes = Vec::new();
    encode_field_list(
        &members,
        TypeIndex(0x1000),
        LLVM_FIELD_LIST_LIMIT,
        &mut bytes,
    )?;
    let record = Records::new(&bytes).next().ok_or_else(|| {
        pdbrs::Error::new(pdbrs::ErrorKind::InvalidFormat, None)
            .with_context("field list has no record")
    })??;
    let field_list = editor.add_type(&TypeLeaf::decode(record)?)?;
    let mut class = Class::new(ClassKind::Structure);
    class.member_count = 1;
    class.options = ClassOptions(0);
    class.field_list = field_list;
    class.derivation_list = TypeIndex::NONE;
    class.vtable_shape = TypeIndex::NONE;
    class.size = number(4);
    class.name = ADDED_TYPE.as_bytes();
    class.unique_name = None;
    class.trailing = &[];
    let structure = editor.add_type(&TypeLeaf::Class(class))?;
    let mut udt = UserDefinedType::default();
    udt.type_index = structure;
    udt.name = ADDED_TYPE.as_bytes();
    editor.add_global(S_UDT, &SymbolData::UserDefinedType(udt))?;
    expect.types.push(ADDED_TYPE.into());
    expect.globals.push(ADDED_TYPE.into());
    let has_ipi = pdb
        .container()
        .stream(pdbrs::native::msf::StreamId(4))?
        .is_some_and(|stream| pdbrs::io::ReadAt::len(&stream).is_ok_and(|length| length != 0));
    if has_ipi {
        let mut id = StringId::default();
        id.substrings = None;
        id.string = ADDED_ID.as_bytes();
        editor.add_id(&IdRecord::String(id))?;
        expect.ids.push(ADDED_ID.into());
    } else {
        notes.push("the PDB has no IPI stream".into());
    }
    Ok(())
}
