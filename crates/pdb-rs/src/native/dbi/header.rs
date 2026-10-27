//! The 64-byte header of the DBI stream.
//!
//! The layout is `NewDBIHdr` in Microsoft `PDB/dbi/dbi.h` and
//! `DbiStreamHeader` in LLVM `RawTypes.h`. The header also holds the sizes of
//! the substreams. [`DbiHeader`] does not store the sizes: the encoder
//! gets them from the substreams that follow the header.

use alloc::vec::Vec;

use super::{
    FIXED_SIZE, MODERN_SIGNATURE, SIZE_FIELDS, SUBSTREAM_COUNT, VERSION_V70, stream_id,
    stream_index, u16_at, u32_at,
};
use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::native::section::Machine;
use crate::{Error, ErrorKind, Result};

/// The decoded fields of a modern DBI header, without the substream sizes.
///
/// The signature is always `-1` and the version is always `V70` (19990903).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DbiHeader {
    /// The PDB revision counter.
    pub age: u32,
    /// The global symbol stream, if present.
    pub global_stream: Option<StreamId>,
    /// The bit field with the toolchain version.
    pub build_number: u16,
    /// The public symbol stream, if present.
    pub public_stream: Option<StreamId>,
    /// The version of the PDB writer library.
    pub pdb_dll_version: u16,
    /// The symbol record stream, if present.
    pub symbol_stream: Option<StreamId>,
    /// The rebuild number of the PDB writer library.
    pub pdb_dll_rebuild: u16,
    /// The index of the MFC type server in the type server map.
    pub mfc_type_server_index: u32,
    /// The build flags: bit 0 incremental link, bit 1 stripped, bit 2 C types.
    ///
    /// The other bits are unused. The decoder keeps them as stored.
    pub flags: u16,
    /// The `IMAGE_FILE_MACHINE` value.
    pub machine: Machine,
    /// The reserved `u32` at offset 60.
    pub reserved: u32,
}

impl DbiHeader {
    /// Return `true` if bit 0 of the flags (`fIncLink`) is set: the linker linked incrementally.
    #[must_use]
    pub const fn incrementally_linked(&self) -> bool {
        self.flags & 1 != 0
    }

    /// Return `true` if bit 1 of the flags (`fStripped`) is set.
    ///
    /// `link /PDBSTRIPPED` sets this bit. A stripped PDB has no private symbols.
    #[must_use]
    pub const fn stripped(&self) -> bool {
        self.flags & (1 << 1) != 0
    }

    /// Return `true` if bit 2 of the flags (`fCTypes`) is set: the PDB has conflicting types.
    #[must_use]
    pub const fn has_c_types(&self) -> bool {
        self.flags & (1 << 2) != 0
    }

    /// Return `true` if bit 15 of the build number is set: the new version format.
    #[must_use]
    pub const fn new_version_format(&self) -> bool {
        self.build_number & 0x8000 != 0
    }

    /// Return the major toolchain version in bits 8 to 14 of the build number.
    #[must_use]
    pub const fn build_major(&self) -> u8 {
        self.build_number.to_le_bytes()[1] & 0x7f
    }

    /// Return the minor toolchain version in bits 0 to 7 of the build number.
    #[must_use]
    pub const fn build_minor(&self) -> u8 {
        self.build_number.to_le_bytes()[0]
    }

    /// Decode the fields of a header with a checked signature and version.
    pub(super) fn decode(fixed: &[u8; FIXED_SIZE]) -> Self {
        Self {
            age: u32_at(fixed, 8),
            global_stream: stream_id(u16_at(fixed, 12)),
            build_number: u16_at(fixed, 14),
            public_stream: stream_id(u16_at(fixed, 16)),
            pdb_dll_version: u16_at(fixed, 18),
            symbol_stream: stream_id(u16_at(fixed, 20)),
            pdb_dll_rebuild: u16_at(fixed, 22),
            mfc_type_server_index: u32_at(fixed, 44),
            flags: u16_at(fixed, 56),
            machine: Machine(u16_at(fixed, 58)),
            reserved: u32_at(fixed, 60),
        }
    }

    /// Append the 64-byte header to `out`.
    ///
    /// `sizes` holds the size of each substream in bytes, in the order of
    /// [`Substream::ALL`](super::Substream::ALL).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if a size is larger than `i32::MAX`,
    ///   if a stream index is larger than `0xfffe` or if the output cannot grow.
    ///
    /// The function does not change `out` if it returns an error.
    pub fn encode(&self, sizes: &[usize; SUBSTREAM_COUNT], out: &mut Vec<u8>) -> Result<()> {
        let mut fixed = [0_u8; FIXED_SIZE];
        let mut put = |offset: usize, value: &[u8]| {
            fixed[offset..offset + value.len()].copy_from_slice(value);
        };
        put(0, &MODERN_SIGNATURE.to_le_bytes());
        put(4, &VERSION_V70.to_le_bytes());
        put(8, &self.age.to_le_bytes());
        put(12, &stream_index(self.global_stream)?.to_le_bytes());
        put(14, &self.build_number.to_le_bytes());
        put(16, &stream_index(self.public_stream)?.to_le_bytes());
        put(18, &self.pdb_dll_version.to_le_bytes());
        put(20, &stream_index(self.symbol_stream)?.to_le_bytes());
        put(22, &self.pdb_dll_rebuild.to_le_bytes());
        for (&size, &(offset, _)) in sizes.iter().zip(SIZE_FIELDS.iter()) {
            let size = i32::try_from(size).map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("DBI substream size does not fit in i32")
            })?;
            put(offset, &size.to_le_bytes());
        }
        put(44, &self.mfc_type_server_index.to_le_bytes());
        put(56, &self.flags.to_le_bytes());
        put(58, &self.machine.0.to_le_bytes());
        put(60, &self.reserved.to_le_bytes());
        Output::new(out).bytes(&fixed)
    }
}
