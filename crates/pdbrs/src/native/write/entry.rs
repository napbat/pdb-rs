//! The records that a builder keeps until the write plan encodes them.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::handle::Resolver;
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::symbol::{ModuleSymbolWriter, Symbol, SymbolData};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::lines::LineSubsection;
use crate::native::output::no_memory;
use crate::native::symbols::{SymbolOffset, SymbolRecordWriter};
use crate::{Error, ErrorKind, Result};

/// A function that encodes one item when the write plan resolves its handles.
///
/// The function gets a [`Resolver`] and appends the encoded item to the
/// vector. Use the encoder of the item family, for example
/// [`TypeLeaf::encode`]. The plan calls the function once.
///
/// The item is one record for a type, an ID or a symbol, one complete C13
/// subsection with its header and padding, or the bytes of a stream.
pub type Deferred<'a> = Box<dyn Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a>;

/// A decoded value or a deferred encoder.
pub(super) enum Entry<'a, T> {
    /// A value with final indices.
    Value(T),
    /// An encoder that resolves handles.
    Deferred(Deferred<'a>),
}

impl<T> core::fmt::Debug for Entry<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Value(_) => "Value",
            Self::Deferred(_) => "Deferred",
        })
    }
}

/// A symbol record of a module or of the symbol record stream.
pub(super) enum SymbolEntry<'a> {
    /// A decoded record. The encoder keeps its padding.
    Decoded(Symbol<'a>),
    /// A kind and its fields.
    Fields(u16, SymbolData<'a>),
    /// An encoder that resolves handles.
    Deferred(Deferred<'a>),
}

impl core::fmt::Debug for SymbolEntry<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decoded(symbol) => write!(formatter, "Decoded({:#06x})", symbol.kind()),
            Self::Fields(kind, _) => write!(formatter, "Fields({kind:#06x})"),
            Self::Deferred(_) => formatter.write_str("Deferred"),
        }
    }
}

/// Return the error for an encoder that did not append exactly one record.
fn not_one_record() -> Error {
    Error::new(ErrorKind::InvalidFormat, None)
        .with_context("deferred record encoder did not append exactly one record")
}

/// Call `encode` and check that it appended exactly one framed record.
///
/// Return the bytes of the record.
pub(super) fn deferred_record(encode: &Deferred<'_>, resolver: &Resolver<'_>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode(resolver, &mut bytes)?;
    let mut records = Records::new(&bytes);
    match (records.next(), records.next()) {
        (Some(Ok(_)), None) => Ok(bytes),
        (Some(Err(error)), _) => Err(error),
        _ => Err(not_one_record()),
    }
}

/// Append a type record to `out`.
pub(super) fn encode_type(
    entry: &Entry<'_, TypeLeaf<'_>>,
    resolver: &Resolver<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    match entry {
        Entry::Value(leaf) => leaf.encode(out),
        Entry::Deferred(encode) => append(out, &deferred_record(encode, resolver)?),
    }
}

/// Append an ID record to `out`.
pub(super) fn encode_id(
    entry: &Entry<'_, IdRecord<'_>>,
    resolver: &Resolver<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    match entry {
        Entry::Value(record) => record.encode(out),
        Entry::Deferred(encode) => append(out, &deferred_record(encode, resolver)?),
    }
}

/// Append bytes with a checked allocation.
pub(super) fn append(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    out.try_reserve(bytes.len()).map_err(|_| no_memory())?;
    out.extend_from_slice(bytes);
    Ok(())
}

/// Decode the single record in `bytes` as a symbol.
fn decode_one(bytes: &[u8]) -> Result<Symbol<'_>> {
    Records::new(bytes)
        .next()
        .ok_or_else(not_one_record)??
        .symbol()
}

impl SymbolEntry<'_> {
    /// Encode the record with the scope links of `writer` and return its
    /// module stream offset.
    pub(super) fn push_module(
        &self,
        writer: &mut ModuleSymbolWriter,
        resolver: &Resolver<'_>,
    ) -> Result<u32> {
        match self {
            Self::Decoded(symbol) => writer.push_symbol(symbol),
            Self::Fields(kind, data) => writer.push(*kind, data),
            Self::Deferred(encode) => {
                let bytes = deferred_record(encode, resolver)?;
                writer.push_symbol(&decode_one(&bytes)?)
            }
        }
    }

    /// Encode the record in the symbol record stream and return its offset.
    pub(super) fn push_shared(
        &self,
        writer: &mut SymbolRecordWriter,
        resolver: &Resolver<'_>,
    ) -> Result<SymbolOffset> {
        match self {
            Self::Decoded(symbol) => writer.push_symbol(symbol),
            Self::Fields(kind, data) => writer.push(*kind, data),
            Self::Deferred(encode) => {
                let bytes = deferred_record(encode, resolver)?;
                writer.push_symbol(&decode_one(&bytes)?)
            }
        }
    }
}

/// Append a C13 subsection with its header and padding to `out`.
pub(super) fn encode_subsection(
    entry: &Entry<'_, LineSubsection<'_>>,
    resolver: &Resolver<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    match entry {
        Entry::Value(subsection) => subsection.encode_framed(out),
        Entry::Deferred(encode) => encode(resolver, out),
    }
}
