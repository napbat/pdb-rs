//! Type records that hold a list: argument lists, method overload lists,
//! virtual function table shapes and virtual function table names.
//!
//! [`TypeIndexList`] is the shared list of `u32` type indices after a `u32`
//! count, for example in `LF_ARGLIST`, `LF_VFTPATH` and `LF_DERIVED`.

use core::iter::FusedIterator;

use super::{MemberAttributes, TRUNCATED, VtableSlotKind, fields};
use crate::native::codeview::Fields;
use crate::native::codeview::index::TypeIndex;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The size of one type index in bytes.
const INDEX_SIZE: usize = 4;

/// The size of a method-list entry without a virtual table offset.
const METHOD_ENTRY_SIZE: usize = 8;

/// Return the error for a list length that does not fit its count field.
fn too_long() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("type record list is longer than its count field allows")
}

/// A borrowed list of serialized `u32` type indices.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TypeIndexList<'a> {
    /// The serialized type indices.
    indices: &'a [u8],
}

impl<'a> TypeIndexList<'a> {
    /// Make a list from serialized little-endian `u32` type indices.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length is not a multiple
    /// of 4.
    pub fn from_bytes(indices: &'a [u8]) -> Result<Self> {
        if indices.len().is_multiple_of(INDEX_SIZE) {
            Ok(Self { indices })
        } else {
            Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("type index list has a partial index"))
        }
    }

    /// Read a `u32` count and `count` type indices.
    pub(crate) fn decode_counted(fields: &mut Fields<'a>) -> Result<Self> {
        let count = fields.u32()?;
        Self::decode_count(fields, count)
    }

    /// Read `count` type indices.
    pub(crate) fn decode_count(fields: &mut Fields<'a>, count: u32) -> Result<Self> {
        let length = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(INDEX_SIZE))
            .ok_or_else(|| Error::new(ErrorKind::InvalidFormat, None).with_context(TRUNCATED))?;
        Ok(Self {
            indices: fields.bytes(length)?,
        })
    }

    /// Return the serialized indices.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.indices
    }

    /// Return the number of indices.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.indices.len() / INDEX_SIZE
    }

    /// Return `true` if the list has no indices.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Return an iterator over the type indices.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = TypeIndex> + 'a {
        self.indices
            .as_chunks::<INDEX_SIZE>()
            .0
            .iter()
            .map(|chunk| TypeIndex(u32::from_le_bytes(*chunk)))
    }

    /// Return the number of indices as a `u32` count field.
    pub(crate) fn count(&self) -> Result<u32> {
        u32::try_from(self.len()).map_err(|_| too_long())
    }

    /// Append a `u32` count and the indices.
    pub(crate) fn encode_counted(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.u32(self.count()?)?;
        writer.bytes(self.indices)
    }
}

/// An `LF_ARGLIST` record: the parameter types of a procedure type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ArgumentList<'a> {
    /// The type indices.
    list: TypeIndexList<'a>,
}

impl<'a> ArgumentList<'a> {
    /// Make an argument list from serialized type indices.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TypeIndexList::from_bytes`].
    pub fn from_bytes(indices: &'a [u8]) -> Result<Self> {
        Ok(Self {
            list: TypeIndexList::from_bytes(indices)?,
        })
    }

    /// Decode the payload of an `LF_ARGLIST` record.
    ///
    /// The payload has a `u32` count and then `count` type indices.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let list = TypeIndexList::decode_counted(&mut fields)?;
        fields.finish_type_padding()?;
        Ok(Self { list })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        self.list.encode_counted(writer)
    }

    /// Return the list of type indices.
    #[must_use]
    pub const fn list(&self) -> TypeIndexList<'a> {
        self.list
    }

    /// Return the number of arguments.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.list.len()
    }

    /// Return `true` if the list has no arguments.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Return an iterator over the argument types.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = TypeIndex> + 'a {
        self.list.iter()
    }
}

/// One method in an `LF_METHODLIST` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MethodListEntry {
    /// The access and the method properties.
    pub attributes: MemberAttributes,
    /// The `LF_MFUNCTION` record of the method.
    pub method_type: TypeIndex,
    /// The offset of the method in the virtual function table.
    ///
    /// This value is present only if the method introduces a virtual slot.
    pub vtable_offset: Option<i32>,
}

