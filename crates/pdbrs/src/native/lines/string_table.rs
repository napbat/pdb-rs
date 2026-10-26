//! `DEBUG_S_STRINGTABLE` subsections: the strings of an object file.
//!
//! The data is a sequence of NUL-terminated strings, as in LLVM
//! `DebugStringTableSubsection`. Other subsections refer to a string by the
//! byte offset of its first byte. In object files the table holds the file
//! names of the file checksum table. `link.exe` moves the strings to the
//! `/names` stream (`Mod1::processC13Strings` in `PDB/dbi/mod.cpp`).

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{invalid, put};
use crate::Result;

/// A decoded `DEBUG_S_STRINGTABLE` subsection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StringTableSubsection<'a> {
    /// The strings with their NUL terminators.
    data: &'a [u8],
}

impl<'a> StringTableSubsection<'a> {
    /// Decode the data of a `DEBUG_S_STRINGTABLE` subsection.
    ///
    /// The data is empty or ends with a NUL byte. LLVM writes an empty
    /// string first, but the decoder does not require it. Zero padding at
    /// the end gives empty strings.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat)
    /// if the last string has no NUL terminator. The error does not have a
    /// file offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        if data.last().is_some_and(|&byte| byte != 0) {
            return Err(invalid("string table does not end with a NUL byte"));
        }
        Ok(Self { data })
    }

    /// Return the size of the table in bytes.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.data.len()
    }

    /// Return the string that starts at `offset`, without its NUL
    /// terminator.
    ///
    /// An offset inside a string gives the rest of that string, as in
    /// `DebugStringTableSubsectionRef::getString` of LLVM. Return `None` if
    /// `offset` is not inside the table.
    #[must_use]
    pub fn get(&self, offset: u32) -> Option<&'a [u8]> {
        let rest = self.data.get(usize::try_from(offset).ok()?..)?;
        // `decode` checked that the table ends with a NUL byte.
        let end = rest.iter().position(|&byte| byte == 0)?;
        Some(&rest[..end])
    }

    /// Return an iterator over the strings and their offsets, in table
    /// order.
    #[must_use]
    pub const fn iter(&self) -> StringTableIter<'a> {
        StringTableIter {
            data: self.data,
            position: 0,
        }
    }

    /// Append the subsection data to `out`: each string with its NUL
    /// terminator.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`StringTableSubsection::encode_string`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for (_, string) in self {
            Self::encode_string(string, out)?;
        }
        Ok(())
    }

    /// Append one string and its NUL terminator to `out`.
    ///
    /// The string starts at the offset `out.len()` minus the start of the
    /// table. Other subsections refer to it with that offset.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if
    ///   `string` contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    ///   the output cannot grow.
    pub fn encode_string(string: &[u8], out: &mut Vec<u8>) -> Result<()> {
        if string.contains(&0) {
            return Err(invalid("string table string contains a NUL byte"));
        }
        put(out, string)?;
        put(out, &[0])
    }
}

impl<'a> IntoIterator for StringTableSubsection<'a> {
    type Item = (u32, &'a [u8]);
    type IntoIter = StringTableIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &StringTableSubsection<'a> {
    type Item = (u32, &'a [u8]);
    type IntoIter = StringTableIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the strings of a [`StringTableSubsection`].
///
/// Each item is the offset of a string and the string without its NUL
/// terminator. The iteration stops at an offset that does not fit in
/// 32 bits.
#[derive(Clone, Debug)]
pub struct StringTableIter<'a> {
    /// The table.
    data: &'a [u8],
    /// The offset of the next string.
    position: usize,
}

impl<'a> Iterator for StringTableIter<'a> {
    type Item = (u32, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self
            .data
            .get(self.position..)
            .filter(|rest| !rest.is_empty())?;
        let offset = u32::try_from(self.position).ok()?;
        // `decode` checked that the table ends with a NUL byte.
        let end = rest.iter().position(|&byte| byte == 0)?;
        self.position += end + 1;
        Some((offset, &rest[..end]))
    }
}

impl FusedIterator for StringTableIter<'_> {}
