//! User-defined type records: classes, structures, interfaces, unions and
//! enumerations.
//!
//! These records end with a name. If [`ClassOptions::has_unique_name`] is
//! `true`, a unique (decorated) name follows the name (`mapNameAndUniqueName`
//! in LLVM `TypeRecordMapping.cpp`).
//!
//! The wide kinds `LF_CLASS2`, `LF_STRUCTURE2`, `LF_INTERFACE2` and
//! `LF_UNION2` have a `u32` property word and a numeric member count (LLVM
//! `TypeRecordMapping.cpp`). `cvinfo.h` does not define them.
//!
//! Microsoft Macro Assembler (`ml64` 14.50) writes `f1` and two stale bytes
//! after an odd-length name, instead of the `LF_PAD` run `f3 f2 f1`, for
//! example `f1 6d 65` after `CsFrameEncoded` in `libvcruntime.lib`. `link.exe`
//! and `lld-link` copy these records to the PDB. LLVM, DIA and `pdbtool` read
//! them; LLVM does not check the bytes after the last field
//! (`CodeViewRecordIO::endRecord`). The decoders keep these bytes in the
//! `trailing` field, and the encoders write them again.

use super::{ClassOptions, fields};
use crate::bytes::into_owned;
use crate::native::codeview::Fields;
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::numeric::Numeric;
use crate::native::codeview::writer::Padding;
use crate::native::output::Output;
use crate::{Bytes, Error, ErrorKind, Result};

/// The options and the names of a user-defined type.
///
/// LLVM calls the records with these fields tag records. Type hashing and
/// forward-reference resolution use only these fields.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Tag<'a> {
    /// The properties of the type.
    pub options: ClassOptions,
    /// The name bytes.
    pub name: Bytes<'a>,
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<Bytes<'a>>,
}

into_owned!(struct Tag { name, unique_name; options });

/// Return a tag that borrows the names of a record.
fn borrowed_tag<'b>(
    options: ClassOptions,
    name: &'b [u8],
    unique_name: Option<&'b [u8]>,
) -> Tag<'b> {
    Tag {
        options,
        name: Bytes::new(name),
        unique_name: unique_name.map(Bytes::new),
    }
}

/// The name, the unique name and the producer padding at the end of a UDT record.
type Names<'a> = (&'a [u8], Option<&'a [u8]>, &'a [u8]);

/// Return `true` for the MASM padding form: 1 to 3 bytes, and the first byte is an
/// `LF_PAD` byte (`0xf1` to `0xf3`).
const fn is_producer_padding(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        [0xf1..=0xf3] | [0xf1..=0xf3, _] | [0xf1..=0xf3, _, _]
    )
}

/// Read the name, the optional unique name and the padding at the end of a record.
///
/// The standard `LF_PAD` run gives empty producer padding. Bytes in the MASM form give
/// the producer padding if the record of the `length` payload bytes ends on a 4-byte boundary.
/// Other bytes give the error of [`Fields::finish_type_padding`].
fn names<'a>(fields: &mut Fields<'a>, length: usize, options: ClassOptions) -> Result<Names<'a>> {
    let name = fields.name()?;
    let unique_name = if options.has_unique_name() {
        Some(fields.name()?)
    } else {
        None
    };
    let producer_padding = match fields.finish_type_padding() {
        Ok(()) => &[][..],
        // The MASM form ends the record on a 4-byte boundary, as the encoder requires.
        Err(_) if is_producer_padding(fields.rest()) && length.is_multiple_of(4) => fields.rest(),
        Err(error) => return Err(error),
    };
    Ok((name, unique_name, producer_padding))
}

/// Append the name, the unique name and the producer padding, and return the padding rule.
///
/// The unique name must be present exactly if the options have the
/// unique-name flag. Empty producer padding gives the standard `LF_PAD` run.
/// Other producer padding must be in the MASM form and end the record on a
/// 4-byte boundary.
fn encode_names(
    writer: &mut Output<'_>,
    options: ClassOptions,
    (name, unique_name, producer_padding): Names<'_>,
) -> Result<Padding> {
    if options.has_unique_name() != unique_name.is_some() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("unique name does not match the unique-name flag"));
    }
    writer.c_string(name)?;
    if let Some(unique_name) = unique_name {
        writer.c_string(unique_name)?;
    }
    if producer_padding.is_empty() {
        return Ok(Padding::Type);
    }
    if !is_producer_padding(producer_padding) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("producer padding is not 1 to 3 bytes that start with LF_PAD"));
    }
    writer.bytes(producer_padding)?;
    Ok(Padding::Aligned)
}

