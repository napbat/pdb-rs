//! A bounded cache of decompressed chunks.
//!
//! The cache keeps decompressed chunks up to a byte capacity. When a new chunk does not fit,
//! it removes the least recently used chunks first. A counter that increments on each access
//! gives the order, so the same sequence of reads gives the same evictions. A chunk that is
//! larger than the capacity is decoded for the read and not kept. A chunk that does not decode
//! is decoded only once: the cache keeps its error.
//!
//! Under `std`, a `Mutex` protects the cache, so the cache is `Sync`. Without `std`, a
//! `RefCell` protects it, as in [`crate::io::SeekReader`].

use alloc::vec::Vec;

use super::no_memory;
use crate::{Error, ErrorKind, Result};

/// The interior mutability of the cache.
#[cfg(feature = "std")]
#[derive(Debug, Default)]
struct Lock<T>(std::sync::Mutex<T>);

#[cfg(feature = "std")]
impl<T> Lock<T> {
    /// Call `f` with exclusive access to the value.
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        // A panic in `f` cannot leave the state inconsistent: each change is a single push,
        // removal or assignment.
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }
}

/// The interior mutability of the cache.
#[cfg(not(feature = "std"))]
#[derive(Debug, Default)]
struct Lock<T>(core::cell::RefCell<T>);

#[cfg(not(feature = "std"))]
impl<T> Lock<T> {
    /// Call `f` with exclusive access to the value.
    ///
    /// The cache does not call itself from `f`, so the borrow does not fail.
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

/// One cached chunk.
#[derive(Debug)]
struct Slot {
    /// The chunk index.
    chunk: u32,
    /// The counter value of the last access.
    last_use: u64,
    /// The decompressed bytes.
    data: Vec<u8>,
}

/// The cached chunks and the access counter.
#[derive(Debug, Default)]
struct State {
    slots: Vec<Slot>,
    /// The sum of the sizes of the cached chunks.
    used: usize,
    /// The access counter.
    clock: u64,
    /// The chunks that do not decode, with the error kind and context.
    ///
    /// The source bytes do not change ([`crate::io::ReadAt`]), so a chunk that does not decode
    /// once does not decode again. The list has at most one entry for each chunk.
    failures: Vec<(u32, ErrorKind, Option<&'static str>)>,
}

/// A bounded cache of decompressed chunks.
#[derive(Debug)]
pub(super) struct ChunkCache {
    state: Lock<State>,
    capacity: usize,
}

impl ChunkCache {
    /// Make an empty cache that keeps at most `capacity` bytes.
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            state: Lock::default(),
            capacity,
        }
    }

    /// Call `visit` with the decompressed bytes of chunk `chunk`.
    ///
    /// If the cache does not hold the chunk, `load` decodes it. Then the cache keeps it if it
    /// is not larger than the capacity. If `load` fails with [`ErrorKind::InvalidFormat`],
    /// [`ErrorKind::UnexpectedEof`] or [`ErrorKind::UnsupportedVersion`], the cache keeps the
    /// error kind and context, and later calls return them without a new decode. Other errors,
    /// for example I/O errors and failed allocations, are not kept.
    pub(super) fn with_chunk<R>(
        &self,
        chunk: u32,
        load: impl FnOnce() -> Result<Vec<u8>>,
        visit: impl FnOnce(&[u8]) -> R,
    ) -> Result<R> {
        let capacity = self.capacity;
        self.state.with(|state| {
            state.clock = state.clock.saturating_add(1);
            let clock = state.clock;
            if let Some(slot) = state.slots.iter_mut().find(|slot| slot.chunk == chunk) {
                slot.last_use = clock;
                return Ok(visit(&slot.data));
            }
            let failed = state
                .failures
                .binary_search_by_key(&chunk, |&(failed, ..)| failed);
            if let Ok(index) = failed {
                let (_, kind, context) = state.failures[index];
                let error = Error::new(kind, None);
                return Err(match context {
                    Some(context) => error.with_context(context),
                    None => error,
                });
            }
            let data = match load() {
                Ok(data) => data,
                Err(error) => {
                    if matches!(
                        error.kind(),
                        ErrorKind::InvalidFormat
                            | ErrorKind::UnexpectedEof
                            | ErrorKind::UnsupportedVersion
                    ) && let Err(position) = failed
                        && state.failures.try_reserve(1).is_ok()
                    {
                        let failure = (chunk, error.kind(), error.context());
                        state.failures.insert(position, failure);
                    }
                    return Err(error);
                }
            };
            if data.len() > capacity {
                return Ok(visit(&data));
            }
            while state.used + data.len() > capacity {
                let Some(oldest) = state
                    .slots
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, slot)| slot.last_use)
                    .map(|(index, _)| index)
                else {
                    break;
                };
                let removed = state.slots.swap_remove(oldest);
                state.used -= removed.data.len();
            }
            state.slots.try_reserve(1).map_err(|_| no_memory())?;
            state.used += data.len();
            state.slots.push(Slot {
                chunk,
                last_use: clock,
                data,
            });
            let slot = state.slots.last().map_or(&[][..], |slot| &slot.data);
            Ok(visit(slot))
        })
    }

    /// Return the indexes of the cached chunks in ascending order.
    #[cfg(test)]
    pub(super) fn cached(&self) -> Vec<u32> {
        self.state.with(|state| {
            let mut chunks: Vec<u32> = state.slots.iter().map(|slot| slot.chunk).collect();
            chunks.sort_unstable();
            chunks
        })
    }
}
