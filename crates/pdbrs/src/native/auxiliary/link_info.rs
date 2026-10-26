//! The `/LinkInfo` stream.
//!
//! The linker stores the data that it needs to link the image again. The layout follows
//! `struct LinkInfo` in Microsoft `langapi/include/pdb.h` and `DBI1::GetUTF8LinkInfo` in
//! Microsoft `PDB/dbi/dbi.cpp`:
//!
//! 1. Six `u32` fields: `cb`, `ver`, `offszCwd`, `offszCommand`, `ichOutfile`, `offszLibs`.
//! 2. The NUL-terminated current directory at `offszCwd`.
//! 3. The NUL-terminated linker command at `offszCommand`. The output file name starts at
//!    `ichOutfile` in the command.
//! 4. The library bytes from `offszLibs` to `cb`.
//!
//! `GetUTF8LinkInfo` writes the strings directly after each other. This reader requires that
//! layout, so each byte of the record has a field. `DBI1::AddLinkInfoW` writes the record size
//! of the caller's wide-character record, which can be larger than `cb`. The bytes after `cb`
//! stay available as [`LinkInfo::trailing`]. LLVM writes an empty stream.
//!
//! # Versions
//!
//! `pdb.h` defines `vliOne` (1) and `vliTwo` (2). Both versions use the same layout:
//! `GetUTF8LinkInfo` copies `ver` from the caller and always writes the six fields and the
//! UTF-8 strings. `DBI1::QueryLinkInfo` does not read `ver`. The wide-character form
//! (`LinkInfoW`) exists only in memory; the stream always holds the narrow form. MSVC
//! `link.exe` 14.51 writes version 2. No producer that is available writes version 1, so the
//! tests use synthetic records for it.

use alloc::vec::Vec;

use crate::native::hash_table::Cursor;
use crate::native::msf::StreamId;
use crate::native::output::{Output, to_u32};
use crate::{Error, ErrorKind, Result};

/// The record version `vliOne`.
pub const LINK_INFO_VERSION_ONE: u32 = 1;

/// The record version `vliTwo` (`vliCur`), which MSVC `link.exe` writes.
pub const LINK_INFO_VERSION_TWO: u32 = 2;

/// The size of the fixed fields in bytes (`sizeof(LinkInfo)`).
const HEADER_SIZE: u32 = 24;

/// A decoded `/LinkInfo` record, borrowed from the stream bytes.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LinkInfo<'a> {
    /// The record size `cb`.
    pub size: u32,
    /// The record version `ver`: [`LINK_INFO_VERSION_ONE`] or [`LINK_INFO_VERSION_TWO`].
    pub version: u32,
    /// The current directory of the link, without its NUL.
    pub current_directory: &'a [u8],
    /// The linker command, without its NUL.
    pub command: &'a [u8],
    /// The offset of the output file name in [`LinkInfo::command`] (`ichOutfile`).
    pub output_file_offset: u32,
    /// The library bytes from `offszLibs` to `cb`, kept raw.
    pub libraries: &'a [u8],
    /// The stream bytes after the record, kept raw.
    pub trailing: &'a [u8],
}

impl<'a> LinkInfo<'a> {
    /// Make a record with the layout of `GetUTF8LinkInfo` and no trailing bytes.
    ///
    /// The function calculates [`LinkInfo::size`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if `version` is not 1 or 2.
    /// - [`ErrorKind::InvalidFormat`] if a string contains a NUL byte or `output_file_offset`
    ///   is after the end of `command`.
    /// - [`ErrorKind::LimitExceeded`] if the record size does not fit in `u32`.
    pub fn new(
        version: u32,
        current_directory: &'a [u8],
        command: &'a [u8],
        output_file_offset: u32,
        libraries: &'a [u8],
    ) -> Result<Self> {
        let mut record = Self {
            size: 0,
            version,
            current_directory,
            command,
            output_file_offset,
            libraries,
            trailing: &[],
        };
        record.size = record.layout()?[3];
        Ok(record)
    }