/// Return the low 16 bits of a wide property word.
const fn low_options(options: u32) -> ClassOptions {
    let [low, high, ..] = options.to_le_bytes();
    ClassOptions(u16::from_le_bytes([low, high]))
}

/// The leaf kind of a [`Class`] or [`WideClass`] record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ClassKind {
    /// `LF_CLASS` or `LF_CLASS2`.
    Class,
    /// `LF_STRUCTURE` or `LF_STRUCTURE2`.
    Structure,
    /// `LF_INTERFACE` or `LF_INTERFACE2`.
    Interface,
}

/// An `LF_CLASS`, `LF_STRUCTURE` or `LF_INTERFACE` record.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Class<'a> {
    /// The leaf kind.
    pub kind: ClassKind,
    /// The number of members in the field list.
    pub member_count: u16,
    /// The properties of the type.
    pub options: ClassOptions,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The derivation list, usually [`TypeIndex::NONE`].
    pub derivation_list: TypeIndex,
    /// The `LF_VTSHAPE` record, or [`TypeIndex::NONE`].
    pub vtable_shape: TypeIndex,
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: Bytes<'a>,
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<Bytes<'a>>,
    /// The bytes after the names if they are not the standard `LF_PAD` run, or an empty slice.
    ///
    /// Only the MASM form is valid: 1 to 3 bytes that start with an `LF_PAD` byte, for
    /// example `f1 6d 65`. The encoder writes these bytes instead of the `LF_PAD` run.
    pub trailing: Bytes<'a>,
}

impl<'a> Class<'a> {
    /// Make a record with `kind`. The other fields are zero or empty.
    #[must_use]
    pub fn new(kind: ClassKind) -> Self {
        Self {
            kind,
            member_count: 0,
            options: ClassOptions(0),
            field_list: TypeIndex::NONE,
            derivation_list: TypeIndex::NONE,
            vtable_shape: TypeIndex::NONE,
            size: Numeric::default(),
            name: Bytes::default(),
            unique_name: None,
            trailing: Bytes::default(),
        }
    }

    /// Decode the payload of a class, structure or interface record.
    pub(super) fn decode(kind: ClassKind, data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let member_count = fields.u16()?;
        let options = ClassOptions(fields.u16()?);
        let field_list = fields.type_index()?;
        let derivation_list = fields.type_index()?;
        let vtable_shape = fields.type_index()?;
        let size = fields.numeric()?;
        let (name, unique_name, producer_padding) = names(&mut fields, data.len(), options)?;
        Ok(Self {
            kind,
            member_count,
            options,
            field_list,
            derivation_list,
            vtable_shape,
            size,
            name: Bytes::new(name),
            unique_name: unique_name.map(Bytes::new),
            trailing: Bytes::new(producer_padding),
        })
    }

    /// Append the payload fields and return the padding rule.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<Padding> {
        writer.u16(self.member_count)?;
        writer.u16(self.options.0)?;
        writer.type_index(self.field_list)?;
        writer.type_index(self.derivation_list)?;
        writer.type_index(self.vtable_shape)?;
        self.size.encode(writer.out())?;
        encode_names(writer, self.options, self.names())
    }

    /// Return the names and the producer padding.
    fn names(&self) -> Names<'_> {
        (&self.name, self.unique_name.as_deref(), &self.trailing)
    }

    /// Return the options and a view of the names.
    #[must_use]
    pub fn tag(&self) -> Tag<'_> {
        borrowed_tag(self.options, &self.name, self.unique_name.as_deref())
    }
}

into_owned!(struct Class {
    size, name, unique_name, trailing;
    kind, member_count, options, field_list, derivation_list, vtable_shape
});

