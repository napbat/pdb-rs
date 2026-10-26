//! The module stream writer.
//!
//! The writer joins the regions of a module stream in the order of
//! `Mod1::fCommit` in `PDB/dbi/mod.cpp` of `microsoft/microsoft-pdb`: the
//! `CV_SIGNATURE_C13` signature and the symbol records, the C11 line region,
//! the C13 line region, the global reference size and the global
//! references. It returns the region sizes for the DBI module record.

use alloc::vec::Vec;

use super::global_refs::REF_SIZE;
use super::{ModuleRegionSizes, ModuleStream, SIGNATURE_C13, SIGNATURE_SIZE, Subsections};
use crate::native::codeview::Records;
use crate::native::symbols::SymbolOffset;
use crate::{Error, ErrorKind, Result};

/// The regions of a module stream for [`ModuleStreamParts::encode`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ModuleStreamParts<'a> {
    /// The symbol records without the signature, or `None` for a module
    /// stream without a symbol region.
    ///
    /// `Some(&[])` gives a symbol region with only the signature.
    pub symbols: Option<&'a [u8]>,
    /// The C11 line region. The writer keeps it raw.
    pub c11: &'a [u8],
    /// The C13 line region: framed subsections, for example the output of
    /// [`ModuleLines::encode`](crate::native::lines::ModuleLines::encode).
    pub c13: &'a [u8],
    /// The global references: offsets into the symbol record stream.
    pub global_refs: &'a [SymbolOffset],
}

/// A module stream from [`ModuleStreamParts::encode`] or
/// [`ModuleStream::encode`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncodedModuleStream {
    /// The module stream bytes.
    pub bytes: Vec<u8>,
    /// The region sizes for the DBI module record.
    pub sizes: ModuleRegionSizes,
}

impl ModuleStreamParts<'_> {
    /// Check the regions and join them into module stream bytes.
    ///
    /// The function checks all sizes before it allocates the output. The
    /// output has the exact size of the stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the symbol records or the C13 line
    ///   region do not end at a 4-byte boundary.
    /// - The framing errors of [`Records`] for the symbol records and of
    ///   [`ModuleStream::c13_subsections`] for the C13 line region.
    /// - [`ErrorKind::LimitExceeded`] if a region size or the stream size
    ///   does not fit in 32 bits, or if the output cannot be allocated.
    ///
    /// The errors do not have a stream location.
    pub fn encode(&self) -> Result<EncodedModuleStream> {
        write(
            self.symbols,
            self.c11,
            self.c13,
            self.global_refs.iter().copied(),
        )
    }
}

impl ModuleStream {
    /// Join the regions of this module stream again.
    ///
    /// The output is equal to the parsed bytes, and the sizes are equal to
    /// the sizes of the module record, if the regions are canonical: the
    /// symbol records and the C13 line region end at a 4-byte boundary.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ModuleStreamParts::encode`].
    pub fn encode(&self) -> Result<EncodedModuleStream> {
        let symbols = (self.symbol_offset() == SIGNATURE_SIZE).then(|| self.symbol_data());
        let refs = self.global_refs()?;
        write(symbols, self.c11_data(), self.c13_data(), refs.iter())
    }
}

/// Check the regions and join them into module stream bytes.
fn write(
    symbols: Option<&[u8]>,
    c11: &[u8],
    c13: &[u8],
    global_refs: impl ExactSizeIterator<Item = SymbolOffset>,
) -> Result<EncodedModuleStream> {
    let symbol_size = match symbols {
        None => 0,
        Some(records) => {
            if !records.len().is_multiple_of(4) {
                return Err(invalid(
                    "module symbol records do not end at a 4-byte boundary",
                ));
            }
            for record in Records::new(records) {
                record?;
            }
            SIGNATURE_SIZE + records.len()
        }
    };
    if !c13.len().is_multiple_of(4) {
        return Err(invalid(
            "module C13 line region does not end at a 4-byte boundary",
        ));
    }
    // The framing errors name a stream; the writer has no stream yet.
    let subsections = Subsections {
        bytes: c13,
        base: 0,
        stream: 0,
        position: 0,
    };
    for subsection in subsections {
        subsection.map_err(|error| {
            Error::new(error.kind(), None)
                .with_context(error.context().unwrap_or("C13 subsection is malformed"))
        })?;
    }
    let refs_size = global_refs
        .len()
        .checked_mul(REF_SIZE)
        .ok_or_else(|| too_large("module global reference size does not fit in 32 bits"))?;
    let sizes = ModuleRegionSizes {
        symbol_bytes: to_u32(
            symbol_size,
            "module symbol region size does not fit in 32 bits",
        )?,
        c11_bytes: to_u32(
            c11.len(),
            "module C11 line region size does not fit in 32 bits",
        )?,
        c13_bytes: to_u32(
            c13.len(),
            "module C13 line region size does not fit in 32 bits",
        )?,
    };
    let refs_field = to_u32(
        refs_size,
        "module global reference size does not fit in 32 bits",
    )?;
    let total = [c11.len(), c13.len(), REF_SIZE, refs_size]
        .into_iter()
        .try_fold(symbol_size, usize::checked_add)
        .filter(|&total| u32::try_from(total).is_ok())
        .ok_or_else(|| too_large("module stream size does not fit in 32 bits"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total)
        .map_err(|_| too_large("module stream cannot be allocated"))?;
    if let Some(records) = symbols {
        bytes.extend_from_slice(&SIGNATURE_C13.to_le_bytes());
        bytes.extend_from_slice(records);
    }
    bytes.extend_from_slice(c11);
    bytes.extend_from_slice(c13);
    bytes.extend_from_slice(&refs_field.to_le_bytes());
    for offset in global_refs {
        bytes.extend_from_slice(&offset.0.to_le_bytes());
    }
    Ok(EncodedModuleStream { bytes, sizes })
}

/// Convert a size to the `u32` of its field.
fn to_u32(size: usize, context: &'static str) -> Result<u32> {
    u32::try_from(size).map_err(|_| too_large(context))
}

/// Return an invalid-format error with `context`.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// Return a limit error with `context`.
fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}