    /// Decode the `/LinkInfo` stream `stream`. Returns `Ok(None)` for an empty stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if `ver` is not 1 or 2.
    /// - [`ErrorKind::UnexpectedEof`] if the stream is shorter than the fixed fields or `cb`.
    /// - [`ErrorKind::InvalidFormat`] if an offset does not match the layout, a string has no
    ///   NUL inside the record, or `ichOutfile` is after the end of the command.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Option<Self>> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let mut cursor = Cursor::new(bytes, stream, "/LinkInfo record");
        let size = cursor.u32()?;
        let version = cursor.u32()?;
        let directory_offset = cursor.u32()?;
        let command_offset = cursor.u32()?;
        let output_file_offset = cursor.u32()?;
        let libraries_offset = cursor.u32()?;
        if !matches!(version, LINK_INFO_VERSION_ONE | LINK_INFO_VERSION_TWO) {
            return Err(cursor.fail(ErrorKind::UnsupportedVersion, 4, "/LinkInfo version"));
        }
        if size < HEADER_SIZE {
            return Err(cursor.fail(ErrorKind::InvalidFormat, 0, "/LinkInfo record size"));
        }
        let record_len = usize::try_from(size).unwrap_or(usize::MAX);
        let record = bytes.get(..record_len).ok_or_else(|| {
            cursor.fail(
                ErrorKind::UnexpectedEof,
                bytes.len(),
                "/LinkInfo record extends past the stream",
            )
        })?;
        let trailing = &bytes[record_len..];
        if directory_offset != HEADER_SIZE {
            return Err(cursor.fail(ErrorKind::InvalidFormat, 8, "/LinkInfo directory offset"));
        }
        let current_directory = string(&cursor, record, directory_offset)?;
        if Some(command_offset) != next(directory_offset, current_directory) {
            return Err(cursor.fail(ErrorKind::InvalidFormat, 12, "/LinkInfo command offset"));
        }
        let command = string(&cursor, record, command_offset)?;
        if Some(libraries_offset) != next(command_offset, command) {
            return Err(cursor.fail(ErrorKind::InvalidFormat, 20, "/LinkInfo library offset"));
        }
        if usize::try_from(output_file_offset).map_or(true, |offset| offset > command.len()) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                16,
                "/LinkInfo output file offset is after the command",
            ));
        }
        let libraries = record
            .get(usize::try_from(libraries_offset).unwrap_or(usize::MAX)..)
            .ok_or_else(|| cursor.fail(ErrorKind::InvalidFormat, 20, "/LinkInfo library offset"))?;
        Ok(Some(Self {
            size,
            version,
            current_directory,
            command,
            output_file_offset,
            libraries,
            trailing,
        }))
    }

    /// Return the output file name: the command bytes from [`LinkInfo::output_file_offset`].
    #[must_use]
    pub fn output_file(&self) -> &'a [u8] {
        let start = usize::try_from(self.output_file_offset).unwrap_or(usize::MAX);
        self.command.get(start..).unwrap_or_default()
    }

    /// Append the record and the trailing bytes to `out`.
    ///
    /// The function writes the layout of `GetUTF8LinkInfo`. A decoded stream gives the same
    /// bytes again. An empty `/LinkInfo` stream has no record: do not call this function for
    /// it.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if [`LinkInfo::version`] is not 1 or 2.
    /// - [`ErrorKind::InvalidFormat`] if a string contains a NUL byte, the output file offset
    ///   is after the end of the command, or [`LinkInfo::size`] differs from the size of the
    ///   fields.
    /// - [`ErrorKind::LimitExceeded`] if an offset does not fit in `u32` or the output cannot
    ///   grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let [directory, command, libraries, size] = self.layout()?;
        if size != self.size {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("/LinkInfo record size differs from the size of its fields"));
        }
        let mut output = Output::new(out);
        output.reserve(
            usize::try_from(size)
                .unwrap_or(usize::MAX)
                .saturating_add(self.trailing.len()),
        )?;
        for word in [
            size,
            self.version,
            directory,
            command,
            self.output_file_offset,
            libraries,
        ] {
            output.u32(word)?;
        }
        output.c_string(self.current_directory)?;
        output.c_string(self.command)?;
        output.bytes(self.libraries)?;
        output.bytes(self.trailing)
    }

    /// Check the fields and return the directory, command and library offsets and `cb`.
    fn layout(&self) -> Result<[u32; 4]> {
        if !matches!(self.version, LINK_INFO_VERSION_ONE | LINK_INFO_VERSION_TWO) {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("/LinkInfo version is not 1 or 2"));
        }
        let invalid = |context| Error::new(ErrorKind::InvalidFormat, None).with_context(context);
        if self.current_directory.contains(&0) || self.command.contains(&0) {
            return Err(invalid("/LinkInfo string contains a NUL byte"));
        }
        if usize::try_from(self.output_file_offset).map_or(true, |at| at > self.command.len()) {
            return Err(invalid("/LinkInfo output file offset is after the command"));
        }
        let too_large = || {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("/LinkInfo record size overflows u32")
        };
        let command = next(HEADER_SIZE, self.current_directory).ok_or_else(too_large)?;
        let libraries = next(command, self.command).ok_or_else(too_large)?;
        let size = libraries
            .checked_add(to_u32(self.libraries.len(), "/LinkInfo library size")?)
            .ok_or_else(too_large)?;
        Ok([HEADER_SIZE, command, libraries, size])
    }
}

/// Return the NUL-terminated string at `offset` in `record`, without its NUL.
fn string<'a>(cursor: &Cursor<'_>, record: &'a [u8], offset: u32) -> Result<&'a [u8]> {
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    let suffix = record.get(start..).unwrap_or_default();
    let length = suffix.iter().position(|&byte| byte == 0).ok_or_else(|| {
        cursor.fail(
            ErrorKind::InvalidFormat,
            start.min(record.len()),
            "/LinkInfo string has no NUL inside the record",
        )
    })?;
    Ok(&suffix[..length])
}

/// Return the offset after the string at `offset` and its NUL.
fn next(offset: u32, string: &[u8]) -> Option<u32> {
    u32::try_from(string.len())
        .ok()?
        .checked_add(1)?
        .checked_add(offset)
}
