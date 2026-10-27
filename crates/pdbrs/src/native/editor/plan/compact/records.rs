//! The index fields of the TPI and IPI records.
//!
//! [`type_record`] and [`id_record`] give a record with the output index in
//! each index field. [`type_record_bytes`] and [`id_record_bytes`] write the
//! changed fields into the bytes of one framed record. The checklist of the
//! type index fields is `TypeLeaf::for_each_reference` and
//! `Member::for_each_reference`.
//!
//! [`Rewrite`] and the other helpers of this module also serve the other
//! rewriters of the compaction.

use alloc::vec::Vec;

use super::bytes::apply_difference;
use super::map::IndexRemap;
use crate::native::codeview::field_list::{FieldList, Member};
use crate::native::codeview::id_record::{IdList, IdRecord};
use crate::native::codeview::index::{IdIndex, TypeIndex};
use crate::native::codeview::type_record::{
    ArgumentList, BasedPointer, MethodList, TypeIndexList, TypeLeaf,
};
use crate::native::codeview::{Record, Records};
use crate::native::output::{Output, no_memory};
use crate::{Bytes, Error, ErrorKind, Result};

/// The size of the length field and the kind field of a framed record.
const PREFIX_SIZE: usize = 4;

/// The size of one serialized index.
const INDEX_SIZE: usize = 4;

/// The remap of the index fields of one value, with a record of a change.
pub(super) struct Rewrite<'m, M> {
    /// The output index of each model index.
    map: &'m M,
    /// `true` after an index field got a new value.
    changed: bool,
}

impl<'m, M: IndexRemap> Rewrite<'m, M> {
    /// Make a rewrite with `map` and no change.
    pub(super) const fn new(map: &'m M) -> Self {
        Self {
            map,
            changed: false,
        }
    }

    /// Return the index map.
    pub(super) const fn map(&self) -> &'m M {
        self.map
    }

    /// Return `true` if an index field got a new value.
    pub(super) const fn changed(&self) -> bool {
        self.changed
    }

    /// Record a change that a nested rewrite made.
    pub(super) const fn note(&mut self, changed: bool) {
        self.changed |= changed;
    }

    /// Write the output index of the type index `index` into `index`.
    pub(super) fn type_index(&mut self, index: &mut TypeIndex) -> Result<()> {
        let output = self.map.type_index(*index)?;
        if output != *index {
            *index = output;
            self.changed = true;
        }
        Ok(())
    }

    /// Write the output index of the ID index `index` into `index`.
    pub(super) fn id_index(&mut self, index: &mut IdIndex) -> Result<()> {
        let output = self.map.id_index(*index)?;
        if output != *index {
            *index = output;
            self.changed = true;
        }
        Ok(())
    }

    /// Write the output index of the raw type index `value` into `value`.
    pub(super) fn type_value(&mut self, value: &mut u32) -> Result<()> {
        let mut index = TypeIndex(*value);
        self.type_index(&mut index)?;
        *value = index.0;
        Ok(())
    }

    /// Write the output index of the raw ID index `value` into `value`.
    pub(super) fn id_value(&mut self, value: &mut u32) -> Result<()> {
        let mut index = IdIndex(*value);
        self.id_index(&mut index)?;
        *value = index.0;
        Ok(())
    }

    /// Return a copy of the serialized type indices `words` with the output
    /// indices, or `None` if no index changes.
    pub(super) fn type_words(&mut self, words: &[u8]) -> Result<Option<Vec<u8>>> {
        self.words(words, |map, value| Ok(map.type_index(TypeIndex(value))?.0))
    }

    /// Return a copy of the serialized ID indices `words` with the output
    /// indices, or `None` if no index changes.
    pub(super) fn id_words(&mut self, words: &[u8]) -> Result<Option<Vec<u8>>> {
        self.words(words, |map, value| Ok(map.id_index(IdIndex(value))?.0))
    }

    /// Return a copy of the little-endian `u32` values `words` with the
    /// values of `remap`, or `None` if no value changes.
    fn words(
        &mut self,
        words: &[u8],
        remap: impl Fn(&M, u32) -> Result<u32>,
    ) -> Result<Option<Vec<u8>>> {
        let mut output: Option<Vec<u8>> = None;
        for (position, word) in words.as_chunks::<INDEX_SIZE>().0.iter().enumerate() {
            let value = u32::from_le_bytes(*word);
            let new = remap(self.map, value)?;
            if new == value {
                continue;
            }
            let copy = match &mut output {
                Some(copy) => copy,
                None => output.insert(copy(words)?),
            };
            let start = position * INDEX_SIZE;
            copy[start..start + INDEX_SIZE].copy_from_slice(&new.to_le_bytes());
        }
        self.changed |= output.is_some();
        Ok(output)
    }
}

