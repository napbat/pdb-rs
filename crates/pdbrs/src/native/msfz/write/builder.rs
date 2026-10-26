//! Construction of a new MSFZ container from arbitrary streams.

use alloc::vec::Vec;

use super::{WriteOptions, WritePlan, plan_allocation_failed};
use crate::io::ReadAt;
use crate::native::msf::{StreamId, StreamSource};
use crate::{Error, ErrorKind, Result};

/// A list of streams for a new MSFZ container.
///
/// Each stream is nil or has the bytes of a [`ReadAt`] source. Stream IDs count from 0 in the
/// order of the calls to [`MsfzBuilder::push`] and [`MsfzBuilder::push_nil`]. A stream can be
/// larger than 4 GiB.
///
/// The builder makes no claim that the streams form a valid PDB.
#[derive(Clone, Debug)]
pub struct MsfzBuilder<R> {
    streams: Vec<Option<R>>,
}

impl<R> MsfzBuilder<R> {
    /// Create a builder with no streams.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            streams: Vec::new(),
        }
    }

    /// Add a stream with the bytes of `data` and return its ID.
    ///
    /// The plan reads the length of `data` when it starts and the bytes when it compresses the
    /// chunks.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the stream count does not fit in `u32` or the
    /// allocation fails.
    pub fn push(&mut self, data: R) -> Result<StreamId> {
        self.add(Some(data))
    }

    /// Add a nil stream and return its ID.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`MsfzBuilder::push`].
    pub fn push_nil(&mut self) -> Result<StreamId> {
        self.add(None)
    }

    /// Return the number of streams, including nil streams.
    #[must_use]
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    /// Add one stream entry.
    fn add(&mut self, stream: Option<R>) -> Result<StreamId> {
        let id = u32::try_from(self.streams.len())
            .ok()
            .filter(|&id| id < u32::MAX)
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("stream count does not fit in the MSFZ header")
            })?;
        self.streams
            .try_reserve(1)
            .map_err(|_| plan_allocation_failed())?;
        self.streams.push(stream);
        Ok(StreamId(id))
    }
}

impl<R: ReadAt> MsfzBuilder<R> {
    /// Plan a container with the streams of the builder.
    ///
    /// The plan reads and compresses all stream bytes (see [`WritePlan`]).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the builder has no streams or the maximum chunk size
    ///   in `options` is not valid.
    /// - [`ErrorKind::LimitExceeded`] if the layout exceeds a limit (see [`WritePlan`]) or an
    ///   allocation fails.
    /// - An error from a source if its length or its bytes are not available.
    pub fn plan(&self, options: WriteOptions) -> Result<WritePlan<'_>> {
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(self.streams.len())
            .map_err(|_| plan_allocation_failed())?;
        for (id, stream) in (0_u32..).zip(&self.streams) {
            sizes.push(
                stream
                    .as_ref()
                    .map(|data| {
                        data.len()
                            .map_err(|error| error.with_stream(StreamId(id), 0))
                    })
                    .transpose()?,
            );
        }
        WritePlan::new(options, &sizes, self)
    }
}

impl<R> Default for MsfzBuilder<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ReadAt> StreamSource for MsfzBuilder<R> {
    fn read_stream(&self, id: u32, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let data = usize::try_from(id)
            .ok()
            .and_then(|index| self.streams.get(index))
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                Error::new(ErrorKind::MissingStream, None)
                    .with_context("planned stream is nil in the builder")
                    .with_stream(StreamId(id), offset)
            })?;
        data.read_exact_at(offset, buffer)
            .map_err(|error| error.with_stream(StreamId(id), offset))
    }
}
