//! A writer of module symbol records that sets the scope links.
//!
//! [`ModuleSymbolWriter`] encodes records in stream order. For each record
//! that starts a scope, it writes the parent link: the offset of the
//! enclosing scope record, or 0 at the top level. When the matching end
//! record arrives, it writes the end link of the scope record. The links of
//! the given [`SymbolData`] values are ignored. All scope records start
//! with the parent and end links (`pParent` and `pEnd` in `cvinfo.h`).
//!
//! The `next` link of procedures and thunks keeps its given value. The MSVC,
//! LLVM and .NET producers of the fixtures write 0 (#30).

use alloc::vec::Vec;

use super::super::{S_INLINESITE, S_INLINESITE_END, S_INLINESITE2, SymbolData};
use crate::{Error, ErrorKind, Result};

/// The byte offset of the parent link in a record, after the length and kind.
const PARENT_FIELD: usize = 4;
/// The byte offset of the end link in a record.
const END_FIELD: usize = 8;

/// An open scope of a [`ModuleSymbolWriter`].
#[derive(Clone, Copy, Debug)]
struct Open {
    /// The position of the scope record in the output.
    position: usize,
    /// The module stream offset of the scope record.
    offset: u32,
    /// `true` for an inline site.
    inline: bool,
}

/// A writer of the symbol records of a module stream.
///
/// The output does not include the 4-byte signature. Give it to
/// [`ModuleStreamParts::symbols`](crate::native::dbi::ModuleStreamParts::symbols).
#[derive(Clone, Debug, Default)]
pub struct ModuleSymbolWriter {
    /// The encoded records.
    out: Vec<u8>,
    /// The module stream offset of the first record.
    base: u32,
    /// The open scopes, innermost last.
    stack: Vec<Open>,
}

impl ModuleSymbolWriter {
    /// Create a writer whose first record is at module stream offset `base`.
    ///
    /// A module stream starts with a 4-byte signature, so `base` is
    /// usually 4.
    #[must_use]
    pub const fn new(base: u32) -> Self {
        Self {
            out: Vec::new(),
            base,
            stack: Vec::new(),
        }
    }

    /// Return the module stream offset of the next record.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::OffsetOverflow`] if the offset does not fit in
    /// 32 bits.
    pub fn next_offset(&self) -> Result<u32> {
        u32::try_from(self.out.len())
            .ok()
            .and_then(|length| self.base.checked_add(length))
            .ok_or_else(|| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("module symbol offset does not fit in 32 bits")
            })
    }

    /// Encode a record of `kind` and return its module stream offset.
    ///
    /// A scope record gets its parent link now and its end link at its end
    /// record.
    ///
    /// # Errors
    ///
    /// - The errors of [`SymbolData::encode`].
    /// - [`ErrorKind::OffsetOverflow`] if the offset does not fit in 32 bits.
    /// - [`ErrorKind::InvalidFormat`] for an end record without an open
    ///   scope, or an end kind that does not match its scope.
    ///
    /// After an error, the writer is unchanged.
    pub fn push(&mut self, kind: u16, data: &SymbolData<'_>) -> Result<u32> {
        let offset = self.next_offset()?;
        let position = self.out.len();
        let starts_scope = data.scope_links().is_some();
        let ends_scope = matches!(data, SymbolData::ScopeEnd | SymbolData::InlineSiteEnd);
        if ends_scope {
            let inline = kind == S_INLINESITE_END;
            match self.stack.last() {
                Some(open) if open.inline == inline => {}
                Some(_) => return Err(invalid("symbol scope end kind does not match its scope")),
                None => return Err(invalid("symbol scope end has no open scope")),
            }
        }
        if starts_scope {
            self.stack.try_reserve(1).map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("cannot allocate the symbol scope stack")
            })?;
        }
        if let Err(error) = data.encode(kind, &mut self.out) {
            self.out.truncate(position);
            return Err(error);
        }
        if starts_scope {
            let parent = self.stack.last().map_or(0, |open| open.offset);
            self.patch(position + PARENT_FIELD, parent);
            self.patch(position + END_FIELD, 0);
            self.stack.push(Open {
                position,
                offset,
                inline: matches!(kind, S_INLINESITE | S_INLINESITE2),
            });
        } else if ends_scope && let Some(open) = self.stack.pop() {
            self.patch(open.position + END_FIELD, offset);
        }
        Ok(offset)
    }

    /// Return the records.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if a scope has no end record.
    pub fn finish(self) -> Result<Vec<u8>> {
        if self.stack.is_empty() {
            Ok(self.out)
        } else {
            Err(invalid("symbol scope has no end record"))
        }
    }

    /// Write a `u32` at `position` of the output.
    fn patch(&mut self, position: usize, value: u32) {
        self.out[position..position + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// Return an [`ErrorKind::InvalidFormat`] error.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}
