//! `DEBUG_S_MERGED_ASSEMBLYINPUT` subsections: the input assemblies of a
//! merged assembly.
//!
//! `cvinfo.h` does not define the layout. The layout comes from
//! `DumpModMergedAssemblyInput` in `cvdump/dumpsym7.cpp` of
//! `microsoft/microsoft-pdb`. Each entry has these fields:
//!
//! - A `u32` time stamp.
//! - A `u32` index. Bit 31 tells that a PDB is present.
//! - A `u16` version size, which counts the size field itself, and the
//!   version bytes.
//! - A NUL-terminated name.
//! - Zero bytes up to a 4-byte boundary.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{invalid, pad4, truncated};
use crate::Result;
use crate::native::output::Output;

/// The size of the time stamp and the index.
const FIXED_SIZE: usize = 8;

/// The size of the version size field.
const VERSION_SIZE_SIZE: usize = 2;

/// The bit of the index that tells that a PDB is present.
const PDB_BIT: u32 = 0x8000_0000;

/// The alignment of each entry.
const ALIGNMENT: usize = 4;

/// One input assembly of a merged assembly.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AssemblyInput<'a> {
    /// The time stamp.
    pub timestamp: u32,
    /// The index, bits 0 to 30.
    pub index: u32,
    /// `true` if a PDB is present (bit 31 of the index field).
    pub has_pdb: bool,
    /// The version bytes, without the size field.
    pub version: &'a [u8],
    /// The name, without its NUL terminator.
    pub name: &'a [u8],
}

impl<'a> AssemblyInput<'a> {
    /// Create an entry from its fields.
    #[must_use]
    pub const fn new(
        timestamp: u32,
        index: u32,
        has_pdb: bool,
        version: &'a [u8],
        name: &'a [u8],
    ) -> Self {
        Self {
            timestamp,
            index,
            has_pdb,
            version,
            name,
        }
    }

    /// Append the entry and its padding to `out`.
    ///
    /// The entry starts at a 4-byte boundary of the subsection data.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AssemblyInput::encode`], without padding.
    pub fn encode_padded(&self, out: &mut Vec<u8>) -> Result<()> {
        let size = self.write(out)?;
        pad4(out, size)
    }

    /// Append the entry to `out` without the padding after the name.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if
    ///   the index does not fit in 31 bits, if the version bytes with the
    ///   size field do not fit in 16 bits, or if the name contains a NUL
    ///   byte.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    ///   the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        self.write(out).map(|_| ())
    }

    /// Append the entry without padding to `out` and return its size.
    fn write(&self, out: &mut Vec<u8>) -> Result<usize> {
        if self.index & PDB_BIT != 0 {
            return Err(invalid(
                "merged assembly input index does not fit in 31 bits",
            ));
        }
        let version_size = self
            .version
            .len()
            .checked_add(VERSION_SIZE_SIZE)
            .and_then(|size| u16::try_from(size).ok())
            .ok_or_else(|| invalid("merged assembly input version does not fit in 16 bits"))?;
        if self.name.contains(&0) {
            return Err(invalid("merged assembly input name contains a NUL byte"));
        }
        let flag = if self.has_pdb { PDB_BIT } else { 0 };
        Output::new(out).u32(self.timestamp)?;
        Output::new(out).u32(self.index | flag)?;
        Output::new(out).u16(version_size)?;
        Output::new(out).bytes(self.version)?;
        Output::new(out).bytes(self.name)?;
        Output::new(out).bytes(&[0])?;
        Ok(FIXED_SIZE + usize::from(version_size) + self.name.len() + 1)
    }
}

/// A decoded `DEBUG_S_MERGED_ASSEMBLYINPUT` subsection.
///
/// [`MergedAssemblyInput::decode`] validates all entries. The entry iterator
/// then cannot fail.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MergedAssemblyInput<'a> {
    /// The validated entries.
    data: &'a [u8],
    /// The number of entries.
    count: usize,
}

