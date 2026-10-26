//! The options of the write plan of a [`PdbBuilder`](super::PdbBuilder).

use crate::native::Producer;
use crate::native::msf::MsfWriteOptions;
use crate::native::types::DEFAULT_HASH_BUCKETS;

/// The options of the write plan of a [`PdbBuilder`](super::PdbBuilder).
///
/// [`Default`] gives the conventions of Microsoft `link.exe`. Set a value
/// with its `with_*` function and read it with the function of the same
/// name. A later version can add options, for example an output profile,
/// without a change for callers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BuildOptions {
    /// The container options: page size and limits.
    msf: MsfWriteOptions,
    /// The producer of the layout of the TPI and IPI hash streams.
    type_hash_producer: Producer,
    /// The hash bucket count of the TPI and IPI streams.
    hash_buckets: u32,
    /// The producer of the conventions of the symbol indexes.
    index_producer: Producer,
    /// The producer of the conventions of the named-stream map.
    info_producer: Producer,
    /// `true` to write the TPI hash stream.
    type_hash_stream: bool,
    /// `true` to write the IPI hash stream.
    id_hash_stream: bool,
}

impl Default for BuildOptions {
    /// Return the conventions of Microsoft `link.exe`: the default
    /// [`MsfWriteOptions`], [`Producer::Microsoft`] for all producer
    /// conventions, [`DEFAULT_HASH_BUCKETS`] hash buckets and both hash
    /// streams.
    fn default() -> Self {
        Self {
            msf: MsfWriteOptions::default(),
            type_hash_producer: Producer::Microsoft,
            hash_buckets: DEFAULT_HASH_BUCKETS,
            index_producer: Producer::Microsoft,
            info_producer: Producer::Microsoft,
            type_hash_stream: true,
            id_hash_stream: true,
        }
    }
}

impl BuildOptions {
    /// Return the container options: the page size and the limits.
    #[must_use]
    pub const fn msf(self) -> MsfWriteOptions {
        self.msf
    }

    /// Set the container options: the page size and the limits.
    #[must_use]
    pub const fn with_msf(mut self, msf: MsfWriteOptions) -> Self {
        self.msf = msf;
        self
    }

    /// Return the producer whose layout of the TPI and IPI hash streams the
    /// writer follows.
    #[must_use]
    pub const fn type_hash_producer(self) -> Producer {
        self.type_hash_producer
    }

    /// Set the producer whose layout of the TPI and IPI hash streams the
    /// writer follows (see
    /// [`TypeStreamOptions::with_producer`](crate::native::types::TypeStreamOptions::with_producer)).
    #[must_use]
    pub const fn with_type_hash_producer(mut self, producer: Producer) -> Self {
        self.type_hash_producer = producer;
        self
    }

    /// Return the hash bucket count of the TPI and IPI streams.
    #[must_use]
    pub const fn hash_buckets(self) -> u32 {
        self.hash_buckets
    }

    /// Set the hash bucket count of the TPI and IPI streams.
    #[must_use]
    pub const fn with_hash_buckets(mut self, buckets: u32) -> Self {
        self.hash_buckets = buckets;
        self
    }

    /// Return the producer whose conventions of the global and public symbol
    /// indexes the writer follows.
    #[must_use]
    pub const fn index_producer(self) -> Producer {
        self.index_producer
    }

    /// Set the producer whose conventions of the global and public symbol
    /// indexes the writer follows (see
    /// [`PublicSymbolIndex::encode_parts`](crate::native::symbols::PublicSymbolIndex::encode_parts)).
    #[must_use]
    pub const fn with_index_producer(mut self, producer: Producer) -> Self {
        self.index_producer = producer;
        self
    }

    /// Return the producer whose conventions of the named-stream map of the
    /// Info stream the writer follows.
    #[must_use]
    pub const fn info_producer(self) -> Producer {
        self.info_producer
    }

    /// Set the producer whose conventions of the named-stream map of the
    /// Info stream the writer follows (see
    /// [`OwnedHashTable`](crate::native::hash_table::OwnedHashTable)).
    #[must_use]
    pub const fn with_info_producer(mut self, producer: Producer) -> Self {
        self.info_producer = producer;
        self
    }

    /// Return `true` if the plan writes the TPI hash stream.
    #[must_use]
    pub const fn type_hash_stream(self) -> bool {
        self.type_hash_stream
    }

    /// Select whether the plan writes the TPI hash stream. Microsoft and
    /// LLVM linkers write it.
    #[must_use]
    pub const fn with_type_hash_stream(mut self, present: bool) -> Self {
        self.type_hash_stream = present;
        self
    }

    /// Return `true` if the plan writes the IPI hash stream.
    #[must_use]
    pub const fn id_hash_stream(self) -> bool {
        self.id_hash_stream
    }

    /// Select whether the plan writes the IPI hash stream.
    #[must_use]
    pub const fn with_id_hash_stream(mut self, present: bool) -> Self {
        self.id_hash_stream = present;
        self
    }
}
