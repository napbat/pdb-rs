//! `DEBUG_S_COFF_SYMBOL_RVA` subsections: the RVAs of the COFF symbols of an
//! object file.
//!
//! The subsection is an array of `u32` RVAs without a header, as in LLVM
//! `DebugSymbolRVASubsection.h`. The MSVC FASTLINK fixture of #12 shows the
//! meaning (#13): entry `i` is the image RVA of record `i` of the COFF symbol
//! table of the object file. The value is 0 for a record without an image
//! address, for example an absolute static symbol or an auxiliary record.
//! The value is 0xffffffff for an external symbol that the linker resolved
//! to an absolute value, for example `__guard_flags`.

use alloc::vec::Vec;

use super::{entry_list, invalid, put_u32};
use crate::Result;

/// The size of one entry.
const RVA_SIZE: usize = 4;

entry_list!(
    /// The RVAs of a [`DEBUG_S_COFF_SYMBOL_RVA`](super::DEBUG_S_COFF_SYMBOL_RVA)
    /// subsection, one for each COFF symbol table record of the object file.
    CoffSymbolRvas,
    /// An iterator over [`CoffSymbolRvas`].
    CoffSymbolRvasIter,
    u32,
    RVA_SIZE,
    u32::from_le_bytes
);

impl<'a> CoffSymbolRvas<'a> {
    /// Decode the data of a subsection.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat)
    /// if the data size is not a multiple of 4. The error does not have a
    /// file offset.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        if !data.len().is_multiple_of(RVA_SIZE) {
            return Err(invalid(
                "COFF symbol RVA subsection size is not a multiple of 4",
            ));
        }
        Ok(Self::new(data))
    }

    /// Append the subsection data to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        write_rvas(self.iter(), out)
    }
}

/// Append the data of a `DEBUG_S_COFF_SYMBOL_RVA` subsection to `out`: one
/// RVA for each COFF symbol table record of the object file.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
/// the output cannot grow.
pub fn encode_coff_symbol_rvas(rvas: &[u32], out: &mut Vec<u8>) -> Result<()> {
    write_rvas(rvas.iter().copied(), out)
}

/// Append each RVA of `rvas` to `out`.
fn write_rvas(rvas: impl Iterator<Item = u32>, out: &mut Vec<u8>) -> Result<()> {
    for rva in rvas {
        put_u32(out, rva)?;
    }
    Ok(())
}
