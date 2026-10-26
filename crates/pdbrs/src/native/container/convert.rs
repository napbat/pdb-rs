//! Conversion between MSF 7.00 and MSFZ.
//!
//! Both conversions keep the stream IDs, the nil streams, the empty streams and the stream
//! bytes. [`OldDirectoryPolicy`] selects stream 0, the previous MSF stream directory. The
//! conversions do not interpret the stream contents.

use alloc::vec::Vec;

use super::Container;
use crate::io::ReadAt;
use crate::native::msf::{self, OldDirectoryPolicy, StreamId, StreamSource};
use crate::native::msfz;
use crate::{Error, ErrorKind, Result};

impl<S: ReadAt> Container<S> {
    /// Plan an MSFZ container with the streams of this container.
    ///
    /// The plan keeps the stream IDs, the nil streams, the empty streams and the stream bytes.
    /// `old_directory` selects the contents of stream 0:
    ///
    /// - [`OldDirectoryPolicy::Empty`] writes stream 0 as a present stream of length zero, as
    ///   `pdbtool pdz-encode` does.
    /// - [`OldDirectoryPolicy::Preserve`] copies the bytes of stream 0, or its nil state. MSFZ
    ///   has no pages, so the bytes describe the pages of the source.
    ///
    /// The plan reads and compresses all stream bytes (see [`msfz::MsfzWritePlan`]). It reads the
    /// stream bytes from this container again when it writes.
    ///
    /// # Errors
    ///
    /// - The errors of [`Container::stream`] for each stream.
    /// - The planning errors of [`msfz::MsfzWritePlan`], and the errors of the stream reads.
    pub fn to_msfz(
        &self,
        options: msfz::MsfzWriteOptions,
        old_directory: OldDirectoryPolicy,
    ) -> Result<msfz::MsfzWritePlan<'_>> {
        let sizes = self.stream_sizes(old_directory)?;
        msfz::MsfzWritePlan::new(options, &sizes, self)
    }

    /// Plan an MSF 7.00 container with the streams of this container.
    ///
    /// For an MSF 7.00 container, this is [`Msf::repack`](msf::Msf::repack). For an MSFZ
    /// container, the plan keeps the stream IDs, the nil streams, the empty streams and the
    /// stream bytes. [`OldDirectoryPolicy::Preserve`] copies stream 0 without a check: MSFZ
    /// has no page size. Give the page size of the MSF 7.00 file that stream 0 describes.
    ///
    /// # Errors
    ///
    /// - The errors of [`Msf::repack`](msf::Msf::repack) for an MSF 7.00 container.
    /// - The errors of [`Container::stream`] for each stream.
    /// - [`ErrorKind::LimitExceeded`] if a stream has `0xFFFF_FFFF` bytes or more.
    /// - The planning errors of [`msf::MsfWritePlan`].
    pub fn to_msf(
        &self,
        options: msf::MsfWriteOptions,
        old_directory: OldDirectoryPolicy,
    ) -> Result<msf::MsfWritePlan<'_>> {
        if let Self::Msf(msf) = self {
            return msf.repack(options, old_directory);
        }
        let sizes = self.stream_sizes(old_directory)?;
        msf::plan_sizes(options, &sizes, self)
    }

    /// Return the size of each stream, or `None` for a nil stream, with stream 0 as
    /// `old_directory` selects it.
    fn stream_sizes(&self, old_directory: OldDirectoryPolicy) -> Result<Vec<Option<u64>>> {
        let count = self.stream_count();
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(usize::try_from(count).unwrap_or(usize::MAX))
            .map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("stream size list allocation failed")
            })?;
        for id in (0..count).map(StreamId) {
            sizes.push(self.stream(id)?.map(|stream| stream.len()).transpose()?);
        }
        if old_directory == OldDirectoryPolicy::Empty
            && let Some(first) = sizes.first_mut()
        {
            *first = Some(0);
        }
        Ok(sizes)
    }
}

impl<S: ReadAt> StreamSource for Container<S> {
    fn read_stream(&self, id: u32, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let Some(stream) = self.stream(StreamId(id))? else {
            return Err(Error::new(ErrorKind::MissingStream, None)
                .with_context("planned stream is nil in the source")
                .with_stream(StreamId(id), offset));
        };
        stream.read_exact_at(offset, buffer)
    }
}
