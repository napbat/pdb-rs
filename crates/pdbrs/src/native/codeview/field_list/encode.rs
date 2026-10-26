//! Encoders of field-list members, and the split of a long member sequence
//! into `LF_FIELDLIST` records with `LF_INDEX` continuations.

use alloc::vec::Vec;

use super::{
    FieldList, LF_BCLASS, LF_BINTERFACE, LF_ENUMERATE, LF_FRIENDCLS, LF_FRIENDFCN, LF_INDEX,
    LF_IVBCLASS, LF_MEMBER, LF_MEMBERMODIFY, LF_METHOD, LF_NESTTYPE, LF_NESTTYPEEX, LF_ONEMETHOD,
    LF_STMEMBER, LF_VBCLASS, LF_VFUNCOFF, LF_VFUNCTAB, Member,
};
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::type_record::{LF_FIELDLIST, encode_vtable_offset};
use crate::native::codeview::writer::Padding;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The largest record size of LLVM `ContinuationRecordBuilder`
/// (`MaxRecordLength`, `0xff00`).
///
/// The size counts the length field, the kind field, the members and the
/// `LF_INDEX` continuation.
pub const LLVM_FIELD_LIST_LIMIT: usize = 0xff00;

/// The size of the length field and the kind field of a record.
const RECORD_PREFIX: usize = 4;

/// The size of an `LF_INDEX` member.
const CONTINUATION_SIZE: usize = 8;

impl Member<'_> {
    /// Return the member kind.
    #[must_use]
    pub const fn kind(&self) -> u16 {
        match self {
            Self::BaseClass(_) => LF_BCLASS,
            Self::BaseInterface(_) => LF_BINTERFACE,
            Self::VirtualBaseClass(base) => {
                if base.indirect {
                    LF_IVBCLASS
                } else {
                    LF_VBCLASS
                }
            }
            Self::Continuation(_) => LF_INDEX,
            Self::VirtualFunctionTable(_) => LF_VFUNCTAB,
            Self::VirtualFunctionOffset(_) => LF_VFUNCOFF,
            Self::FriendClass(_) => LF_FRIENDCLS,
            Self::FriendFunction(_) => LF_FRIENDFCN,
            Self::Enumerator(_) => LF_ENUMERATE,
            Self::DataMember(_) => LF_MEMBER,
            Self::StaticMember(_) => LF_STMEMBER,
            Self::OverloadedMethod(_) => LF_METHOD,
            Self::NestedType(_) => LF_NESTTYPE,
            Self::NestedTypeEx(_) => LF_NESTTYPEEX,
            Self::MemberModify(_) => LF_MEMBERMODIFY,
            Self::OneMethod(_) => LF_ONEMETHOD,
        }
    }

    /// Append the member and its `LF_PAD` padding to a multiple of 4 bytes.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if `out` cannot grow.
    /// - [`ErrorKind::InvalidFormat`] if a name contains a NUL byte, or if a
    ///   virtual table offset does not match the method property.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut writer = Output::new(out);
        self.encode_into(&mut writer)
    }

    /// Append the member and its padding with `writer`.
    pub(crate) fn encode_into(&self, writer: &mut Output<'_>) -> Result<()> {
        let start = writer.position();
        writer.u16(self.kind())?;
        self.encode_fields(writer)?;
        writer.pad(start, Padding::Type)
    }

    /// Append the fields after the kind.
    fn encode_fields(&self, writer: &mut Output<'_>) -> Result<()> {
        match self {
            Self::BaseClass(base) | Self::BaseInterface(base) => {
                writer.u16(base.attributes.bits())?;
                writer.type_index(base.base_type)?;
                base.offset.encode(writer.out())
            }
            Self::VirtualBaseClass(base) => {
                writer.u16(base.attributes.bits())?;
                writer.type_index(base.base_type)?;
                writer.type_index(base.vbptr_type)?;
                base.vbptr_offset.encode(writer.out())?;
                base.vbtable_index.encode(writer.out())
            }
            Self::Continuation(index)
            | Self::VirtualFunctionTable(index)
            | Self::FriendClass(index) => {
                writer.u16(0)?;
                writer.type_index(*index)
            }
            Self::VirtualFunctionOffset(value) => {
                writer.u16(0)?;
                writer.type_index(value.pointer_type)?;
                writer.i32(value.offset)
            }
            Self::FriendFunction(value) => {
                writer.u16(0)?;
                writer.type_index(value.function_type)?;
                writer.c_string(value.name)
            }
            Self::Enumerator(value) => {
                writer.u16(value.attributes.bits())?;
                value.value.encode(writer.out())?;
                writer.c_string(value.name)
            }
            Self::DataMember(value) => {
                writer.u16(value.attributes.bits())?;
                writer.type_index(value.member_type)?;
                value.offset.encode(writer.out())?;
                writer.c_string(value.name)
            }
            Self::StaticMember(value) => {
                writer.u16(value.attributes.bits())?;
                writer.type_index(value.member_type)?;
                writer.c_string(value.name)
            }
            Self::OverloadedMethod(value) => {
                writer.u16(value.overload_count)?;
                writer.type_index(value.method_list)?;
                writer.c_string(value.name)
            }
            Self::NestedType(value) => {
                writer.u16(0)?;
                writer.type_index(value.nested_type)?;
                writer.c_string(value.name)
            }
            Self::NestedTypeEx(value) => {
                writer.u16(value.attributes.bits())?;
                writer.type_index(value.nested_type)?;
                writer.c_string(value.name)
            }
            Self::MemberModify(value) => {
                writer.u16(value.attributes.bits())?;
                writer.type_index(value.base_type)?;
                writer.c_string(value.name)
            }
            Self::OneMethod(value) => {
                writer.u16(value.attributes.bits())?;
                writer.type_index(value.method_type)?;
                encode_vtable_offset(writer, value.attributes, value.vtable_offset)?;
                writer.c_string(value.name)
            }
        }
    }
}