impl MethodListEntry {
    /// Append the entry.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the virtual table offset is
    /// present for a method that does not introduce a virtual slot, or
    /// absent for one that does.
    pub(crate) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.u16(self.attributes.bits())?;
        writer.u16(0)?;
        writer.type_index(self.method_type)?;
        encode_vtable_offset(writer, self.attributes, self.vtable_offset)
    }
}

/// Append the virtual table offset of a method, if its kind needs one.
pub(crate) fn encode_vtable_offset(
    writer: &mut Output<'_>,
    attributes: MemberAttributes,
    vtable_offset: Option<i32>,
) -> Result<()> {
    match (
        attributes.method_kind().is_introducing_virtual(),
        vtable_offset,
    ) {
        (true, Some(offset)) => writer.i32(offset),
        (false, None) => Ok(()),
        _ => Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("virtual table offset does not match the method property")),
    }
}

/// An `LF_METHODLIST` record: the overloads of one method name.
///
/// Each entry has a `u16` attribute word, a `u16` padding field that must be
/// zero, a type index and, for an introducing virtual method, an `i32`
/// virtual table offset (`MapOneMethodRecord` in LLVM).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MethodList<'a> {
    /// The serialized entries and the record padding.
    data: &'a [u8],
}

impl<'a> MethodList<'a> {
    /// Decode and validate the payload of an `LF_METHODLIST` record.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] for a malformed entry or
    /// invalid padding.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let list = Self { data };
        for entry in list.entries() {
            entry?;
        }
        Ok(list)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        for entry in self.entries() {
            entry?.encode(writer)?;
        }
        Ok(())
    }

    /// Return an iterator over the entries.
    ///
    /// [`MethodList::decode`] validates all entries before it returns the
    /// list, so the iterator does not give errors for a decoded list.
    #[must_use]
    pub const fn entries(&self) -> MethodListIter<'a> {
        MethodListIter {
            data: self.data,
            done: false,
        }
    }
}

/// An iterator over the entries of a [`MethodList`].
#[derive(Clone, Debug)]
pub struct MethodListIter<'a> {
    /// The bytes that are not read yet.
    data: &'a [u8],
    /// `true` after the end or after an error.
    done: bool,
}

impl MethodListIter<'_> {
    /// Read the next entry. Return `Ok(None)` at valid end padding.
    fn read(&mut self) -> Result<Option<MethodListEntry>> {
        let mut fields = fields(self.data);
        if self.data.len() < METHOD_ENTRY_SIZE {
            fields.finish_type_padding()?;
            return Ok(None);
        }
        let attributes = MemberAttributes::from_raw(fields.u16()?)?;
        if fields.u16()? != 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("method list entry padding is not zero"));
        }
        let method_type = fields.type_index()?;
        let vtable_offset = if attributes.method_kind().is_introducing_virtual() {
            Some(fields.i32()?)
        } else {
            None
        };
        self.data = fields.rest();
        Ok(Some(MethodListEntry {
            attributes,
            method_type,
            vtable_offset,
        }))
    }
}

impl Iterator for MethodListIter<'_> {
    type Item = Result<MethodListEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = self.read();
        if !matches!(result, Ok(Some(_))) {
            self.done = true;
        }
        result.transpose()
    }
}

impl FusedIterator for MethodListIter<'_> {}

/// An `LF_VTSHAPE` record: the slot kinds of a virtual function table.
///
/// The record has a `u16` slot count and then one 4-bit descriptor for each
/// slot. The first slot of each byte is in the high 4 bits. For an odd
/// count, the low 4 bits of the last byte must be zero.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VtableShape<'a> {
    /// The number of slots.
    count: u16,
    /// The descriptor bytes.
    descriptors: &'a [u8],
}

