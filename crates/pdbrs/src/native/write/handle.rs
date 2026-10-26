//! Handles of builder items and the resolver that converts them.
//!
//! A handle is a position in a [`PdbBuilder`](super::PdbBuilder). It is not
//! an on-disk value. The write plan assigns the on-disk values and gives a
//! [`Resolver`] to each deferred record.

use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::msf::StreamId;
use crate::native::symbols::SymbolOffset;
use crate::{Error, ErrorKind, Result};

/// A type record of the TPI stream before it has a type index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypeHandle(pub(super) u32);

/// An ID record of the IPI stream before it has an ID index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdHandle(pub(super) u32);

/// A stream without a name before it has a stream ID.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StreamHandle(pub(super) u32);

/// A module of the DBI stream.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModuleHandle(pub(super) u32);

/// A record of the symbol record stream before it has a [`SymbolOffset`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolHandle(pub(super) u32);

/// A symbol record of a module stream before it has a module stream offset.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModuleSymbolHandle {
    /// The module.
    pub(super) module: u32,
    /// The position of the record in the module.
    pub(super) position: u32,
}

impl ModuleSymbolHandle {
    /// Return the module of the record.
    #[must_use]
    pub const fn module(self) -> ModuleHandle {
        ModuleHandle(self.module)
    }
}

/// Return the error for a handle that the resolver cannot convert.
pub(super) fn unresolved(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// Return the index of position `position` in a type stream.
pub(super) fn record_index(position: u32, context: &'static str) -> Result<u32> {
    FIRST_RECORD_INDEX
        .checked_add(position)
        .ok_or_else(|| Error::new(ErrorKind::LimitExceeded, None).with_context(context))
}

/// The on-disk values that the write plan assigned.
///
/// A deferred record gets a resolver when the plan encodes it. The values
/// that a later planning step assigns are not available yet: for example,
/// a module symbol record cannot resolve a [`SymbolHandle`], because the
/// plan encodes the symbol record stream after the module streams.
#[derive(Clone, Copy, Debug)]
pub struct Resolver<'p> {
    /// The number of TPI records.
    pub(super) type_count: u32,
    /// The number of IPI records.
    pub(super) id_count: u32,
    /// The number of modules.
    pub(super) module_count: u32,
    /// The stream ID of each stream without a name.
    pub(super) streams: &'p [StreamId],
    /// The module stream offset of each symbol record of each module, after
    /// the plan encoded the module streams.
    pub(super) module_offsets: Option<&'p [alloc::vec::Vec<u32>]>,
    /// The offset of each record of the symbol record stream, after the plan
    /// encoded that stream.
    pub(super) symbol_offsets: Option<&'p [SymbolOffset]>,
}

impl Resolver<'_> {
    /// Return the type index of `handle`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder has no type
    /// record with this handle.
    pub fn type_index(&self, handle: TypeHandle) -> Result<TypeIndex> {
        if handle.0 >= self.type_count {
            return Err(unresolved("type handle has no record in the builder"));
        }
        Ok(TypeIndex(record_index(
            handle.0,
            "type index does not fit in 32 bits",
        )?))
    }

    /// Return the ID index of `handle`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder has no ID
    /// record with this handle.
    pub fn id_index(&self, handle: IdHandle) -> Result<IdIndex> {
        if handle.0 >= self.id_count {
            return Err(unresolved("ID handle has no record in the builder"));
        }
        Ok(IdIndex(record_index(
            handle.0,
            "ID index does not fit in 32 bits",
        )?))
    }

    /// Return the stream ID of a stream without a name.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder has no stream
    /// with this handle.
    pub fn stream(&self, handle: StreamHandle) -> Result<StreamId> {
        usize::try_from(handle.0)
            .ok()
            .and_then(|position| self.streams.get(position))
            .copied()
            .ok_or_else(|| unresolved("stream handle has no stream in the builder"))
    }

    /// Return the module number of `handle`: the module index plus 1.
    ///
    /// Symbol records such as `S_PROCREF` store this number.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the builder has no module with
    ///   this handle.
    /// - [`ErrorKind::LimitExceeded`] if the number does not fit in 16 bits.
    pub fn module_number(&self, handle: ModuleHandle) -> Result<u16> {
        if handle.0 >= self.module_count {
            return Err(unresolved("module handle has no module in the builder"));
        }
        handle
            .0
            .checked_add(1)
            .and_then(|number| u16::try_from(number).ok())
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("module number does not fit in 16 bits")
            })
    }

    /// Return the module stream offset of a module symbol record.
    ///
    /// The offset includes the 4-byte signature of the module stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder has no such
    /// record, or if the plan did not encode the module streams yet.
    pub fn module_symbol_offset(&self, handle: ModuleSymbolHandle) -> Result<u32> {
        let offsets = self.module_offsets.ok_or_else(|| {
            unresolved("module symbol offsets are not available in this planning step")
        })?;
        usize::try_from(handle.module)
            .ok()
            .and_then(|module| offsets.get(module))
            .and_then(|module| module.get(usize::try_from(handle.position).ok()?))
            .copied()
            .ok_or_else(|| unresolved("module symbol handle has no record in the builder"))
    }

    /// Return the offset of a record in the symbol record stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder has no such
    /// record, or if the plan did not encode the symbol record stream yet.
    pub fn symbol_offset(&self, handle: SymbolHandle) -> Result<SymbolOffset> {
        let offsets = self.symbol_offsets.ok_or_else(|| {
            unresolved("symbol record offsets are not available in this planning step")
        })?;
        usize::try_from(handle.0)
            .ok()
            .and_then(|position| offsets.get(position))
            .copied()
            .ok_or_else(|| unresolved("symbol handle has no record in the builder"))
    }
}