impl FieldList<'_> {
    /// Append the members of a decoded list, each with its padding.
    pub(crate) fn encode_members(&self, writer: &mut Output<'_>) -> Result<()> {
        for member in self.members() {
            member?.encode_into(writer)?;
        }
        Ok(())
    }
}

/// The records that [`encode_field_list`] appended.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FieldListRecords {
    /// The index of the first member: the record that a type refers to.
    pub head: TypeIndex,
    /// The number of records.
    pub count: u32,
}

/// Append a member sequence as one or more `LF_FIELDLIST` records.
///
/// The first record that this function appends gets `first_index`, and each
/// next record gets the next index. The members fill records in order. A
/// record ends before a member that makes the record longer than
/// `max_record_size` minus the 8 bytes of an `LF_INDEX` member. The
/// record of the first members is the head. It comes last and has the
/// highest index, because a record can only refer to a record before it.
/// Each record except the last segment ends with an `LF_INDEX` member that
/// refers to the record with the next members (LLVM
/// `ContinuationRecordBuilder`). Microsoft compilers use the same order.
///
/// Use [`LLVM_FIELD_LIST_LIMIT`] for the LLVM limit.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if a member is an `LF_INDEX` member, or
///   for the errors of [`Member::encode`].
/// - [`ErrorKind::LimitExceeded`] if one member does not fit a record of
///   `max_record_size` bytes, if `max_record_size` is larger than a record
///   can be, if the indices overflow, or if `out` cannot grow. `out` can then
///   contain a part of the records.
pub fn encode_field_list<'m, I>(
    members: I,
    first_index: TypeIndex,
    max_record_size: usize,
    out: &mut Vec<u8>,
) -> Result<FieldListRecords>
where
    I: IntoIterator<Item = &'m Member<'m>>,
{
    if max_record_size > usize::from(u16::MAX) + 2
        || max_record_size < RECORD_PREFIX + CONTINUATION_SIZE
    {
        return Err(Error::new(ErrorKind::LimitExceeded, None)
            .with_context("field list record size limit is not valid"));
    }
    let segment_limit = max_record_size - CONTINUATION_SIZE;
    // Encode all members once, and keep the end of each segment.
    let mut encoded = Vec::new();
    let mut ends: Vec<usize> = Vec::new();
    let mut segment_start = 0;
    {
        let mut writer = Output::new(&mut encoded);
        for member in members {
            if matches!(member, Member::Continuation(_)) {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("field list members contain an LF_INDEX member"));
            }
            let before = writer.len();
            member.encode_into(&mut writer)?;
            if RECORD_PREFIX + writer.len() - before > segment_limit {
                return Err(Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("field list member does not fit one record"));
            }
            if RECORD_PREFIX + writer.len() - segment_start > segment_limit {
                ends.try_reserve(1).map_err(|_| no_memory())?;
                ends.push(before);
                segment_start = before;
            }
        }
        ends.try_reserve(1).map_err(|_| no_memory())?;
        ends.push(writer.len());
    }
    let count = u32::try_from(ends.len()).map_err(|_| index_overflow())?;
    // Write the last segment first. Each next record refers to the record
    // before it.
    let mut writer = Output::new(out);
    let mut previous: Option<TypeIndex> = None;
    for (position, &end) in ends.iter().enumerate().rev() {
        let start = position.checked_sub(1).map_or(0, |before| ends[before]);
        let record = writer.begin_record(LF_FIELDLIST)?;
        writer.bytes(&encoded[start..end])?;
        if let Some(previous) = previous {
            Member::Continuation(previous).encode_into(&mut writer)?;
        }
        writer.finish_record(record, Padding::Type)?;
        previous = Some(match previous {
            Some(TypeIndex(value)) => TypeIndex(value.checked_add(1).ok_or_else(index_overflow)?),
            None => first_index,
        });
    }
    Ok(FieldListRecords {
        head: previous.unwrap_or(first_index),
        count,
    })
}

/// Return the error for an allocation that failed.
fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("field list cannot be allocated")
}

/// Return the error for a type index that does not fit 32 bits.
fn index_overflow() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("field list type index overflows")
}
