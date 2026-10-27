//! One stream container type for MSF 7.00 and MSFZ.
//!
//! The native PDB streams are the same in both containers. [`Container`] selects the codec
//! from the signature and gives one [`Stream`] reader for both. PDB stream decoders use only
//! the stream bytes; they do not see MSF pages or MSFZ chunks.
//!
//! Use [`Container::msf`] or [`Container::msfz`] for checks that apply to one container only,
//! for example [`Msf::validate_free_page_map`].
//!
//! [`Container::to_msfz`] and [`Container::to_msf`] convert either container to MSFZ or to
//! MSF 7.00 (#38).

use alloc::vec::Vec;

mod convert;
mod view;

pub use view::StreamBytes;

use super::msf::{self, Msf, MsfLimits, StreamId};
use super::msfz::{self, Msfz, MsfzLimits};
use crate::io::ReadAt;
use crate::{ContainerKind, probe};
use crate::{Error, ErrorKind, Result};

/// A native stream container: MSF 7.00 or MSFZ.
#[derive(Debug)]
#[non_exhaustive]
pub enum Container<S> {
    /// An MSF 7.00 container.
    Msf(Msf<S>),
    /// An MSFZ container.
    Msfz(Msfz<S>),
}

/// Resource limits for [`Container::with_limits`].
///
/// The value holds the limits of each container format. The default holds
/// [`MsfLimits::default`] and [`MsfzLimits::default`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ContainerLimits {
    msf: MsfLimits,
    msfz: MsfzLimits,
}

impl ContainerLimits {
    /// Return the limits for an MSF 7.00 container.
    #[must_use]
    pub const fn msf(self) -> MsfLimits {
        self.msf
    }

    /// Set the limits for an MSF 7.00 container.
    #[must_use]
    pub const fn with_msf(mut self, limits: MsfLimits) -> Self {
        self.msf = limits;
        self
    }

    /// Return the limits for an MSFZ container.
    #[must_use]
    pub const fn msfz(self) -> MsfzLimits {
        self.msfz
    }

    /// Set the limits for an MSFZ container.
    #[must_use]
    pub const fn with_msfz(mut self, limits: MsfzLimits) -> Self {
        self.msfz = limits;
        self
    }
}

impl<S: ReadAt> Container<S> {
    /// Read a container with the default limits.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Container::with_limits`].
    pub fn read(source: S) -> Result<Self> {
        Self::with_limits(source, ContainerLimits::default())
    }

    /// Read an MSF 7.00 container with [`ContainerLimits::msf`] or an MSFZ container with
    /// [`ContainerLimits::msfz`].
    ///
    /// # Errors
    ///
    /// - The errors of [`probe`].
    /// - [`ErrorKind::UnsupportedVersion`] for a small MSF 2.00 container.
    /// - [`ErrorKind::InvalidSignature`] for other signatures.
    /// - The errors of [`Msf::with_limits`] or [`Msfz::with_limits`].
    pub fn with_limits(source: S, limits: ContainerLimits) -> Result<Self> {
        match probe(&source)? {
            ContainerKind::NativeMsfz => Msfz::with_limits(source, limits.msfz).map(Self::Msfz),
            ContainerKind::NativeMsf7 => Msf::with_limits(source, limits.msf).map(Self::Msf),
            ContainerKind::LegacyMsf2 => Err(Error::new(ErrorKind::UnsupportedVersion, Some(0))
                .with_context("small MSF 2.00 container")),
            _ => Err(Error::new(ErrorKind::InvalidSignature, Some(0))
                .with_context("not an MSF 7.00 or MSFZ container")),
        }
    }

    /// Return a reader for the stream `id`.
    ///
    /// Returns `Ok(None)` if the directory marks the stream as nil.
    /// An empty stream is `Some` with a length of zero.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidStreamIndex`] if `id` is not less than
    /// [`Container::stream_count`], and the other errors of [`Msf::stream`] and
    /// [`Msfz::stream`].
    pub fn stream(&self, id: StreamId) -> Result<Option<Stream<'_, S>>> {
        Ok(match self {
            Self::Msf(msf) => msf.stream(id)?.map(Stream::Msf),
            Self::Msfz(msfz) => msfz.stream(id)?.map(Stream::Msfz),
        })
    }
}

impl<S> Container<S> {
    /// Return the container kind: [`ContainerKind::NativeMsf7`] or
    /// [`ContainerKind::NativeMsfz`].
    #[must_use]
    pub const fn kind(&self) -> ContainerKind {
        match self {
            Self::Msf(_) => ContainerKind::NativeMsf7,
            Self::Msfz(_) => ContainerKind::NativeMsfz,
        }
    }

    /// Return the number of streams in the stream directory, including nil streams.
    #[must_use]
    pub fn stream_count(&self) -> u32 {
        match self {
            Self::Msf(msf) => msf.stream_count(),
            Self::Msfz(msfz) => msfz.stream_count(),
        }
    }

    /// Return the MSF 7.00 container, or `None` for MSFZ.
    #[must_use]
    pub const fn msf(&self) -> Option<&Msf<S>> {
        match self {
            Self::Msf(msf) => Some(msf),
            Self::Msfz(_) => None,
        }
    }

    /// Return the MSFZ container, or `None` for MSF 7.00.
    #[must_use]
    pub const fn msfz(&self) -> Option<&Msfz<S>> {
        match self {
            Self::Msf(_) => None,
            Self::Msfz(msfz) => Some(msfz),
        }
    }
}

impl<S: ReadAt> Container<S> {
    /// Return the `length` bytes at the source offset `physical` as a slice
    /// of the source, if the source lends them. An MSFZ container does not
    /// lend (#48).
    pub(crate) fn lend_at(&self, physical: u64, length: usize) -> Option<&[u8]> {
        match self {
            Self::Msf(msf) => msf.lend_at(physical, length),
            Self::Msfz(_) => None,
        }
    }
}

/// A reader for the bytes of one stream of either container.
///
/// Offsets for [`ReadAt`] are logical offsets in the stream. See [`msf::Stream`] and
/// [`msfz::Stream`] for the read behavior and the error locations.
#[derive(Debug)]
#[non_exhaustive]
pub enum Stream<'a, S> {
    /// A stream of an MSF 7.00 container.
    Msf(msf::Stream<'a, S>),
    /// A stream of an MSFZ container.
    Msfz(msfz::Stream<'a, S>),
}

impl<S> Stream<'_, S> {
    /// Return the ID of the stream.
    #[must_use]
    pub const fn id(&self) -> StreamId {
        match self {
            Self::Msf(stream) => stream.id(),
            Self::Msfz(stream) => stream.id(),
        }
    }
}

impl<S: ReadAt> Stream<'_, S> {
    /// Read the full stream into a new vector.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`msf::Stream::read_all`] or [`msfz::Stream::read_all`].
    pub fn read_all(&self, max_bytes: usize) -> Result<Vec<u8>> {
        match self {
            Self::Msf(stream) => stream.read_all(max_bytes),
            Self::Msfz(stream) => stream.read_all(max_bytes),
        }
    }
}

impl<S: ReadAt> ReadAt for Stream<'_, S> {
    fn len(&self) -> Result<u64> {
        match self {
            Self::Msf(stream) => stream.len(),
            Self::Msfz(stream) => stream.len(),
        }
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        match self {
            Self::Msf(stream) => stream.read_exact_at(offset, buffer),
            Self::Msfz(stream) => stream.read_exact_at(offset, buffer),
        }
    }

    fn borrow_at(&self, offset: u64, length: usize) -> Option<&[u8]> {
        self.lend(offset, length)
    }
}
