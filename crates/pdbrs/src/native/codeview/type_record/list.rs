//! Type records that hold a list: argument lists, method overload lists,
//! virtual function table shapes and virtual function table names.

use core::iter::FusedIterator;

use super::{MemberAttributes, TRUNCATED, VtableSlotKind, fields};
use crate::native::codeview::index::TypeIndex;
use crate::{Error, ErrorKind, Result};

/// The size of one type index in bytes.
const INDEX_SIZE: usize = 4;

/// The size of a method-list entry without a virtual table offset.
const METHOD_ENTRY_SIZE: usize = 8;

/// An `LF_ARGLIST` record: the parameter types of a procedure type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ArgumentList<'a> {
    /// The serialized type indices.
    indices: &'a [u8],
}

impl<'a> ArgumentList<'a> {
    /// Decode the payload of an `LF_ARGLIST` record.
    ///
    /// The payload has a `u32` count and then `count` type indices.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let count = fields.u32()?;
        let length = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(INDEX_SIZE))
            .ok_or_else(|| Error::new(ErrorKind::InvalidFormat, None).with_context(TRUNCATED))?;
        let indices = fields.bytes(length)?;
        fields.finish_type_padding()?;
        Ok(Self { indices })
    }

    /// Return the number of arguments.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.indices.len() / INDEX_SIZE
    }

    /// Return `true` if the list has no arguments.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Return an iterator over the argument types.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = TypeIndex> + 'a {
        self.indices
            .as_chunks::<INDEX_SIZE>()
            .0
            .iter()
            .map(|chunk| TypeIndex(u32::from_le_bytes(*chunk)))
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
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let list = Self { data };
        for entry in list.entries() {
            entry?;
        }
        Ok(list)
    }

    /// Return an iterator over the entries.
    ///
    /// [`super::TypeLeaf::decode`] validates all entries before it returns
    /// the list, so the iterator does not give errors for a decoded list.
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
    /// Decode and validate the payload of an `LF_VTSHAPE` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let count = fields.u16()?;
        let descriptors = fields.bytes(usize::from(count).div_ceil(2))?;
        fields.finish_type_padding()?;
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
        // `decode` validated each descriptor, so `filter_map` keeps all slots.
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
        if names.last() != Some(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("virtual function table names do not end with a NUL byte"));
        }
        Ok(Self {
            complete_class,
            overridden_table: (overridden != TypeIndex::NONE).then_some(overridden),
            table_pointer_offset,
            names,
        })
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
        // `decode` makes sure that the bytes end with a NUL byte.
        let names = self.names;
        names[..names.len() - 1].split(|&byte| byte == 0)
    }
}