/// Return a copy of `bytes` in a checked allocation.
pub(super) fn copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

/// Return an [`ErrorKind::UnsupportedEdit`] error with `context`.
pub(super) fn unsupported(context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedEdit, None).with_context(context)
}

/// Write the bytes of `after` that differ from `before` into `target`.
///
/// `before` is the encoding of the decoded value and `after` the encoding of
/// the value with the output indices. Return `true` if a byte changed.
///
/// # Errors
///
/// Returns [`ErrorKind::UnsupportedEdit`] if `before` does not have the size
/// of `target`: the positions of the fields are then not known.
pub(super) fn write_difference(target: &mut [u8], before: &[u8], after: &[u8]) -> Result<bool> {
    if before.len() != target.len() {
        return Err(unsupported(
            "a record with changed indices does not encode to the size of its source bytes",
        ));
    }
    apply_difference(target, before, after)?;
    Ok(before != after)
}

/// Frame the first record of `bytes`.
///
/// # Errors
///
/// Returns the framing errors of [`Records`], and
/// [`ErrorKind::UnexpectedEof`] if `bytes` is empty.
pub(super) fn first_record(bytes: &[u8]) -> Result<Record<'_>> {
    Records::new(bytes).next().unwrap_or_else(|| {
        Err(Error::new(ErrorKind::UnexpectedEof, None)
            .with_context("a record rewrite has no record bytes"))
    })
}

/// Frame `bytes` as exactly one record.
fn single_record(bytes: &[u8]) -> Result<Record<'_>> {
    let record = first_record(bytes)?;
    if record.data.len() + PREFIX_SIZE != bytes.len() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("bytes follow the record of a record rewrite"));
    }
    Ok(record)
}

/// Return the record `leaf` with the output index in each type index field,
/// or `None` if no index changes.
///
/// # Errors
///
/// - The errors of `map`, unchanged.
/// - [`ErrorKind::UnsupportedEdit`] for [`TypeLeaf::Unsupported`]: the
///   index fields of the record are not known.
/// - The errors of the constructors of the list views.
pub(in crate::native::editor::plan) fn type_record(
    leaf: &TypeLeaf<'_>,
    map: &impl IndexRemap,
) -> Result<Option<TypeLeaf<'static>>> {
    let mut rewrite = Rewrite::new(map);
    let mut leaf = leaf.clone();
    remap_leaf(&mut leaf, &mut rewrite)?;
    Ok(rewrite.changed().then(|| leaf.into_owned()))
}