impl<'a> VtableShape<'a> {
    /// Make a shape from a slot count and the descriptor bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the byte count does not match
    /// the slot count, if a descriptor is unknown, or if the unused low
    /// 4 bits of an odd count are not zero.
    pub fn new(count: u16, descriptors: &'a [u8]) -> Result<Self> {
        if descriptors.len() != usize::from(count).div_ceil(2) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("virtual table shape byte count does not match its slots"));
        }
        for (position, &byte) in descriptors.iter().enumerate() {
            VtableSlotKind::from_raw(byte >> 4)?;
            let low = byte & 0x0f;
            if position * 2 + 1 < usize::from(count) {
                VtableSlotKind::from_raw(low)?;
            } else if low != 0 {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("virtual table shape has a nonzero unused descriptor"));
            }
        }
        Ok(Self { count, descriptors })
    }

    /// Decode and validate the payload of an `LF_VTSHAPE` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let count = fields.u16()?;
        let descriptors = fields.bytes(usize::from(count).div_ceil(2))?;
        fields.finish_type_padding()?;
        Self::new(count, descriptors)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.u16(self.count)?;
        writer.bytes(self.descriptors)
    }

    /// Return the number of slots.
    #[must_use]
    pub const fn len(&self) -> u16 {
        self.count
    }

    /// Return `true` if the table has no slots.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return the raw descriptor bytes.
    #[must_use]
    pub const fn descriptors(&self) -> &'a [u8] {
        self.descriptors
    }

    /// Return an iterator over the slot kinds.
    pub fn slots(&self) -> impl Iterator<Item = VtableSlotKind> + 'a {
        let descriptors = self.descriptors;
        // `new` validated each descriptor, so `filter_map` keeps all slots.
        (0..usize::from(self.count)).filter_map(move |slot| {
            let byte = descriptors.get(slot / 2)?;
            let nibble = if slot % 2 == 0 {
                byte >> 4
            } else {
                byte & 0x0f
            };
            VtableSlotKind::from_raw(nibble).ok()
        })
    }
}

/// An `LF_VFTABLE` record: a virtual function table and its method names.
///
/// The payload has the complete class, the overridden table, the offset of
/// the table pointer, a `u32` byte length and then that many bytes of
/// NUL-terminated names (`lfVftable` in `cvinfo.h`, `VFTableRecord` in
/// LLVM). The first name is the table name; the others are method names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualFunctionTable<'a> {
    /// The class or structure that owns the table.
    pub complete_class: TypeIndex,
    /// The table that this table overrides, if any.
    pub overridden_table: Option<TypeIndex>,
    /// The offset of the table pointer in the object layout.
    pub table_pointer_offset: u32,
    /// The names, each with its NUL terminator.
    names: &'a [u8],
}

impl<'a> VirtualFunctionTable<'a> {
    /// Make a table from its fields and the serialized names.
    ///
    /// `names` holds the table name and then the method names, each with its
    /// NUL terminator.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `names` does not end with a
    /// NUL byte.
    pub fn new(
        complete_class: TypeIndex,
        overridden_table: Option<TypeIndex>,
        table_pointer_offset: u32,
        names: &'a [u8],
    ) -> Result<Self> {
        if names.last() != Some(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("virtual function table names do not end with a NUL byte"));
        }
        Ok(Self {
            complete_class,
            overridden_table,
            table_pointer_offset,
            names,
        })
    }

    /// Decode and validate the payload of an `LF_VFTABLE` record.
    ///
    /// The name bytes must hold at least the table name, and every name must
    /// end with a NUL byte inside the declared length.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let complete_class = fields.type_index()?;
        let overridden = fields.type_index()?;
        let table_pointer_offset = fields.u32()?;
        let length = usize::try_from(fields.u32()?)
            .map_err(|_| Error::new(ErrorKind::InvalidFormat, None).with_context(TRUNCATED))?;
        let names = fields.bytes(length)?;
        fields.finish_type_padding()?;
        Self::new(
            complete_class,
            (overridden != TypeIndex::NONE).then_some(overridden),
            table_pointer_offset,
            names,
        )
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.complete_class)?;
        writer.type_index(self.overridden_table.unwrap_or(TypeIndex::NONE))?;
        writer.u32(self.table_pointer_offset)?;
        writer.u32(u32::try_from(self.names.len()).map_err(|_| too_long())?)?;
        writer.bytes(self.names)
    }

    /// Return the serialized names, each with its NUL terminator.
    #[must_use]
    pub const fn names(&self) -> &'a [u8] {
        self.names
    }

    /// Return the name of the table.
    #[must_use]
    pub fn table_name(&self) -> &'a [u8] {
        self.all_names().next().unwrap_or_default()
    }

    /// Return an iterator over the method names, in table order.
    pub fn method_names(&self) -> impl Iterator<Item = &'a [u8]> + 'a {
        self.all_names().skip(1)
    }

    /// Return all names, the table name first.
    fn all_names(&self) -> impl Iterator<Item = &'a [u8]> + 'a {
        // `new` makes sure that the bytes end with a NUL byte.
        let names = self.names;
        names[..names.len() - 1].split(|&byte| byte == 0)
    }
}
