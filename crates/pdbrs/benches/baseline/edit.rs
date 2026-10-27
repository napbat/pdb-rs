//! The write scenarios of native PDBs: the edits, the rebuild and the MSFZ write.
//!
//! Each scenario reads the file before the timed runs. Each timed run makes the change, plans
//! the output and writes it to a `Vec<u8>`. The count is the number of output bytes.

use std::collections::BTreeMap;
use std::path::Path;

use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::field_list::{
    DataMember, LLVM_FIELD_LIST_LIMIT, Member, encode_field_list,
};
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::type_record::{
    Class, ClassKind, ClassOptions, MemberAttributes, TypeLeaf,
};
use pdbrs::native::container::Container;
use pdbrs::native::msf::OldDirectoryPolicy;
use pdbrs::native::msfz::MsfzWriteOptions;
use pdbrs::native::symbols::indexed_name;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

use crate::Outcome;
use crate::measure::Bench;

/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);

/// The `srcsrv` stream that the named stream edit writes.
const SOURCE_SERVER: &str = "SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
VERCTRL=pdbrs\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=%targ%\\%var2%\r\n\
SRCSRV: source files ---------------------------------------\r\n\
C:\\pdbrs\\edit.cpp*edit.cpp\r\n\
SRCSRV: end ------------------------------------------------\r\n";

/// The suffix that the rename edit appends to the name.
const RENAME_SUFFIX: &[u8] = b"_pdbrs_renamed";

/// The name of the structure that the type edit adds.
const ADDED_TYPE: &[u8] = b"PdbrsBenchType";

/// Plan the edits of `editor` with the default options and write the output.
fn write(editor: &PdbEditor<'_, &[u8]>) -> Outcome<u64> {
    let bytes = editor.plan(EditOptions::default())?.to_vec()?;
    Ok(u64::try_from(bytes.len())?)
}

/// Set the `srcsrv` named stream.
pub(crate) fn named_stream(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor.set_named_stream(b"srcsrv", SOURCE_SERVER.as_bytes().to_vec())?;
        write(&editor)
    })
}

/// Return the name of the public symbol with the smallest record offset whose name is unique
/// in the public index.
fn unique_public(pdb: &Pdb<&[u8]>) -> Outcome<Vec<u8>> {
    let dbi = pdb.debug_info()?.ok_or("missing DBI stream")?;
    let id = dbi
        .header()
        .symbol_stream
        .ok_or("missing symbol record stream")?;
    let records = pdb.stream_bytes(id)?;
    let publics = pdb.public_symbols(&dbi)?.ok_or("missing public index")?;
    let mut names = BTreeMap::<&[u8], (usize, u32)>::new();
    for entry in publics.hash_table().records() {
        let name = indexed_name(&records, entry.symbol)?;
        let value = names.entry(name).or_insert((0, entry.symbol.0));
        value.0 += 1;
        value.1 = value.1.min(entry.symbol.0);
    }
    names
        .into_iter()
        .filter(|(name, (count, _))| *count == 1 && !name.is_empty())
        .min_by_key(|(_, (_, offset))| *offset)
        .map(|(name, _)| name.to_vec())
        .ok_or_else(|| "no public symbol with a unique name".into())
}

/// Rename the first public symbol with a unique name.
pub(crate) fn rename_public(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let old = unique_public(&pdb)?;
    let new = [old.as_slice(), RENAME_SUFFIX].concat();
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        editor.rename_public(&old, &new)?;
        write(&editor)
    })
}

/// Return a numeric leaf with an unsigned value.
fn number(value: u32) -> Numeric<'static> {
    Numeric::from_integer(Integer::Unsigned(u128::from(value)))
}

/// Return the bytes of a field list with one `int` member.
fn field_list() -> Outcome<Vec<u8>> {
    let mut member = DataMember::default();
    member.attributes = MemberAttributes::from_raw(3)?;
    member.member_type = T_INT4;
    member.offset = number(0);
    member.name = b"value";
    let mut bytes = Vec::new();
    encode_field_list(
        &[Member::DataMember(member)],
        TypeIndex(0x1000),
        LLVM_FIELD_LIST_LIMIT,
        &mut bytes,
    )?;
    Ok(bytes)
}

/// Append a field list and a structure to the TPI stream.
pub(crate) fn append_type(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let list = field_list()?;
    let record = Records::new(&list)
        .next()
        .ok_or("field list has no record")??;
    let list = TypeLeaf::decode(record)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        let field_list = editor.add_type(&list)?;
        let mut class = Class::new(ClassKind::Structure);
        class.member_count = 1;
        class.options = ClassOptions(0);
        class.field_list = field_list;
        class.derivation_list = TypeIndex::NONE;
        class.vtable_shape = TypeIndex::NONE;
        class.size = number(4);
        class.name = ADDED_TYPE;
        class.unique_name = None;
        class.trailing = &[];
        editor.add_type(&TypeLeaf::Class(class))?;
        write(&editor)
    })
}

/// Strip the private symbols with [`PdbEditor::strip_private`].
pub(crate) fn strip_private(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor.strip_private();
        write(&editor)
    })
}

/// Rebuild the PDB from its decoded records, as the `native_rebuild` example does.
pub(crate) fn rebuild(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let contents = pdb.contents()?;
        let options = contents.build_options();
        let builder = contents.to_builder(IdentityPolicy::Preserve)?;
        let output = builder.plan(options)?.to_vec()?;
        Ok(u64::try_from(output.len())?)
    })
}

/// Write the container as MSFZ with [`MsfzWriteOptions::default`].
///
/// The policy [`OldDirectoryPolicy::Empty`] is the policy of `verify-msfz` and of
/// `pdbtool pdz-encode`.
pub(crate) fn msfz_write(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let container = Container::read(bytes.as_slice())?;
    bench.measure(|| {
        let plan = container.to_msfz(MsfzWriteOptions::default(), OldDirectoryPolicy::Empty)?;
        Ok(u64::try_from(plan.to_vec()?.len())?)
    })
}