/// Write the output indices into the type index fields of `leaf`.
fn remap_leaf<M: IndexRemap>(leaf: &mut TypeLeaf<'_>, rewrite: &mut Rewrite<'_, M>) -> Result<()> {
    match leaf {
        TypeLeaf::Modifier(value) => rewrite.type_index(&mut value.modified_type),
        TypeLeaf::Pointer(value) => {
            rewrite.type_index(&mut value.referent_type)?;
            if let Some(member) = &mut value.member {
                rewrite.type_index(&mut member.containing_type)?;
            }
            match &mut value.based {
                Some(BasedPointer::Type { base_type, .. }) => rewrite.type_index(base_type),
                Some(BasedPointer::Symbol(record)) => based_symbol(record, rewrite),
                Some(BasedPointer::Segment(_) | BasedPointer::SelfBased) | None => Ok(()),
            }
        }
        TypeLeaf::Procedure(value) => {
            rewrite.type_index(&mut value.return_type)?;
            rewrite.type_index(&mut value.argument_list)
        }
        TypeLeaf::MemberFunction(value) => {
            rewrite.type_index(&mut value.return_type)?;
            rewrite.type_index(&mut value.class_type)?;
            rewrite.type_index(&mut value.this_type)?;
            rewrite.type_index(&mut value.argument_list)
        }
        TypeLeaf::ArgumentList(list) => {
            if let Some(words) = rewrite.type_words(list.list().as_bytes())? {
                *list = ArgumentList::parse(words)?;
            }
            Ok(())
        }
        TypeLeaf::VirtualFunctionTablePath(list) | TypeLeaf::DerivedClasses(list) => {
            index_list(list, rewrite)
        }
        TypeLeaf::FieldList(list) => field_list(list, rewrite),
        TypeLeaf::Bitfield(value) => rewrite.type_index(&mut value.underlying_type),
        TypeLeaf::MethodList(list) => method_list(list, rewrite),
        TypeLeaf::Array(value) => {
            rewrite.type_index(&mut value.element_type)?;
            rewrite.type_index(&mut value.index_type)
        }
        TypeLeaf::Class(value) => {
            rewrite.type_index(&mut value.field_list)?;
            rewrite.type_index(&mut value.derivation_list)?;
            rewrite.type_index(&mut value.vtable_shape)
        }
        TypeLeaf::WideClass(value) => {
            rewrite.type_index(&mut value.field_list)?;
            rewrite.type_index(&mut value.derivation_list)?;
            rewrite.type_index(&mut value.vtable_shape)
        }
        TypeLeaf::Union(value) => rewrite.type_index(&mut value.field_list),
        TypeLeaf::WideUnion(value) => rewrite.type_index(&mut value.field_list),
        TypeLeaf::Enum(value) => {
            rewrite.type_index(&mut value.underlying_type)?;
            rewrite.type_index(&mut value.field_list)
        }
        TypeLeaf::VirtualFunctionTable(value) => {
            rewrite.type_index(&mut value.complete_class)?;
            match &mut value.overridden_table {
                Some(table) => rewrite.type_index(table),
                None => Ok(()),
            }
        }
        TypeLeaf::Cobol0(value) => rewrite.type_index(&mut value.parent_type),
        TypeLeaf::BasicArray(index) => rewrite.type_index(index),
        TypeLeaf::Oem(value) => index_list(&mut value.indices, rewrite),
        TypeLeaf::Oem2(value) => index_list(&mut value.indices, rewrite),
        // The bound type is a primitive type with a known size (the decoder
        // rejects other types), so its index maps to itself. The width of
        // the serialized bounds depends on it and stays valid.
        TypeLeaf::ConstantDimensions(value) => rewrite.type_index(&mut value.index_type),
        TypeLeaf::VariableDimensions(value) => {
            rewrite.type_index(&mut value.index_type)?;
            index_list(&mut value.bounds, rewrite)
        }
        TypeLeaf::DimensionedArray(value) => {
            rewrite.type_index(&mut value.element_type)?;
            rewrite.type_index(&mut value.dimensions)
        }
        TypeLeaf::Alias(value) => rewrite.type_index(&mut value.underlying_type),
        TypeLeaf::DefaultArgument(value) => rewrite.type_index(&mut value.argument_type),
        TypeLeaf::StridedArray(value) => {
            rewrite.type_index(&mut value.element_type)?;
            rewrite.type_index(&mut value.index_type)
        }
        TypeLeaf::Hlsl(value) => rewrite.type_index(&mut value.subtype),
        TypeLeaf::ModifierEx(value) => rewrite.type_index(&mut value.modified_type),
        TypeLeaf::Vector(value) => rewrite.type_index(&mut value.element_type),
        TypeLeaf::Matrix(value) => rewrite.type_index(&mut value.element_type),
        // `LF_SKIP` holds the next index of the record numbering, not a
        // reference to a record. This crate gives each record one index and
        // does not read the value. `LF_PRECOMP` holds the first index of the
        // precompiled types of an object file, also not a reference to a
        // record of this PDB. The other kinds have no type index.
        TypeLeaf::Skip(_)
        | TypeLeaf::Precompiled(_)
        | TypeLeaf::VtableShape(_)
        | TypeLeaf::Label(_)
        | TypeLeaf::TypeServer(_)
        | TypeLeaf::TypeServer2(_)
        | TypeLeaf::EndPrecompiled(_)
        | TypeLeaf::Managed(_) => Ok(()),
        TypeLeaf::Unsupported(_) => Err(unsupported(
            "a type record of a kind without a decoder can refer to a changed index",
        )),
    }
}