impl<'a> MergedAssemblyInput<'a> {
    /// Decode the data of a `DEBUG_S_MERGED_ASSEMBLYINPUT` subsection.
    ///
    /// These rules apply:
    ///
    /// - The version size counts at least the size field.
    /// - Zero bytes pad each entry to a 4-byte boundary. The padding after
    ///   the last entry can be absent, because the subsection framing can
    ///   exclude it. A part of the padding is not valid.
    /// - The entries fill the data exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if
    ///   an entry, its name or its padding is truncated.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   version size is smaller than 2 or a padding byte is not zero.
    ///
    /// The errors do not have a file offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut rest = data;
        let mut count = 0usize;
        while !rest.is_empty() {
            rest = split_entry(rest)?.2;
            count += 1;
        }
        Ok(Self { data, count })
    }

    /// Return the number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Return `true` if the subsection has no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return an iterator over the entries.
    #[must_use]
    pub const fn iter(&self) -> MergedAssemblyInputIter<'a> {
        MergedAssemblyInputIter {
            bytes: self.data,
            left: self.count,
        }
    }

    /// Append the subsection data to `out`.
    ///
    /// Each entry gets the padding that it had in the decoded data, so the
    /// output is equal to that data.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AssemblyInput::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut rest = self.data;
        while !rest.is_empty() {
            let (entry, padding, next) = split_entry(rest)?;
            entry.encode(out)?;
            Output::new(out).bytes(&[0; ALIGNMENT - 1][..padding])?;
            rest = next;
        }
        Ok(())
    }
}

impl<'a> IntoIterator for MergedAssemblyInput<'a> {
    type Item = AssemblyInput<'a>;
    type IntoIter = MergedAssemblyInputIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &MergedAssemblyInput<'a> {
    type Item = AssemblyInput<'a>;
    type IntoIter = MergedAssemblyInputIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the entries of a [`MergedAssemblyInput`] subsection.
#[derive(Clone, Debug)]
pub struct MergedAssemblyInputIter<'a> {
    /// The remaining validated entries.
    bytes: &'a [u8],
    /// The number of remaining entries.
    left: usize,
}

impl<'a> Iterator for MergedAssemblyInputIter<'a> {
    type Item = AssemblyInput<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // `MergedAssemblyInput::decode` validated the entries, so this cannot fail.
        let (entry, _, rest) = split_entry(self.bytes).ok()?;
        self.bytes = rest;
        self.left = self.left.saturating_sub(1);
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for MergedAssemblyInputIter<'_> {}

impl FusedIterator for MergedAssemblyInputIter<'_> {}

/// Split one entry from the start of `bytes`.
///
/// Return the entry, the size of its padding and the rest.
fn split_entry(bytes: &[u8]) -> Result<(AssemblyInput<'_>, usize, &[u8])> {
    let Some((&[t0, t1, t2, t3, i0, i1, i2, i3, s0, s1], rest)) =
        bytes.split_first_chunk::<{ FIXED_SIZE + VERSION_SIZE_SIZE }>()
    else {
        return Err(truncated("merged assembly input entry is truncated"));
    };
    let version_size = usize::from(u16::from_le_bytes([s0, s1]));
    let version_size = version_size
        .checked_sub(VERSION_SIZE_SIZE)
        .ok_or_else(|| invalid("merged assembly input version size is smaller than 2"))?;
    if rest.len() < version_size {
        return Err(truncated("merged assembly input version is truncated"));
    }
    let (version, rest) = rest.split_at(version_size);
    let name_size = rest
        .iter()
        .position(|&byte| byte == 0)
        .ok_or_else(|| truncated("merged assembly input name is truncated"))?;
    let name = &rest[..name_size];
    let size = FIXED_SIZE + VERSION_SIZE_SIZE + version_size + name_size + 1;
    // The entry starts at a 4-byte boundary, so its size gives the padding.
    let tail = &rest[name_size + 1..];
    let padding = if tail.is_empty() {
        0
    } else {
        (ALIGNMENT - size % ALIGNMENT) % ALIGNMENT
    };
    let pad = tail
        .get(..padding)
        .ok_or_else(|| truncated("merged assembly input padding is truncated"))?;
    if pad.iter().any(|&byte| byte != 0) {
        return Err(invalid("merged assembly input padding is not zero"));
    }
    let index = u32::from_le_bytes([i0, i1, i2, i3]);
    let entry = AssemblyInput {
        timestamp: u32::from_le_bytes([t0, t1, t2, t3]),
        index: index & !PDB_BIT,
        has_pdb: index & PDB_BIT != 0,
        version,
        name,
    };
    Ok((entry, padding, &tail[padding..]))
}
