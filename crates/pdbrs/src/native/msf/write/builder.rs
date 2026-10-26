//! Construction of a new container from arbitrary streams.

use alloc::vec::Vec;

use super::super::{NIL_STREAM_SIZE, StreamId};
use super::layout::check_page_size;
use super::{StreamSource, WriteOptions, WritePlan, plan_allocation_failed};
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// A list of streams for a new container.
///
/// Each stream is nil or has the bytes of a [`ReadAt`] source. Stream IDs count from 0 in the
/// order of the calls to [`MsfBuilder::push`] and [`MsfBuilder::push_nil`]. Stream 0 is the
/// previous stream directory of the MSF format. The free page map marks its pages as free.
///
/// The builder makes no claim that the streams form a valid PDB.
#[derive(Clone, Debug)]
pub struct MsfBuilder<R> {
    streams: Vec<Option<R>>,
}

impl<R> MsfBuilder<R> {
    /// Create a builder with no streams.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            streams: Vec::new(),
        }
    }

    /// Add a stream with the bytes of `data` and return its ID.
    ///
    /// The plan reads the length of `data` when it starts and the bytes when it writes.
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
    /// Returns the errors of [`MsfBuilder::push`].
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
                    .with_context("stream count does not fit in the stream directory")
            })?;
        self.streams
            .try_reserve(1)
            .map_err(|_| plan_allocation_failed())?;
        self.streams.push(stream);
        Ok(StreamId(id))
    }
}

impl<R: ReadAt> MsfBuilder<R> {
    /// Plan a container with the streams of the builder.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the page size in `options` is 65536 (see
    ///   [`WriteOptions::with_page_size`]).
    /// - [`ErrorKind::InvalidFormat`] if the builder has no streams or the page size in
    ///   `options` is not valid.
    /// - [`ErrorKind::LimitExceeded`] if a stream has `0xFFFF_FFFF` bytes or more, the layout
    ///   exceeds a limit (see [`WritePlan`]) or an allocation fails.
    /// - An error from a source if its length is not available.
    pub fn plan(&self, options: WriteOptions) -> Result<WritePlan<'_>> {
        check_page_size(options.page_size())?;
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(self.streams.len())
            .map_err(|_| plan_allocation_failed())?;
        for (id, stream) in (0_u32..).zip(&self.streams) {
            let size = match stream {
                None => NIL_STREAM_SIZE,
                Some(data) => u32::try_from(data.len()?)
                    .ok()
                    .filter(|&size| size != NIL_STREAM_SIZE)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::LimitExceeded, None)
                            .with_context("stream is too large for an MSF stream")
                            .with_stream(id, 0)
                    })?,
            };
            sizes.push(size);
        }
        WritePlan::new(options, sizes, self)
    }
}

impl<R> Default for MsfBuilder<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ReadAt> StreamSource for MsfBuilder<R> {
    fn read_stream(&self, id: u32, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let data = usize::try_from(id)
            .ok()
            .and_then(|index| self.streams.get(index))
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                Error::new(ErrorKind::MissingStream, None)
                    .with_context("planned stream is nil in the builder")
                    .with_stream(id, offset)
            })?;
        data.read_exact_at(offset, buffer)
            .map_err(|error| error.with_stream(id, offset))
    }
}
