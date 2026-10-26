//! A walk over a module symbol stream that checks the scope links.
//!
//! Procedures, blocks, thunks and inline sites start a scope. `S_END` or
//! `S_PROC_ID_END` ends a procedure, block or thunk scope. `S_INLINESITE_END`
//! ends an inline site scope. These rules follow `symbolOpensScope` and
//! `symbolEndsScope` in LLVM `SymbolRecordHelpers.h`.
//!
//! The parent and end links are offsets in the module stream. A module
//! stream starts with a 4-byte signature, so the offset of a record is its
//! offset in the symbol records plus a base offset of 4.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::super::{Record, Records};
use super::{S_INLINESITE, S_INLINESITE_END, Symbol, SymbolData};
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// Kinds that start a scope but that this crate does not decode.
///
/// The list comes from `symbolOpensScope` and `getScopeEndOffset` in LLVM:
/// `S_SEPCODE`, `S_LPROC32_DPC`, `S_LPROC32_DPC_ID` and `S_INLINESITE2`.
const UNDECODED_SCOPE_KINDS: [u16; 4] = [0x1132, 0x1155, 0x1156, 0x115d];

/// A symbol with its place in the scope tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ScopedSymbol<'a> {
    /// The offset of the record in the module stream.
    pub offset: u32,
    /// The number of open scopes that contain the record.
    ///
    /// A record that ends a scope belongs to the scope that it ends.
    pub depth: usize,
    /// The module stream offset of the innermost scope that contains the
    /// record, or `None` at the top level.
    ///
    /// A record that ends a scope gives the scope that it ends.
    pub scope: Option<u32>,
    /// The decoded symbol.
    pub symbol: Symbol<'a>,
}

/// An open scope on the stack of [`ModuleSymbols`].
#[derive(Clone, Copy, Debug)]
struct OpenScope {
    /// The module stream offset of the record that starts the scope.
    offset: u32,
    /// The end link of that record.
    end: u32,
    /// `true` for an inline site scope.
    inline: bool,
}

/// An iterator that decodes module symbols and checks their scope links.
///
/// For each record that starts a scope, the iterator makes sure that:
///
/// - The parent link is the offset of the enclosing scope, or 0 at the top level.
/// - The nesting depth stays at or below the configured limit.
/// - The matching end record has the correct kind and is at the end link.
///
/// The iterator does not check the `next` link of procedures and thunks.
///
/// Each error has the stream ID and the module stream offset of the record
/// that caused it. Read them with [`Error::stream_location`]. For a wrong end
/// link or a scope without an end record, the offset identifies the record
/// that starts the scope. After an error, the iterator stops.
#[derive(Clone, Debug)]
pub struct ModuleSymbols<'a> {
    /// The record framing.
    records: Records<'a>,
    /// The module stream ID for errors.
    stream: u32,
    /// The module stream offset of the first record.
    base: u32,
    /// The largest number of open scopes.
    max_depth: usize,
    /// The open scopes, innermost last.
    stack: Vec<OpenScope>,
    /// `true` after an error or after the last record.
    done: bool,
}

impl<'a> ModuleSymbols<'a> {
    /// Create an iterator over the module symbol records in `symbols`.
    ///
    /// `symbols` holds the records without the signature, and `base` is the
    /// module stream offset of the first record: 4 if the signature is
    /// present. `stream` identifies the module stream in errors.
    /// `max_depth` limits the number of open scopes.
    #[must_use]
    pub const fn new(symbols: &'a [u8], stream: StreamId, base: u32, max_depth: usize) -> Self {
        Self {
            records: Records::new(symbols),
            stream: stream.0,
            base,
            max_depth,
            stack: Vec::new(),
            done: false,
        }
    }

    /// Create an error at a module stream offset.
    fn error(&self, kind: ErrorKind, offset: u32, context: &'static str) -> Error {
        Error::new(kind, None)
            .with_context(context)
            .with_stream(self.stream, u64::from(offset))
    }

    /// Return the module stream offset of `record`.
    fn offset(&self, record: Record<'_>) -> Result<u32> {
        self.base.checked_add(record.offset).ok_or_else(|| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("module symbol offset does not fit in 32 bits")
                .with_stream(self.stream, u64::from(self.base))
        })
    }

    /// Decode `record` and update the scope stack.
    fn step(&mut self, record: Record<'a>) -> Result<ScopedSymbol<'a>> {
        let offset = self.offset(record)?;
        let symbol = record
            .symbol()
            .map_err(|error| error.with_stream(self.stream, u64::from(offset)))?;
        let depth = self.stack.len();
        let scope = self.stack.last().map(|open| open.offset);
        if let Some(links) = symbol.scope() {
            if links.parent != scope.unwrap_or(0) {
                return Err(self.error(
                    ErrorKind::InvalidReference,
                    offset,
                    "symbol scope parent is not the enclosing scope",
                ));
            }
            if links.end <= offset {
                return Err(self.error(
                    ErrorKind::InvalidReference,
                    offset,
                    "symbol scope end is not after its start",
                ));
            }
            if depth >= self.max_depth {
                return Err(self.error(
                    ErrorKind::LimitExceeded,
                    offset,
                    "symbol scope nesting exceeds the depth limit",
                ));
            }
            self.stack.try_reserve(1).map_err(|_| {
                self.error(
                    ErrorKind::LimitExceeded,
                    offset,
                    "cannot allocate the symbol scope stack",
                )
            })?;
            self.stack.push(OpenScope {
                offset,
                end: links.end,
                inline: symbol.kind() == S_INLINESITE,
            });
        } else if matches!(
            symbol.data(),
            SymbolData::ScopeEnd | SymbolData::InlineSiteEnd
        ) {
            self.close(offset, symbol.kind() == S_INLINESITE_END)?;
        } else if UNDECODED_SCOPE_KINDS.contains(&symbol.kind()) {
            return Err(self.error(
                ErrorKind::UnsupportedVersion,
                offset,
                "symbol kind starts a scope that this crate does not decode",
            ));
        }
        Ok(ScopedSymbol {
            offset,
            depth,
            scope,
            symbol,
        })
    }

    /// Close the innermost scope with the end record at `offset`.
    fn close(&mut self, offset: u32, inline: bool) -> Result<()> {
        let Some(open) = self.stack.pop() else {
            return Err(self.error(
                ErrorKind::InvalidFormat,
                offset,
                "symbol scope end has no open scope",
            ));
        };
        if open.inline != inline {
            return Err(self.error(
                ErrorKind::InvalidFormat,
                offset,
                "symbol scope end kind does not match its scope",
            ));
        }
        if open.end != offset {
            return Err(self.error(
                ErrorKind::InvalidReference,
                open.offset,
                "symbol scope end link does not identify its end record",
            ));
        }
        Ok(())
    }
}

impl<'a> Iterator for ModuleSymbols<'a> {
    type Item = Result<ScopedSymbol<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = match self.records.next() {
            None => {
                self.done = true;
                let open = self.stack.last()?;
                return Some(Err(self.error(
                    ErrorKind::InvalidFormat,
                    open.offset,
                    "symbol scope has no end record",
                )));
            }
            Some(Err(error)) => {
                // A slice length always fits in `u64` on supported targets.
                let position = u64::try_from(self.records.position())
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::from(self.base));
                Err(error.with_stream(self.stream, position))
            }
            Some(Ok(record)) => self.step(record),
        };
        self.done = result.is_err();
        Some(result)
    }
}

impl FusedIterator for ModuleSymbols<'_> {}
