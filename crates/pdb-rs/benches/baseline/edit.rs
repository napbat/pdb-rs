//! The write scenarios of native PDBs: the edits, the rebuild and the MSFZ write.
//!
//! Each scenario reads the file before the timed runs. Each timed run makes the change, plans
//! the output and writes it to a `Vec<u8>`. The count is the number of output bytes.

use std::collections::BTreeMap;
use std::path::Path;

use pdb_rs::native::codeview::Records;
use pdb_rs::native::codeview::field_list::{
    DataMember, LLVM_FIELD_LIST_LIMIT, Member, encode_field_list,
};
use pdb_rs::native::codeview::index::{SymbolHandle, TypeIndex};
use pdb_rs::native::codeview::numeric::{Integer, Numeric};
use pdb_rs::native::codeview::type_record::{
    Class, ClassKind, ClassOptions, MemberAttributes, TypeLeaf,
};
use pdb_rs::native::container::Container;
use pdb_rs::native::locate::SymbolLocator;
use pdb_rs::native::msf::OldDirectoryPolicy;
use pdb_rs::native::msfz::MsfzWriteOptions;
use pdb_rs::native::symbols::indexed_name;
use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};
use pdb_rs::{Bytes, ErrorKind};

use crate::measure::Bench;
use crate::{Outcome, lookup};

/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);

/// The `srcsrv` stream that the named stream edit writes.
const SOURCE_SERVER: &str = "SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
VERCTRL=pdb-rs\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=%targ%\\%var2%\r\n\
SRCSRV: source files ---------------------------------------\r\n\
C:\\pdb-rs\\edit.cpp*edit.cpp\r\n\
SRCSRV: end ------------------------------------------------\r\n";

/// The suffix that the rename edit appends to the name.
const RENAME_SUFFIX: &[u8] = b"_pdb_rs_renamed";

/// The name of the structure that the type edit adds.
const ADDED_TYPE: &[u8] = b"pdb_rs_bench_type";

/// Plan the edits of `editor` with `options` and write the output.
fn write(editor: &PdbEditor<'_, &[u8]>, options: EditOptions) -> Outcome<u64> {
    let bytes = editor.plan(options)?.to_vec()?;
    Ok(u64::try_from(bytes.len())?)
}

/// Set the `srcsrv` named stream.
pub(crate) fn named_stream(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor
            .named_streams_mut()?
            .insert(b"srcsrv", SOURCE_SERVER.as_bytes().to_vec())?;
        write(&editor, EditOptions::default())
    })
}

/// Return the name of the public symbol with the smallest record offset whose name is unique
/// in the public index.
fn unique_public(pdb: &Pdb<&[u8]>) -> Outcome<Vec<u8>> {
    let publics = pdb.publics()?.ok_or("missing public index")?;
    if publics.records().is_empty() {
        return Err("missing symbol record stream".into());
    }
    let mut names = BTreeMap::<&[u8], (usize, u32)>::new();
    for entry in publics.index().hash_table().records() {
        let name = indexed_name(publics.records(), entry.symbol)?;
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
    measure_rename(&bytes, bench)
}

/// Rename the first public symbol with a unique name in a rebuild of the fixture.
///
/// The rebuild happens before the timed runs. Its streams are contiguous, so a slice source
/// can lend them.
pub(crate) fn rebuilt_rename_public(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let rebuilt = {
        let pdb = Pdb::parse(&bytes)?;
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor.decode_all()?;
        editor.plan(EditOptions::default())?.to_vec()?
    };
    drop(bytes);
    measure_rename(&rebuilt, bench)
}

/// Measure the rename of the first public symbol with a unique name in the PDB `bytes`.
fn measure_rename(bytes: &[u8], bench: &mut Bench) -> Outcome {
    let pdb = Pdb::parse(bytes)?;
    let old = unique_public(&pdb)?;
    let new = [old.as_slice(), RENAME_SUFFIX].concat();
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        let mut publics = editor.publics_mut()?;
        let handle = publics.find(&old)?.ok_or("missing public symbol")?;
        let name = publics
            .get_mut(handle)
            .and_then(|record| record.data_mut().name_mut())
            .ok_or("public symbol without a name")?;
        *name = new.clone().into();
        write(&editor, EditOptions::default())
    })
}

/// Find each sample name of `native/symbol-by-name` with [`PdbEditor::publics_mut`] and
/// [`PdbEditor::globals_mut`].
///
/// The decode of the editor model happens before the timed runs. The count is the number of
/// found records. A name of more than one record counts as one.
pub(crate) fn editor_find_name(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let names = lookup::sample_names(&SymbolLocator::new(&pdb)?)?;
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.publics_mut()?;
    bench.measure(|| {
        let mut found = 0;
        for name in &names {
            found += u64::from(present(editor.publics_mut()?.find(name))?);
            found += u64::from(present(editor.globals_mut()?.find(name))?);
        }
        Ok(found)
    })
}

/// Return `true` if a find result has a record. A name of more than one record has records.
fn present(result: pdb_rs::Result<Option<SymbolHandle>>) -> Outcome<bool> {
    match result {
        Ok(handle) => Ok(handle.is_some()),
        Err(error) if error.kind() == ErrorKind::InvalidReference => Ok(true),
        Err(error) => Err(error.into()),
    }
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
    member.name = b"value".into();
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
        let field_list = editor.types_mut()?.push(list.clone())?;
        let mut class = Class::new(ClassKind::Structure);
        class.member_count = 1;
        class.options = ClassOptions(0);
        class.field_list = field_list;
        class.derivation_list = TypeIndex::NONE;
        class.vtable_shape = TypeIndex::NONE;
        class.size = number(4);
        class.name = ADDED_TYPE.into();
        class.unique_name = None;
        class.trailing = Bytes::new(&[]);
        editor.types_mut()?.push(TypeLeaf::Class(class))?;
        write(&editor, EditOptions::default())
    })
}

/// Strip the private symbols with [`EditOptions::with_strip_private`].
pub(crate) fn strip_private(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let editor = pdb.edit(IdentityPolicy::Preserve);
        write(&editor, EditOptions::default().with_strip_private(true))
    })
}

/// Rebuild the PDB from its decoded records, as the `native_rebuild` example does.
pub(crate) fn rebuild(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor.decode_all()?;
        write(&editor, EditOptions::default())
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
