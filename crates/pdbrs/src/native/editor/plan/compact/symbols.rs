//! The index fields of symbol records.
//!
//! [`symbol`] gives the fields of a symbol record with the output index in
//! each TPI and IPI index field. [`symbol_records`] writes the changed
//! fields into a sequence of framed records: the symbol record stream, the
//! symbol region of a module stream, or a `DEBUG_S_SYMBOLS` subsection.
//!
//! `Symbol::type_index` and `Symbol::id_index` tell the space of the index
//! of each kind. The managed kinds hold metadata tokens. A token names a row
//! of the metadata of an assembly outside the PDB, so it stays. A value with
//! a high byte of 0 can be a type index, and the rewrite then stops with an
//! error if the value would change.

use alloc::vec::Vec;

use super::map::IndexRemap;
use super::records::{Rewrite, first_record, id_list, unsupported, write_difference};
use crate::native::codeview::SymbolKind;
use crate::native::codeview::id_record::IdList;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, TypeIndex};
use crate::native::codeview::symbol::{
    FunctionList, ProcedureType, S_MANFRAMEREL, S_MANMANYREG, S_MANMANYREG2, S_MANREGISTER,
    S_MANREGREL, Symbol, SymbolData,
};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::{Error, Result};

/// The size of the length field and the kind field of a framed record.
const PREFIX_SIZE: usize = 4;

/// Return the fields `data` of a symbol record of `kind` with the output
/// index in each type index and ID index field, or `None` if no index
/// changes.
///
/// # Errors
///
/// - The errors of `map`, unchanged.
/// - [`ErrorKind::UnsupportedEdit`](crate::ErrorKind::UnsupportedEdit) for
///   a kind without a decoder, for `S_FASTLINK`, `S_REF_MINIPDB` and
///   `S_MOD_TYPEREF`, for an `S_DISCARDED` record with a discarded record,
///   and for a managed value that can be a changed type index.
/// - The errors of the constructors of the list views.
pub(in crate::native::editor::plan) fn symbol(
    kind: SymbolKind,
    data: &SymbolData<'_>,
    map: &impl IndexRemap,
) -> Result<Option<SymbolData<'static>>> {
    let mut rewrite = Rewrite::new(map);
    let mut data = data.clone();
    remap_symbol(kind, &mut data, &mut rewrite)?;
    Ok(rewrite.changed().then(|| data.into_owned()))
}

