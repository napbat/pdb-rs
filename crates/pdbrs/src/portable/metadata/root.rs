//! The metadata root and its stream headers (ECMA-335 II.24.2.1 and II.24.2.2).

use alloc::vec::Vec;

use crate::PortableLocation;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The signature at the start of a metadata root.
pub const METADATA_SIGNATURE: [u8; 4] = *b"BSJB";

/// The largest version field: 255 string bytes and a terminator.
const MAX_VERSION_LEN: u32 = 256;

/// The largest stream name, with its terminator.
const MAX_NAME_LEN: usize = 32;

/// A metadata stream that this crate recognizes by its name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum MetadataStream {
    /// The metadata root header and the stream headers, not a stream.
    Root,
    /// The `#Pdb` stream: PDB ID, entry point and external row counts.
    Pdb,
    /// The `#~` stream: the compressed tables.
    Tables,
    /// The `#-` stream: the uncompressed tables of edit-and-continue metadata.
    UncompressedTables,
    /// The `#Strings` heap.
    Strings,
    /// The `#US` (user string) heap.
    UserStrings,
    /// The `#Blob` heap.
    Blob,
    /// The `#GUID` heap.
    Guid,
}

impl MetadataStream {
    /// Every recognized stream.
    const ALL: [Self; 7] = [
        Self::Pdb,
        Self::Tables,
        Self::UncompressedTables,
        Self::Strings,
        Self::UserStrings,
        Self::Blob,
        Self::Guid,
    ];

    /// Return the stream name, or `metadata root` for [`MetadataStream::Root`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Root => "metadata root",
            Self::Pdb => "#Pdb",
            Self::Tables => "#~",
            Self::UncompressedTables => "#-",
            Self::Strings => "#Strings",
            Self::UserStrings => "#US",
            Self::Blob => "#Blob",
            Self::Guid => "#GUID",
        }
    }

    /// Find the stream with the exact name bytes `name`.
    #[must_use]
    pub fn from_name(name: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|stream| stream.name().as_bytes() == name)
    }

    /// Return a location in this stream.
    pub(crate) const fn location(self) -> PortableLocation {
        PortableLocation::new().with_stream(self)
    }
}

/// One stream header of the metadata root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamHeader {
    name: [u8; MAX_NAME_LEN],
    name_len: usize,
    offset: u32,
    size: u32,
}

impl StreamHeader {
    /// Return the name bytes without the terminator.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    /// Return the recognized stream, if the name is known.
    #[must_use]
    pub fn stream(&self) -> Option<MetadataStream> {
        MetadataStream::from_name(self.name())
    }

    /// Return the stream offset from the start of the metadata root.
    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    /// Return the stream size in bytes.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// Return the offset after the end of the stream. The root checks that it does not
    /// overflow.
    fn end(&self) -> u64 {
        u64::from(self.offset) + u64::from(self.size)
    }
}

/// A validated metadata root with its stream headers.
///
/// The root checks the signature, the version, the version string, each stream header, the
/// 4-byte alignment and range of each stream, overlaps and duplicate names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataRoot {
    major: u16,
    minor: u16,
    version: Vec<u8>,
    flags: u16,
    streams: Vec<StreamHeader>,
    header_len: u32,
}