/// An `LF_CLASS2`, `LF_STRUCTURE2` or `LF_INTERFACE2` record.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct WideClass<'a> {
    /// The leaf kind.
    pub kind: ClassKind,
    /// The 32-bit property word. The low 16 bits are [`ClassOptions`].
    pub options: u32,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The derivation list, usually [`TypeIndex::NONE`].
    pub derivation_list: TypeIndex,
    /// The `LF_VTSHAPE` record, or [`TypeIndex::NONE`].
    pub vtable_shape: TypeIndex,
    /// The number of members in the field list.
    pub member_count: Numeric<'a>,
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: Bytes<'a>,
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<Bytes<'a>>,
    /// The bytes after the names if they are not the standard `LF_PAD` run, or an empty slice.
    ///
    /// Only the MASM form is valid: 1 to 3 bytes that start with an `LF_PAD` byte, for
    /// example `f1 6d 65`. The encoder writes these bytes instead of the `LF_PAD` run.
    pub trailing: Bytes<'a>,
}

impl<'a> WideClass<'a> {
    /// Make a record with `kind`. The other fields are zero or empty.
    #[must_use]
    pub fn new(kind: ClassKind) -> Self {
        Self {
            kind,
            options: 0,
            field_list: TypeIndex::NONE,
            derivation_list: TypeIndex::NONE,
            vtable_shape: TypeIndex::NONE,
            member_count: Numeric::default(),
            size: Numeric::default(),
            name: Bytes::default(),
            unique_name: None,
            trailing: Bytes::default(),
        }
    }

    /// Decode the payload of a wide class, structure or interface record.
    pub(super) fn decode(kind: ClassKind, data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let options = fields.u32()?;
        let field_list = fields.type_index()?;
        let derivation_list = fields.type_index()?;
        let vtable_shape = fields.type_index()?;
        let member_count = fields.numeric()?;
        let size = fields.numeric()?;
        let (name, unique_name, producer_padding) =
            names(&mut fields, data.len(), low_options(options))?;
        Ok(Self {
            kind,
            options,
            field_list,
            derivation_list,
            vtable_shape,
            member_count,
            size,
            name: Bytes::new(name),
            unique_name: unique_name.map(Bytes::new),
            trailing: Bytes::new(producer_padding),
        })
    }

    /// Append the payload fields and return the padding rule.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<Padding> {
        writer.u32(self.options)?;
        writer.type_index(self.field_list)?;
        writer.type_index(self.derivation_list)?;
        writer.type_index(self.vtable_shape)?;
        self.member_count.encode(writer.out())?;
        self.size.encode(writer.out())?;
        encode_names(writer, low_options(self.options), self.names())
    }

    /// Return the names and the producer padding.
    fn names(&self) -> Names<'_> {
        (&self.name, self.unique_name.as_deref(), &self.trailing)
    }

    /// Return the low 16 bits of the options and a view of the names.
    #[must_use]
    pub fn tag(&self) -> Tag<'_> {
        borrowed_tag(
            low_options(self.options),
            &self.name,
            self.unique_name.as_deref(),
        )
    }
}

into_owned!(struct WideClass {
    member_count, size, name, unique_name, trailing;
    kind, options, field_list, derivation_list, vtable_shape
});

/// An `LF_UNION` record.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Union<'a> {
    /// The number of members in the field list.
    pub member_count: u16,
    /// The properties of the type.
    pub options: ClassOptions,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: Bytes<'a>,
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<Bytes<'a>>,
    /// The bytes after the names if they are not the standard `LF_PAD` run, or an empty slice.
    ///
    /// Only the MASM form is valid: 1 to 3 bytes that start with an `LF_PAD` byte, for
    /// example `f1 6d 65`. The encoder writes these bytes instead of the `LF_PAD` run.
    pub trailing: Bytes<'a>,
}

impl<'a> Union<'a> {
    /// Decode the payload of an `LF_UNION` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let member_count = fields.u16()?;
        let options = ClassOptions(fields.u16()?);
        let field_list = fields.type_index()?;
        let size = fields.numeric()?;
        let (name, unique_name, producer_padding) = names(&mut fields, data.len(), options)?;
        Ok(Self {
            member_count,
            options,
            field_list,
            size,
            name: Bytes::new(name),
            unique_name: unique_name.map(Bytes::new),
            trailing: Bytes::new(producer_padding),
        })
    }

    /// Append the payload fields and return the padding rule.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<Padding> {
        writer.u16(self.member_count)?;
        writer.u16(self.options.0)?;
        writer.type_index(self.field_list)?;
        self.size.encode(writer.out())?;
        encode_names(writer, self.options, self.names())
    }

    /// Return the names and the producer padding.
    fn names(&self) -> Names<'_> {
        (&self.name, self.unique_name.as_deref(), &self.trailing)
    }

    /// Return the options and a view of the names.
    #[must_use]
    pub fn tag(&self) -> Tag<'_> {
        borrowed_tag(self.options, &self.name, self.unique_name.as_deref())
    }
}