/// Write the output indices into the list of type indices `list`.
fn index_list<M: IndexRemap>(
    list: &mut TypeIndexList<'_>,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    if let Some(words) = rewrite.type_words(list.as_bytes())? {
        *list = TypeIndexList::parse(words)?;
    }
    Ok(())
}

/// Write the output indices into the symbol record of a pointer that is
/// based on a symbol.
fn based_symbol<M: IndexRemap>(record: &mut Bytes<'_>, rewrite: &mut Rewrite<'_, M>) -> Result<()> {
    let mut bytes = copy(record)?;
    if super::symbols::record(&mut bytes, rewrite.map())? {
        *record = Bytes::from(bytes);
        rewrite.note(true);
    }
    Ok(())
}

/// Write the output indices into the members of the field list `list`.
fn field_list<M: IndexRemap>(list: &mut FieldList<'_>, rewrite: &mut Rewrite<'_, M>) -> Result<()> {
    let mut data = Vec::new();
    data.try_reserve_exact(list.data().len())
        .map_err(|_| no_memory())?;
    let before = rewrite.changed();
    rewrite.changed = false;
    for member in list.members() {
        let mut member = member?;
        remap_member(&mut member, rewrite)?;
        member.encode(&mut data)?;
    }
    if rewrite.changed() {
        *list = FieldList::parse(data)?;
    }
    rewrite.note(before);
    Ok(())
}

/// Write the output indices into the type index fields of `member`.
fn remap_member<M: IndexRemap>(
    member: &mut Member<'_>,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    match member {
        Member::BaseClass(base) | Member::BaseInterface(base) => {
            rewrite.type_index(&mut base.base_type)
        }
        Member::VirtualBaseClass(base) => {
            rewrite.type_index(&mut base.base_type)?;
            rewrite.type_index(&mut base.vbptr_type)
        }
        Member::Continuation(index)
        | Member::VirtualFunctionTable(index)
        | Member::FriendClass(index) => rewrite.type_index(index),
        Member::VirtualFunctionOffset(value) => rewrite.type_index(&mut value.pointer_type),
        Member::FriendFunction(value) => rewrite.type_index(&mut value.function_type),
        Member::Enumerator(_) => Ok(()),
        Member::DataMember(value) => rewrite.type_index(&mut value.member_type),
        Member::StaticMember(value) => rewrite.type_index(&mut value.member_type),
        Member::OverloadedMethod(value) => rewrite.type_index(&mut value.method_list),
        Member::NestedType(value) => rewrite.type_index(&mut value.nested_type),
        Member::NestedTypeEx(value) => rewrite.type_index(&mut value.nested_type),
        Member::MemberModify(value) => rewrite.type_index(&mut value.base_type),
        Member::OneMethod(value) => rewrite.type_index(&mut value.method_type),
    }
}

/// Write the output indices into the entries of the method list `list`.
fn method_list<M: IndexRemap>(
    list: &mut MethodList<'_>,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    let mut data = Vec::new();
    let before = rewrite.changed();
    rewrite.changed = false;
    for entry in list.entries() {
        let mut entry = entry?;
        rewrite.type_index(&mut entry.method_type)?;
        entry.encode(&mut Output::new(&mut data))?;
    }
    if rewrite.changed() {
        *list = MethodList::parse(data)?;
    }
    rewrite.note(before);
    Ok(())
}

