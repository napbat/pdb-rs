//! The type records of the modules of a PDB without type merging (`NOTM`, #30).
//!
//! MSVC `link /DEBUG:LAZY` leaves TPI and IPI empty. The `S_MOD_TYPEREF` record of each module
//! names the streams with the records of its input object. The example decodes every record of
//! these streams, claims the streams, and checks the module symbols against them.

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::codeview::id_record::{IdRecord, IdTable};
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::symbol::{ModuleTypeReference, ModuleTypeSource};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::dbi::{Dbi, ModuleStream};
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::{ForwardRefs, TypeStream, TypeTable, type_size};

use crate::{Context, Outcome, Report};

/// The first and the last kind of an ID record (`LF_FUNC_ID` to `LF_UDT_MOD_SRC_LINE`).
const ID_KINDS: core::ops::RangeInclusive<u16> = 0x1601..=0x1607;

/// The type records of one module.
pub(crate) struct ModuleTypeStreams {
    /// The type records, or the shared records of a `/Z7` object.
    types: TypeStream,
    /// The ID records of a `/Zi` type server; `None` if they share `types`.
    ids: Option<TypeStream>,
    /// `false` if the indices refer to precompiled types outside the PDB, so that the example
    /// cannot check them.
    checked: bool,
}

/// Return the source of the type records of a module stream.
fn source(stream: &ModuleStream) -> Outcome<Option<ModuleTypeSource>> {
    Ok(ModuleTypeReference::find(stream.symbol_data())?.map(|reference| reference.source()))
}

/// Read one stream of module type records, and decode its records once.
fn read(pdb: &Pdb<impl ReadAt>, id: StreamId, report: &mut Report) -> Outcome<TypeStream> {
    let stream = TypeStream::from_records(&pdb.stream_bytes(id)?)?;
    if report.claimed.insert(id.0) {
        for record in stream.records() {
            let record = record?.record;
            if ID_KINDS.contains(&record.kind) {
                if let IdRecord::Unsupported(_) = IdRecord::decode(record)? {
                    report.undecoded(format!("module ID record {:#06x}", record.kind));
                }
            } else if let TypeLeaf::Unsupported(_) = TypeLeaf::decode(record)? {
                report.undecoded(format!("module type record {:#06x}", record.kind));
            }
            report.add("module type records decoded", 1);
        }
        report.add("module type record streams", 1);
    }
    Ok(stream)
}

/// Return the type records of a module, if its `S_MOD_TYPEREF` record names them.
///
/// A module that uses the records of another module (`fRefTM`) gets the records of that module.
pub(crate) fn streams(
    pdb: &Pdb<impl ReadAt>,
    dbi: &Dbi,
    stream: &ModuleStream,
    report: &mut Report,
) -> Outcome<Option<ModuleTypeStreams>> {
    let mut source = source(stream)?;
    if let Some(ModuleTypeSource::OtherModule { module }) = source {
        let owner = dbi
            .modules()
            .nth(usize::try_from(module.0)?)
            .ok_or("S_MOD_TYPEREF names a missing module")??;
        let owner = pdb
            .module_stream(&owner)?
            .ok_or("S_MOD_TYPEREF names a module without a stream")?;
        source = self::source(&owner)?;
        report.add("modules with the type records of another module", 1);
    }
    let streams = match source {
        None => return Ok(None),
        Some(ModuleTypeSource::NoTypes) => ModuleTypeStreams {
            types: TypeStream::from_records(&[])?,
            ids: None,
            checked: true,
        },
        Some(ModuleTypeSource::TypeRecords { stream }) => ModuleTypeStreams {
            types: read(pdb, stream, report)?,
            ids: None,
            checked: true,
        },
        Some(ModuleTypeSource::TypeServer { types, ids }) => ModuleTypeStreams {
            types: read(pdb, types, report)?,
            ids: Some(read(pdb, ids, report)?),
            checked: true,
        },
        // `/DEBUG:LAZY` with `/Z7` precompiled headers: the own records are in the PDB, the
        // precompiled types stay in the object file of the owner module (#30, #39).
        Some(
            ModuleTypeSource::PrecompiledTypeOwner { stream }
            | ModuleTypeSource::PrecompiledTypeUser { stream, .. },
        ) => {
            report.add("modules with precompiled types in an object file", 1);
            return Ok(Some(ModuleTypeStreams {
                types: read(pdb, stream, report)?,
                ids: None,
                checked: false,
            }));
        }
        Some(_) => {
            report.undecoded("S_MOD_TYPEREF record without a known source".to_owned());
            return Ok(None);
        }
    };
    report.add("modules checked against S_MOD_TYPEREF type records", 1);
    Ok(Some(streams))
}

/// Run `check` with a context whose type and ID indices resolve against `types`.
///
/// If the indices refer to precompiled types outside the PDB, the context does not check them.
pub(crate) fn with_context<R>(
    types: &ModuleTypeStreams,
    context: &Context<'_>,
    check: impl FnOnce(&Context<'_>) -> Outcome<R>,
) -> Outcome<R> {
    let ids = IdTable::new(types.ids.as_ref().unwrap_or(&types.types))?;
    let table = TypeTable::new(&types.types)?;
    let forward = ForwardRefs::new(&table, None)?;
    // The sizes of an unchecked module would come from records with an unknown first index.
    let size_of_type = |index| {
        types
            .checked
            .then(|| type_size(&table, Some(&forward), index).ok().flatten())
            .flatten()
    };
    let module_context = Context {
        names: context.names,
        tpi_end: if types.checked {
            TypeIndex(types.types.header().index_end)
        } else {
            TypeIndex(u32::MAX)
        },
        indices_checked: types.checked,
        ids,
        addresses: context.addresses,
        module_count: context.module_count,
        type_size: &size_of_type,
    };
    check(&module_context)
}