/// Write the output indices into the index fields of `data`.
fn remap_symbol<M: IndexRemap>(
    kind: SymbolKind,
    data: &mut SymbolData<'_>,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    match data {
        SymbolData::BuildInfo(index) | SymbolData::SourceLink(index) => rewrite.id_index(index),
        SymbolData::Procedure(value) => signature(&mut value.signature, rewrite),
        SymbolData::MipsProcedure(value) => signature(&mut value.signature, rewrite),
        SymbolData::Ia64Procedure(value) => signature(&mut value.signature, rewrite),
        SymbolData::InlineSite(value) => rewrite.id_index(&mut value.inlinee),
        SymbolData::Inlinees(list) => id_list(list, rewrite),
        SymbolData::Callees(list) | SymbolData::Callers(list) => function_list(list, rewrite),
        SymbolData::CallSiteInfo(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::HeapAllocationSite(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::HotPatchFunction(value) => rewrite.id_index(&mut value.function),
        SymbolData::VirtualFunctionTable(value) => {
            rewrite.type_index(&mut value.root)?;
            rewrite.type_index(&mut value.path)
        }
        SymbolData::Local(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::RegisterRelative(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::RegisterRelativeIndirect(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::BasePointerRelative(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::BasePointerRelativeIndirect(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::Register(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::FileStatic(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::ThreadData(value) | SymbolData::Data(value) => {
            rewrite.type_index(&mut value.type_index)
        }
        SymbolData::UserDefinedType(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::Constant(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::HlslData(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::HlslDataEx(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::DpcGroupSharedLocal(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::Oem(value) => rewrite.type_index(&mut value.type_index),
        SymbolData::ManagedTypeReference(index) => rewrite.type_index(index),
        SymbolData::ManyRegisters(value) => native_or_token(
            &mut value.type_index,
            kind == S_MANMANYREG || kind == S_MANMANYREG2,
            rewrite,
        ),
        SymbolData::AttributedFrameRelative(value) => {
            native_or_token(&mut value.type_index, kind == S_MANFRAMEREL, rewrite)
        }
        SymbolData::AttributedRegister(value) => {
            native_or_token(&mut value.type_index, kind == S_MANREGISTER, rewrite)
        }
        SymbolData::AttributedRegisterRelative(value) => {
            native_or_token(&mut value.type_index, kind == S_MANREGREL, rewrite)
        }
        // `S_LOCALSLOT` and `S_PARAMSLOT` hold a type index or a metadata
        // token. A token has its table number in the high byte, and no
        // metadata table with the number 0 has rows at `0x1000` or later.
        // So a value with a high byte of 0 is a type index.
        SymbolData::Slot(value) => {
            if value.type_or_token >> 24 == 0 {
                rewrite.type_value(&mut value.type_or_token)
            } else {
                Ok(())
            }
        }
        SymbolData::ManagedData(value) => token(value.type_index.0, rewrite),
        SymbolData::ManagedProcedure(value) => token(value.token.raw(), rewrite),
        SymbolData::ManagedSlot(value) => token(value.type_or_token, rewrite),
        SymbolData::ManagedConstant(value) => token(value.type_or_token, rewrite),
        SymbolData::FastLink(_) => Err(unsupported(
            "an S_FASTLINK or S_REF_MINIPDB record names types of a type source outside the TPI \
             and IPI streams",
        )),
        SymbolData::ModuleTypeReference(_) => Err(unsupported(
            "an S_MOD_TYPEREF record tells that the indices of the module are indices of the \
             object file",
        )),
        SymbolData::Discarded(value) => {
            if value.data.is_empty() {
                Ok(())
            } else {
                Err(unsupported(
                    "an S_DISCARDED record holds a discarded record without a decoder",
                ))
            }
        }
        SymbolData::Unsupported(_) => Err(unsupported(
            "a symbol record of a kind without a decoder can refer to a changed index",
        )),
        // `S_PROCREF`, `S_LPROCREF`, `S_TOKENREF`, `S_DATAREF` and
        // `S_ANNOTATIONREF` name a module and an offset, not an index. The
        // other kinds have no index.
        SymbolData::ProcedureReference(_)
        | SymbolData::SymbolReference(_)
        | SymbolData::ObjectName(_)
        | SymbolData::Compile2(_)
        | SymbolData::Compile3(_)
        | SymbolData::EnvironmentBlock(_)
        | SymbolData::UsingNamespace(_)
        | SymbolData::FrameProcedure(_)
        | SymbolData::Block(_)
        | SymbolData::With(_)
        | SymbolData::SeparatedCode(_)
        | SymbolData::Label(_)
        | SymbolData::Thunk(_)
        | SymbolData::ScopeEnd
        | SymbolData::InlineSiteEnd
        | SymbolData::JumpTable(_)
        | SymbolData::ProfileData(_)
        | SymbolData::Association(_)
        | SymbolData::HybridRange(_)
        | SymbolData::AnnotationStrings(_)
        | SymbolData::DefRangeProgram(_)
        | SymbolData::DefRangeFramePointerRelative(_)
        | SymbolData::DefRangeFramePointerRelativeFullScope(_)
        | SymbolData::DefRangeRegister(_)
        | SymbolData::DefRangeRegisterRelative(_)
        | SymbolData::DefRangeRegisterRelativeIndirect(_)
        | SymbolData::DefRangeSubfieldRegister(_)
        | SymbolData::DefRangeHlsl(_)
        | SymbolData::DefRangeConstantOnEntry(_)
        | SymbolData::DefRangeGlobalOnEntry(_)
        | SymbolData::StaticLocal(_)
        | SymbolData::FrameCookie(_)
        | SymbolData::FrameRegister(_)
        | SymbolData::Trampoline(_)
        | SymbolData::Section(_)
        | SymbolData::CoffGroup(_)
        | SymbolData::Export(_)
        | SymbolData::PdbMap(_)
        | SymbolData::DpcTagMap(_)
        | SymbolData::Public(_)
        | SymbolData::Skip(_) => Ok(()),
    }
}

/// Write the output index into the signature of a procedure: a type index,
/// or an ID index for the `_ID` kinds.
fn signature<M: IndexRemap>(
    signature: &mut ProcedureType,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    match signature {
        ProcedureType::Type(index) => rewrite.type_index(index),
        ProcedureType::Id(index) => rewrite.id_index(index),
    }
}

/// Write the output indices into the functions of `list`. The invocation
/// counts stay.
fn function_list<M: IndexRemap>(
    list: &mut FunctionList<'_>,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    let Some(words) = rewrite.id_words(list.functions().as_bytes())? else {
        return Ok(());
    };
    let mut counts = Vec::new();
    counts
        .try_reserve_exact(list.functions().len() * 4)
        .map_err(|_| no_memory())?;
    for count in list.invocations() {
        counts.extend_from_slice(&count.to_le_bytes());
    }
    *list = FunctionList::new(IdList::parse(words)?, counts)?;
    Ok(())
}

/// Write the output index into the type field of a kind that has a native
/// and a managed form. The managed form (`managed`) holds a metadata token.
fn native_or_token<M: IndexRemap>(
    index: &mut TypeIndex,
    managed: bool,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    if managed {
        token(index.0, rewrite)
    } else {
        rewrite.type_index(index)
    }
}

/// Keep the metadata token `value`.
///
/// # Errors
///
/// Returns [`ErrorKind::UnsupportedEdit`](crate::ErrorKind::UnsupportedEdit)
/// if the value has a high byte of 0 and is at least `0x1000`, and the map
/// does not give the value back: the value can be a type index that
/// changes.
fn token<M: IndexRemap>(value: u32, rewrite: &Rewrite<'_, M>) -> Result<()> {
    if value >> 24 != 0 || value < FIRST_RECORD_INDEX {
        return Ok(());
    }
    match rewrite.map().type_index(TypeIndex(value)) {
        Ok(output) if output.0 == value => Ok(()),
        _ => Err(unsupported(
            "a managed symbol has a value that can be a type index",
        )),
    }
}

/// Write the output indices into the framed symbol record `bytes`.
///
/// `bytes` holds exactly one record. Return `true` if a byte changed.
///
/// # Errors
///
/// - The framing and decoding errors of the record.
/// - The errors of [`symbol`].
/// - [`ErrorKind::UnsupportedEdit`](crate::ErrorKind::UnsupportedEdit) if
///   the decoded record does not encode to the size of `bytes`.
pub(super) fn record(bytes: &mut [u8], map: &impl IndexRemap) -> Result<bool> {
    let (before, after) = {
        let source = first_record(bytes)?;
        if source.data.len() + PREFIX_SIZE != bytes.len() {
            return Err(unsupported(
                "bytes follow a symbol record of a record rewrite",
            ));
        }
        let mut symbol = Symbol::decode(source)?;
        let Some(data) = self::symbol(symbol.kind(), symbol.data(), map)? else {
            return Ok(false);
        };
        let mut before = Vec::new();
        symbol.encode(&mut before)?;
        *symbol.data_mut() = data;
        let mut after = Vec::new();
        symbol.encode(&mut after)?;
        (before, after)
    };
    write_difference(bytes, &before, &after)
}

/// Write the output indices into the framed symbol records of `bytes`.
///
/// `bytes` is a symbol record stream, the symbol region of a module stream
/// without its signature, or the data of a `DEBUG_S_SYMBOLS` subsection.
/// `base` is the offset of `bytes` in `stream`. A record that starts a scope
/// keeps its links. Each record keeps its size and the bytes outside its
/// index fields. Return `true` if a byte changed.
///
/// # Errors
///
/// Returns the errors of the framing, of the decoder and of [`symbol`],
/// with the location of the record in `stream`. The kind, the context and
/// the index space of the error stay.
pub(in crate::native::editor::plan) fn symbol_records(
    bytes: &mut [u8],
    stream: StreamId,
    base: u64,
    map: &impl IndexRemap,
) -> Result<bool> {
    let mut changed = false;
    let mut position = 0;
    while position < bytes.len() {
        let offset = base.saturating_add(u64::try_from(position).unwrap_or(u64::MAX));
        let located = |error: Error| error.with_stream(stream, offset);
        let length = first_record(&bytes[position..])
            .map_err(located)?
            .data
            .len();
        let end = position + PREFIX_SIZE + length;
        changed |= record(&mut bytes[position..end], map).map_err(located)?;
        position = end;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{symbol, symbol_records};
    use crate::native::IndexSpace;
    use crate::native::address::SectionOffset;
    use crate::native::codeview::Records;
    use crate::native::codeview::id_record::IdList;
    use crate::native::codeview::index::{IdIndex, ModuleSymbolOffset, TypeIndex};
    use crate::native::codeview::symbol::{
        Data, Procedure, ProcedureType, RegisterRelative, S_END, S_GMANDATA, S_GPROC32_ID,
        S_INLINEES, S_REGREL32, S_UDT, ScopeLinks, Symbol, SymbolData, UserDefinedType,
    };
    use crate::native::editor::plan::compact::map::IndexMaps;
    use crate::native::editor::plan::compact::records::tests::{removal, words};
    use crate::native::msf::StreamId;
    use crate::{Bytes, ErrorKind};

    /// The stream of the test records.
    const STREAM: StreamId = StreamId(7);

    /// The links of the test procedure.
    const LINKS: ScopeLinks = ScopeLinks::new(ModuleSymbolOffset(0), ModuleSymbolOffset(0x40));

    /// Return an `S_UDT` record for `index`.
    fn udt(index: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        SymbolData::UserDefinedType(UserDefinedType {
            type_index: TypeIndex(index),
            name: Bytes::new(b"point"),
        })
        .encode(S_UDT, &mut bytes)
        .unwrap();
        bytes
    }

    /// Return a procedure scope and an `S_INLINEES` and `S_UDT` record.
    fn records() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut procedure = Procedure::new(ProcedureType::Id(IdIndex(0x1004)));
        procedure.name = Bytes::new(b"run");
        SymbolData::Procedure(procedure)
            .encode_scope(S_GPROC32_ID, LINKS, &mut bytes)
            .unwrap();
        SymbolData::RegisterRelative(RegisterRelative {
            offset: 8,
            type_index: TypeIndex(0x1003),
            register: 335,
            name: Bytes::new(b"value"),
        })
        .encode(S_REGREL32, &mut bytes)
        .unwrap();
        SymbolData::ScopeEnd.encode(S_END, &mut bytes).unwrap();
        let inlinees = IdList::parse(words(&[0x1002, 0x1003])).unwrap();
        SymbolData::Inlinees(inlinees)
            .encode(S_INLINEES, &mut bytes)
            .unwrap();
        bytes.extend_from_slice(&udt(0x1001));
        bytes
    }

    #[test]
    fn records_get_the_output_indices() {
        let mut bytes = records();
        let length = bytes.len();
        assert!(symbol_records(&mut bytes, STREAM, 4, &removal()).unwrap());
        assert_eq!(bytes.len(), length);
        let symbols: Vec<_> = Records::new(&bytes)
            .map(|record| Symbol::decode(record.unwrap()).unwrap())
            .collect();
        assert_eq!(symbols.len(), 5);
        assert_eq!(symbols[0].id_index(), Some(IdIndex(0x1003)));
        assert_eq!(symbols[0].links(), Some(LINKS));
        assert_eq!(symbols[1].type_index(), Some(TypeIndex(0x1002)));
        assert_eq!(symbols[2].kind(), S_END);
        let SymbolData::Inlinees(list) = symbols[3].data() else {
            panic!("not an inlinee list");
        };
        let inlinees: Vec<_> = list.iter().collect();
        assert_eq!(inlinees, [IdIndex(0x1001), IdIndex(0x1002)]);
        assert_eq!(symbols[4].type_index(), Some(TypeIndex(0x1000)));
    }

    #[test]
    fn reference_to_a_removed_record_has_the_record_location() {
        let mut bytes = records();
        let offset = bytes.len();
        bytes.extend_from_slice(&udt(0x1000));
        let error = symbol_records(&mut bytes, STREAM, 4, &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(error.index_space(), Some(IndexSpace::TypeIndex));
        let offset = u64::try_from(offset).unwrap() + 4;
        assert_eq!(error.stream_location(), Some((STREAM, offset)));
    }

    #[test]
    fn identity_map_changes_nothing() {
        let mut bytes = records();
        let source = bytes.clone();
        assert!(!symbol_records(&mut bytes, STREAM, 4, &IndexMaps::default()).unwrap());
        assert_eq!(bytes, source);
        let data = SymbolData::UserDefinedType(UserDefinedType {
            type_index: TypeIndex(0x1005),
            name: Bytes::new(b"point"),
        });
        assert!(
            symbol(S_UDT, &data, &IndexMaps::default())
                .unwrap()
                .is_none()
        );
    }

    /// Return managed data with the type field `value`.
    fn managed(value: u32) -> SymbolData<'static> {
        SymbolData::ManagedData(Data {
            type_index: TypeIndex(value),
            address: SectionOffset::default(),
            name: Bytes::new(b"field"),
        })
    }

    #[test]
    fn managed_token_stays() {
        let data = managed(0x0400_0001);
        assert!(symbol(S_GMANDATA, &data, &removal()).unwrap().is_none());
    }

    #[test]
    fn managed_value_that_can_be_a_changed_type_index_is_unsupported() {
        let error = symbol(S_GMANDATA, &managed(0x1005), &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    }
}