/// Return the record `record` with the output index in each type index and
/// ID index field, or `None` if no index changes.
///
/// # Errors
///
/// - The errors of `map`, unchanged.
/// - [`ErrorKind::UnsupportedEdit`] for [`IdRecord::Unsupported`]: the
///   index fields of the record are not known.
pub(in crate::native::editor::plan) fn id_record(
    record: &IdRecord<'_>,
    map: &impl IndexRemap,
) -> Result<Option<IdRecord<'static>>> {
    let mut rewrite = Rewrite::new(map);
    let mut record = record.clone();
    match &mut record {
        IdRecord::Function(value) => {
            if let Some(scope) = &mut value.scope {
                rewrite.id_index(scope)?;
            }
            rewrite.type_index(&mut value.function_type)?;
        }
        IdRecord::MemberFunction(value) => {
            rewrite.type_index(&mut value.parent_type)?;
            rewrite.type_index(&mut value.function_type)?;
        }
        IdRecord::BuildInfo(value) => id_list(&mut value.arguments, &mut rewrite)?,
        IdRecord::SubstringList(list) => id_list(list, &mut rewrite)?,
        IdRecord::String(value) => {
            if let Some(substrings) = &mut value.substrings {
                rewrite.id_index(substrings)?;
            }
        }
        IdRecord::UdtSourceLine(value) => {
            rewrite.type_index(&mut value.udt)?;
            rewrite.id_index(&mut value.source_file)?;
        }
        // The name offset is a `/names` offset and the module is a DBI
        // module index. Neither is a TPI or IPI index.
        IdRecord::UdtModuleSourceLine(value) => rewrite.type_index(&mut value.udt)?,
        IdRecord::Unsupported(_) => {
            return Err(unsupported(
                "an ID record of a kind without a decoder can refer to a changed index",
            ));
        }
    }
    Ok(rewrite.changed().then(|| record.into_owned()))
}

/// Write the output indices into the list of ID indices `list`.
pub(super) fn id_list<M: IndexRemap>(
    list: &mut IdList<'_>,
    rewrite: &mut Rewrite<'_, M>,
) -> Result<()> {
    if let Some(words) = rewrite.id_words(list.as_bytes())? {
        *list = IdList::parse(words)?;
    }
    Ok(())
}

/// Write the output indices into the framed TPI record `bytes`: the length
/// field, the kind, the payload and the padding.
///
/// Return `true` if a byte changed. The bytes outside the index fields stay.
///
/// # Errors
///
/// - The framing and decoding errors of the record.
/// - The errors of [`type_record`].
/// - [`ErrorKind::UnsupportedEdit`] if the decoded record does not encode to
///   the size of `bytes`.
pub(in crate::native::editor::plan) fn type_record_bytes(
    bytes: &mut [u8],
    map: &impl IndexRemap,
) -> Result<bool> {
    let (before, after) = {
        let leaf = TypeLeaf::decode(single_record(bytes)?)?;
        let Some(output) = type_record(&leaf, map)? else {
            return Ok(false);
        };
        let mut before = Vec::new();
        leaf.encode(&mut before)?;
        let mut after = Vec::new();
        output.encode(&mut after)?;
        (before, after)
    };
    write_difference(bytes, &before, &after)
}

/// Write the output indices into the framed IPI record `bytes`: the length
/// field, the kind, the payload and the padding.
///
/// Return `true` if a byte changed. The bytes outside the index fields stay.
///
/// # Errors
///
/// - The framing and decoding errors of the record.
/// - The errors of [`id_record`].
/// - [`ErrorKind::UnsupportedEdit`] if the decoded record does not encode to
///   the size of `bytes`.
pub(in crate::native::editor::plan) fn id_record_bytes(
    bytes: &mut [u8],
    map: &impl IndexRemap,
) -> Result<bool> {
    let (before, after) = {
        let record = IdRecord::decode(single_record(bytes)?)?;
        let Some(output) = id_record(&record, map)? else {
            return Ok(false);
        };
        let mut before = Vec::new();
        record.encode(&mut before)?;
        let mut after = Vec::new();
        output.encode(&mut after)?;
        (before, after)
    };
    write_difference(bytes, &before, &after)
}

