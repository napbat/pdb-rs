//! A symbol record of the editor model.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::native::codeview::SymbolKind;
use crate::native::codeview::index::ModuleSymbolOffset;
use crate::native::codeview::symbol::{ModuleSymbolWriter, Symbol, SymbolData};
use crate::native::output::no_memory;
use crate::{Bytes, Error, ErrorKind, Result};

/// The state of a record.
#[derive(Clone, Debug, Eq, PartialEq)]
enum State {
    /// A source record that the caller did not access for a change.
    Source(Symbol<'static>),
    /// A record that the caller made or accessed for a change.
    Fields {
        /// The record kind.
        kind: SymbolKind,
        /// The fields.
        data: SymbolData<'static>,
        /// The source record, if the record comes from the source.
        source: Option<Box<Symbol<'static>>>,
    },
}

/// A symbol record of the editor model: a kind and its fields.
///
/// A record from the source keeps its source bytes. While the caller does
/// not change the kind or the fields, or changes them back, the plan writes
/// the source bytes with their padding. Otherwise the plan encodes the
/// fields with the encoder of the kind. A record that starts a scope gets
/// its links from the scope tree of its module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolRecord {
    /// The kind and the fields.
    state: State,
    /// The number of the record in its module, for a
    /// [`ModuleSymbolHandle`](crate::native::codeview::index::ModuleSymbolHandle).
    id: Option<u32>,
}

impl SymbolRecord {
    /// Make a record of `kind` with `data`.
    #[must_use]
    pub fn new(kind: SymbolKind, data: SymbolData<'_>) -> Self {
        Self {
            state: State::Fields {
                kind,
                data: data.into_owned(),
                source: None,
            },
            id: None,
        }
    }

    /// Make a record from a decoded source record.
    pub(super) const fn from_source(symbol: Symbol<'static>) -> Self {
        Self {
            state: State::Source(symbol),
            id: None,
        }
    }

    /// Return the record kind.
    #[must_use]
    pub const fn kind(&self) -> SymbolKind {
        match &self.state {
            State::Source(symbol) => symbol.kind(),
            State::Fields { kind, .. } => *kind,
        }
    }

    /// Return the fields.
    #[must_use]
    pub const fn data(&self) -> &SymbolData<'static> {
        match &self.state {
            State::Source(symbol) => symbol.data(),
            State::Fields { data, .. } => data,
        }
    }

    /// Return the fields for a change.
    pub fn data_mut(&mut self) -> &mut SymbolData<'static> {
        self.make_fields();
        match &mut self.state {
            State::Fields { data, .. } => data,
            // `make_fields` leaves no source state; this arm keeps the match total.
            State::Source(symbol) => symbol.data_mut(),
        }
    }

    /// Replace the kind of the record.
    ///
    /// The fields must belong to the new kind, for example
    /// [`SymbolData::Data`] for [`S_GDATA32`](crate::native::codeview::symbol::S_GDATA32)
    /// and [`S_LDATA32`](crate::native::codeview::symbol::S_LDATA32). The plan
    /// rejects other fields with [`ErrorKind::InvalidFormat`].
    pub fn set_kind(&mut self, kind: SymbolKind) {
        self.make_fields();
        if let State::Fields { kind: current, .. } = &mut self.state {
            *current = kind;
        }
    }

    /// Change a source record to the field state and keep the source record.
    fn make_fields(&mut self) {
        if let State::Source(symbol) = &self.state {
            let kind = symbol.kind();
            let data = symbol.data().clone();
            let State::Source(symbol) = core::mem::replace(
                &mut self.state,
                State::Fields {
                    kind,
                    data,
                    source: None,
                },
            ) else {
                return;
            };
            if let State::Fields { source, .. } = &mut self.state {
                *source = Some(Box::new(symbol));
            }
        }
    }

    /// Return the source record, also after a change.
    pub(super) fn origin(&self) -> Option<&Symbol<'static>> {
        match &self.state {
            State::Source(symbol) => Some(symbol),
            State::Fields { source, .. } => source.as_deref(),
        }
    }

    /// Return the source record if the record has its kind and its fields.
    pub(super) fn unchanged(&self) -> Option<&Symbol<'static>> {
        match &self.state {
            State::Source(symbol) => Some(symbol),
            State::Fields {
                kind,
                data,
                source: Some(source),
            } if *kind == source.kind() && data == source.data() => Some(source),
            State::Fields { .. } => None,
        }
    }

    /// Forget the source record: the plan encodes the fields.
    pub(super) fn forget_source(&mut self) {
        self.make_fields();
        if let State::Fields { source, .. } = &mut self.state {
            *source = None;
        }
    }

    /// Return the number of the record in its module.
    pub(super) const fn id(&self) -> Option<u32> {
        self.id
    }

    /// Set the number of the record in its module.
    pub(super) const fn set_id(&mut self, id: u32) {
        self.id = Some(id);
    }

    /// Return the name of the record, if its kind has a name.
    pub(super) fn name(&self) -> Option<&[u8]> {
        match &self.state {
            State::Source(symbol) => symbol.name().map(Bytes::as_bytes),
            State::Fields { data, .. } => data.name().map(Bytes::as_bytes),
        }
    }

    /// Append the record to a module stream writer and return its offset.
    pub(super) fn push_module(
        &self,
        writer: &mut ModuleSymbolWriter,
    ) -> Result<ModuleSymbolOffset> {
        match self.unchanged() {
            Some(symbol) => writer.push_symbol(symbol),
            None => writer.push(self.kind(), self.data()),
        }
    }

    /// Append the record to `out`. A source record without a change gives
    /// its source bytes.
    pub(super) fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        match self.unchanged() {
            Some(symbol) => append_source(symbol, out),
            None => self.data().encode(self.kind(), out),
        }
    }
}

/// Append the bytes of a source record to `out`: the length, the kind and
/// the payload with its padding.
pub(super) fn append_source(symbol: &Symbol<'_>, out: &mut Vec<u8>) -> Result<()> {
    let record = symbol.record();
    let length = u16::try_from(record.data.len() + 2).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("CodeView record is longer than its u16 length field allows")
    })?;
    out.try_reserve(record.data.len() + 4)
        .map_err(|_| no_memory())?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&record.kind.to_le_bytes());
    out.extend_from_slice(record.data);
    Ok(())
}