into_owned!(struct Union { size, name, unique_name, trailing; member_count, options, field_list });

/// An `LF_UNION2` record.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct WideUnion<'a> {
    /// The 32-bit property word. The low 16 bits are [`ClassOptions`].
    pub options: u32,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The number of members in the field list.
    pub member_count: Numeric<'a>,
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: Bytes<'a>,
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<Bytes<'a>>,
    /// The bytes after the names if they are not the standard `LF_PAD` run, or an empty slice.
    ///
    /// Only the MASM form is valid: 1 to 3 bytes that start with an `LF_PAD` byte, for
    /// example `f1 6d 65`. The encoder writes these bytes instead of the `LF_PAD` run.
    pub trailing: Bytes<'a>,
}

impl<'a> WideUnion<'a> {
    /// Decode the payload of an `LF_UNION2` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let options = fields.u32()?;
        let field_list = fields.type_index()?;
        let member_count = fields.numeric()?;
        let size = fields.numeric()?;
        let (name, unique_name, producer_padding) =
            names(&mut fields, data.len(), low_options(options))?;
        Ok(Self {
            options,
            field_list,
            member_count,
            size,
            name: Bytes::new(name),
            unique_name: unique_name.map(Bytes::new),
            trailing: Bytes::new(producer_padding),
        })
    }

    /// Append the payload fields and return the padding rule.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<Padding> {
        writer.u32(self.options)?;
        writer.type_index(self.field_list)?;
        self.member_count.encode(writer.out())?;
        self.size.encode(writer.out())?;
        encode_names(writer, low_options(self.options), self.names())
    }

    /// Return the names and the producer padding.
    fn names(&self) -> Names<'_> {
        (&self.name, self.unique_name.as_deref(), &self.trailing)
    }

    /// Return the low 16 bits of the options and a view of the names.
    #[must_use]
    pub fn tag(&self) -> Tag<'_> {
        borrowed_tag(
            low_options(self.options),
            &self.name,
            self.unique_name.as_deref(),
        )
    }
}

into_owned!(struct WideUnion {
    member_count, size, name, unique_name, trailing;
    options, field_list
});

/// An `LF_ENUM` record.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Enum<'a> {
    /// The number of enumerators in the field list.
    pub member_count: u16,
    /// The properties of the type.
    pub options: ClassOptions,
    /// The integer type of the enumerator values.
    pub underlying_type: TypeIndex,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The name bytes.
    pub name: Bytes<'a>,
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<Bytes<'a>>,
    /// The bytes after the names if they are not the standard `LF_PAD` run, or an empty slice.
    ///
    /// Only the MASM form is valid: 1 to 3 bytes that start with an `LF_PAD` byte, for
    /// example `f1 6d 65`. The encoder writes these bytes instead of the `LF_PAD` run.
    pub trailing: Bytes<'a>,
}

impl<'a> Enum<'a> {
    /// Decode the payload of an `LF_ENUM` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let member_count = fields.u16()?;
        let options = ClassOptions(fields.u16()?);
        let underlying_type = fields.type_index()?;
        let field_list = fields.type_index()?;
        let (name, unique_name, producer_padding) = names(&mut fields, data.len(), options)?;
        Ok(Self {
            member_count,
            options,
            underlying_type,
            field_list,
            name: Bytes::new(name),
            unique_name: unique_name.map(Bytes::new),
            trailing: Bytes::new(producer_padding),
        })
    }

    /// Append the payload fields and return the padding rule.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<Padding> {
        writer.u16(self.member_count)?;
        writer.u16(self.options.0)?;
        writer.type_index(self.underlying_type)?;
        writer.type_index(self.field_list)?;
        encode_names(writer, self.options, self.names())
    }

    /// Return the names and the producer padding.
    fn names(&self) -> Names<'_> {
        (&self.name, self.unique_name.as_deref(), &self.trailing)
    }

    /// Return the options and a view of the names.
    #[must_use]
    pub fn tag(&self) -> Tag<'_> {
        borrowed_tag(self.options, &self.name, self.unique_name.as_deref())
    }
}

into_owned!(struct Enum {
    name, unique_name, trailing;
    member_count, options, underlying_type, field_list
});