#[cfg(test)]
pub(super) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{id_record, id_record_bytes, type_record, type_record_bytes};
    use crate::native::IndexSpace;
    use crate::native::codeview::Records;
    use crate::native::codeview::field_list::{
        BaseClass, DataMember, FieldList, Member, OverloadedMethod, VirtualBaseClass,
    };
    use crate::native::codeview::id_record::{IdRecord, LF_BUILDINFO, LF_FUNC_ID};
    use crate::native::codeview::index::{IdIndex, TypeIndex};
    use crate::native::codeview::numeric::{Integer, Numeric};
    use crate::native::codeview::type_record::{
        LF_ARGLIST, LF_FIELDLIST, LF_METHODLIST, LF_POINTER, MemberAttributes, TypeLeaf,
    };
    use crate::native::editor::plan::compact::map::{IndexMaps, SpaceMap};
    use crate::{Bytes, ErrorKind};

    /// The number of model records in each space of [`removal`].
    const RECORDS: usize = 64;

    /// Return maps that remove the record `0x1000` of each space and move
    /// each later record down by one index.
    pub(in crate::native::editor::plan::compact) fn removal() -> IndexMaps {
        let space = || SpaceMap::new((0..RECORDS).map(|position| position == 0)).unwrap();
        let mut maps = IndexMaps::default();
        maps.types = Some(space());
        maps.ids = Some(space());
        maps
    }

    /// Return a framed record of `kind` with `payload` and `LF_PAD` padding.
    pub(in crate::native::editor::plan::compact) fn framed(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut record = Vec::new();
        record.extend_from_slice(&[0, 0]);
        record.extend_from_slice(&kind.to_le_bytes());
        record.extend_from_slice(payload);
        let mut padding = (4 - record.len() % 4) % 4;
        while padding > 0 {
            record.push(0xf0 | u8::try_from(padding).unwrap());
            padding -= 1;
        }
        let length = u16::try_from(record.len() - 2).unwrap();
        record[..2].copy_from_slice(&length.to_le_bytes());
        record
    }

    /// Return the little-endian bytes of `values`.
    pub(in crate::native::editor::plan::compact) fn words(values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// Decode the framed TPI record `bytes`.
    fn leaf(bytes: &[u8]) -> TypeLeaf<'_> {
        TypeLeaf::decode(Records::new(bytes).next().unwrap().unwrap()).unwrap()
    }

    /// Return an `LF_POINTER` record to `referent`: a 64-bit near pointer.
    fn pointer(referent: u32) -> Vec<u8> {
        framed(LF_POINTER.0, &words(&[referent, 0x0c | (8 << 13)]))
    }

    #[test]
    fn pointer_gets_the_output_index() {
        let mut bytes = pointer(0x1005);
        let length = bytes.len();
        assert!(type_record_bytes(&mut bytes, &removal()).unwrap());
        assert_eq!(bytes.len(), length);
        let TypeLeaf::Pointer(value) = leaf(&bytes) else {
            panic!("not a pointer");
        };
        assert_eq!(value.referent_type, TypeIndex(0x1004));
    }

    #[test]
    fn argument_list_gets_the_output_indices() {
        let mut bytes = framed(LF_ARGLIST.0, &words(&[3, 0x1002, 0x74, 0x1010]));
        assert!(type_record_bytes(&mut bytes, &removal()).unwrap());
        let TypeLeaf::ArgumentList(list) = leaf(&bytes) else {
            panic!("not an argument list");
        };
        let indices: Vec<_> = list.iter().collect();
        assert_eq!(
            indices,
            [TypeIndex(0x1001), TypeIndex(0x74), TypeIndex(0x100f)]
        );
    }

    /// Return the attributes of a public member.
    fn public() -> MemberAttributes {
        MemberAttributes::from_raw(3).unwrap()
    }

    /// Return a numeric leaf with `value`.
    fn number(value: u32) -> Numeric<'static> {
        Numeric::from_integer(Integer::Unsigned(u128::from(value)))
    }

    #[test]
    fn field_list_members_get_the_output_indices() {
        let members = [
            Member::BaseClass(BaseClass {
                attributes: public(),
                base_type: TypeIndex(0x1003),
                offset: number(0),
            }),
            Member::VirtualBaseClass(VirtualBaseClass {
                indirect: false,
                attributes: public(),
                base_type: TypeIndex(0x1004),
                vbptr_type: TypeIndex(0x1005),
                vbptr_offset: number(0),
                vbtable_index: number(1),
            }),
            Member::DataMember(DataMember {
                attributes: public(),
                member_type: TypeIndex(0x1006),
                offset: number(8),
                name: Bytes::new(b"field"),
            }),
            Member::OverloadedMethod(OverloadedMethod {
                overload_count: 2,
                method_list: TypeIndex(0x1007),
                name: Bytes::new(b"run"),
            }),
        ];
        let mut payload = Vec::new();
        for member in &members {
            member.encode(&mut payload).unwrap();
        }
        let mut bytes = framed(LF_FIELDLIST.0, &payload);
        let length = bytes.len();
        assert!(type_record_bytes(&mut bytes, &removal()).unwrap());
        assert_eq!(bytes.len(), length);
        let TypeLeaf::FieldList(list) = leaf(&bytes) else {
            panic!("not a field list");
        };
        let mut indices = Vec::new();
        for member in list.members() {
            member
                .unwrap()
                .for_each_reference(|reference| {
                    indices.push(reference.index);
                    Ok(())
                })
                .unwrap();
        }
        let expected = [0x1002, 0x1003, 0x1004, 0x1005, 0x1006].map(TypeIndex);
        assert_eq!(indices, expected);
        let names: Vec<_> = list
            .members()
            .filter_map(|member| match member.unwrap() {
                Member::DataMember(value) => Some(value.name.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(names, [b"field".to_vec()]);
    }

    #[test]
    fn method_list_gets_the_output_indices() {
        // Two vanilla public methods: attributes 3, padding 0, method type.
        let mut bytes = framed(LF_METHODLIST.0, &words(&[3, 0x1008, 3, 0x1001]));
        assert!(type_record_bytes(&mut bytes, &removal()).unwrap());
        let TypeLeaf::MethodList(list) = leaf(&bytes) else {
            panic!("not a method list");
        };
        let types: Vec<_> = list
            .entries()
            .map(|entry| entry.unwrap().method_type)
            .collect();
        assert_eq!(types, [TypeIndex(0x1007), TypeIndex(0x1000)]);
    }

    #[test]
    fn function_id_maps_both_spaces() {
        let mut payload = words(&[0x1003, 0x1009]);
        payload.extend_from_slice(b"main\0");
        let mut bytes = framed(LF_FUNC_ID.0, &payload);
        assert!(id_record_bytes(&mut bytes, &removal()).unwrap());
        let record = IdRecord::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
        let IdRecord::Function(value) = record else {
            panic!("not a function ID");
        };
        assert_eq!(value.scope, Some(IdIndex(0x1002)));
        assert_eq!(value.function_type, TypeIndex(0x1008));
        assert_eq!(&*value.name, b"main");
    }

    #[test]
    fn build_info_arguments_get_the_output_indices() {
        let mut payload = vec![3, 0];
        payload.extend_from_slice(&words(&[0x1001, 0, 0x1002]));
        let bytes = framed(LF_BUILDINFO.0, &payload);
        let record = IdRecord::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
        let IdRecord::BuildInfo(value) = id_record(&record, &removal()).unwrap().unwrap() else {
            panic!("not a build information record");
        };
        let arguments: Vec<_> = value.arguments.iter().collect();
        assert_eq!(arguments, [IdIndex(0x1000), IdIndex(0), IdIndex(0x1001)]);
    }

    #[test]
    fn reference_to_a_removed_record_fails() {
        let bytes = pointer(0x1000);
        let error = type_record(&leaf(&bytes), &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(error.index_space(), Some(IndexSpace::TypeIndex));
        assert_eq!(error.stream_location(), None);
    }

    #[test]
    fn identity_map_changes_nothing() {
        let identity = IndexMaps::default();
        let mut bytes = pointer(0x1005);
        let source = bytes.clone();
        assert!(type_record(&leaf(&source), &identity).unwrap().is_none());
        assert!(!type_record_bytes(&mut bytes, &identity).unwrap());
        assert_eq!(bytes, source);
        let list = FieldList::parse(Vec::new()).unwrap();
        let empty = TypeLeaf::FieldList(list);
        assert!(type_record(&empty, &removal()).unwrap().is_none());
    }

    #[test]
    fn record_without_a_decoder_is_unsupported() {
        let bytes = framed(0x1fff, &words(&[0x1001]));
        let error = type_record(&leaf(&bytes), &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    }
}