impl MetadataRoot {
    /// Read and validate the metadata root at offset 0 of `source`.
    ///
    /// All stream ranges must be inside `source`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidSignature`] if the input does not start with `BSJB`.
    /// - [`ErrorKind::UnsupportedVersion`] for a root version other than 1.1.
    /// - [`ErrorKind::LimitExceeded`] if the root has more than `max_streams` streams.
    /// - [`ErrorKind::InvalidFormat`] for a malformed version string or stream name, an
    ///   unaligned stream, overlapping streams or a duplicate stream name.
    /// - [`ErrorKind::UnexpectedEof`] if the root or a stream ends after the input.
    pub fn read(source: &(impl ReadAt + ?Sized), max_streams: u16) -> Result<Self> {
        let root = PortableLocation::new().with_stream(MetadataStream::Root);
        let length = source.len()?;
        let mut header = [0; 16];
        source.read_exact_at(0, &mut header)?;
        if header[..4] != METADATA_SIGNATURE {
            return Err(Error::new(ErrorKind::InvalidSignature, Some(0)));
        }
        let major = u16::from_le_bytes([header[4], header[5]]);
        let minor = u16::from_le_bytes([header[6], header[7]]);
        if (major, minor) != (1, 1) {
            return Err(Error::new(ErrorKind::UnsupportedVersion, Some(4))
                .with_context("metadata root version is not 1.1")
                .with_portable(root));
        }
        let version_len = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        if version_len == 0 || version_len > MAX_VERSION_LEN || version_len % 4 != 0 {
            return Err(invalid(
                12,
                "metadata version length is not 4 to 256 in steps of 4",
            ));
        }
        let mut version = [0; MAX_VERSION_LEN as usize];
        let version = &mut version[..version_len as usize];
        source.read_exact_at(16, version)?;
        let Some(terminator) = version.iter().position(|&byte| byte == 0) else {
            return Err(invalid(16, "metadata version string has no terminator"));
        };
        let version = version[..terminator].to_vec();

        let mut position = 16 + u64::from(version_len);
        let flags = source.read_u16_at(position)?;
        let count = source.read_u16_at(position + 2)?;
        position += 4;
        if count > max_streams {
            return Err(Error::new(ErrorKind::LimitExceeded, Some(position - 2))
                .with_context("metadata stream count")
                .with_portable(root));
        }
        let mut streams = Vec::new();
        streams.try_reserve_exact(usize::from(count)).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None).with_context("stream headers")
        })?;
        for _ in 0..count {
            let (stream, next) = read_header(source, position)?;
            streams.push(stream);
            position = next;
        }
        let header_len = u32::try_from(position)
            .map_err(|_| Error::new(ErrorKind::OffsetOverflow, Some(position)))?;
        let root = Self {
            major,
            minor,
            version,
            flags,
            streams,
            header_len,
        };
        root.check_streams(length)?;
        Ok(root)
    }

    /// Check alignment, ranges, overlaps and duplicate names.
    ///
    /// An error has the location of the recognized stream, or of the root for another stream.
    fn check_streams(&self, length: u64) -> Result<()> {
        for (index, stream) in self.streams.iter().enumerate() {
            let location = stream.stream().unwrap_or(MetadataStream::Root).location();
            let offset = u64::from(stream.offset);
            let invalid = |context| {
                Error::new(ErrorKind::InvalidFormat, Some(offset))
                    .with_context(context)
                    .with_portable(location)
            };
            if stream.offset % 4 != 0 || stream.size % 4 != 0 {
                return Err(invalid(
                    "metadata stream offset or size is not 4-byte aligned",
                ));
            }
            if stream.end() > length {
                return Err(
                    Error::new(ErrorKind::UnexpectedEof, Some(length.min(stream.end())))
                        .with_context("metadata stream ends after the input")
                        .with_portable(location),
                );
            }
            if stream.size > 0 && stream.offset < self.header_len {
                return Err(invalid("metadata stream overlaps the stream headers"));
            }
            for other in &self.streams[..index] {
                if other.name() == stream.name() {
                    return Err(invalid("duplicate metadata stream"));
                }
                let overlaps = stream.size > 0
                    && other.size > 0
                    && u64::from(stream.offset) < other.end()
                    && u64::from(other.offset) < stream.end();
                if overlaps {
                    return Err(invalid("metadata streams overlap"));
                }
            }
        }
        Ok(())
    }

    /// Return the root version as (major, minor). The root accepts only 1.1.
    #[must_use]
    pub const fn version(&self) -> (u16, u16) {
        (self.major, self.minor)
    }

    /// Return the version string bytes without the terminator and padding, for example
    /// `PDB v1.0`.
    #[must_use]
    pub fn version_string(&self) -> &[u8] {
        &self.version
    }

    /// Return the reserved flags field.
    #[must_use]
    pub const fn flags(&self) -> u16 {
        self.flags
    }

    /// Return the stream headers in their serialized order.
    #[must_use]
    pub fn streams(&self) -> &[StreamHeader] {
        &self.streams
    }

    /// Return the header of a recognized stream, if present.
    #[must_use]
    pub fn stream(&self, stream: MetadataStream) -> Option<&StreamHeader> {
        self.streams
            .iter()
            .find(|header| header.stream() == Some(stream))
    }

    /// Return the size of the root header with its stream headers.
    #[must_use]
    pub const fn header_len(&self) -> u32 {
        self.header_len
    }
}

/// Read one stream header at `position`. Return it and the position of the next header.
fn read_header(source: &(impl ReadAt + ?Sized), position: u64) -> Result<(StreamHeader, u64)> {
    let offset = source.read_u32_at(position)?;
    let size = source.read_u32_at(position + 4)?;
    let mut name = [0; MAX_NAME_LEN];
    let start = position + 8;
    let mut name_len = None;
    for chunk in 0..MAX_NAME_LEN / 4 {
        let bytes = &mut name[chunk * 4..chunk * 4 + 4];
        source.read_exact_at(start + chunk as u64 * 4, bytes)?;
        if let Some(end) = bytes.iter().position(|&byte| byte == 0) {
            name_len = Some(chunk * 4 + end);
            break;
        }
    }
    let Some(name_len) = name_len else {
        return Err(invalid(
            start,
            "metadata stream name is longer than 31 bytes",
        ));
    };
    name[name_len..].fill(0);
    let padded = (name_len + 4) & !3;
    let header = StreamHeader {
        name,
        name_len,
        offset,
        size,
    };
    Ok((header, start + padded as u64))
}

/// Make an [`ErrorKind::InvalidFormat`] error in the metadata root.
fn invalid(offset: u64, context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, Some(offset))
        .with_context(context)
        .with_portable(MetadataStream::Root.location())
}
