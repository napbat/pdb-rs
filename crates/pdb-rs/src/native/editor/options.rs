//! The options of the write plan of a [`PdbEditor`](super::PdbEditor).

use crate::native::Producer;
use crate::native::msf::MsfWriteOptions;

/// The options of the write plan of a [`PdbEditor`](super::PdbEditor).
///
/// [`Default`] keeps the conventions of the source. Set a value with its
/// `with_*` function and read it with the function of the same name. A
/// later version can add options, for example an output profile, without a
/// change for callers.
///
/// The producer conventions, the hash bucket count and the hash streams
/// apply to the new layout: an editor from [`PdbEditor::new`](super::PdbEditor::new)
/// or after [`PdbEditor::decode_all`](super::PdbEditor::decode_all). If an
/// option has no value, the plan uses the convention of the source that
/// `decode_all` read. Without a source, it uses the conventions of Microsoft
/// `link.exe`: [`Producer::Microsoft`], [`DEFAULT_HASH_BUCKETS`](crate::native::types::DEFAULT_HASH_BUCKETS)
/// buckets and both hash streams. The copy-on-write layout keeps the
/// conventions of each source stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct EditOptions {
    /// The container options, or `None` for the page size of the source.
    msf: Option<MsfWriteOptions>,
    /// The producer of the layout of the TPI and IPI hash streams.
    type_hash_producer: Option<Producer>,
    /// The hash bucket count of the TPI and IPI streams.
    hash_buckets: Option<u32>,
    /// The producer of the conventions of the symbol indexes.
    index_producer: Option<Producer>,
    /// The producer of the conventions of the named-stream map.
    info_producer: Option<Producer>,
    /// `Some(true)` to write the TPI hash stream.
    type_hash_stream: Option<bool>,
    /// `Some(true)` to write the IPI hash stream.
    id_hash_stream: Option<bool>,
    /// `true` to strip the private symbols.
    strip_private: bool,
}

impl EditOptions {
    /// Return the container options, or `None` if the plan uses the page
    /// size of the source.
    ///
    /// With `None`, the plan uses the default [`MsfWriteOptions`] with the
    /// page size of an MSF 7.00 source. For an MSFZ source and without a
    /// source, it uses the default page size of [`MsfWriteOptions`].
    #[must_use]
    pub const fn msf(self) -> Option<MsfWriteOptions> {
        self.msf
    }

    /// Set the container options: the page size and the limits.
    #[must_use]
    pub const fn with_msf(mut self, msf: MsfWriteOptions) -> Self {
        self.msf = Some(msf);
        self
    }

    /// Return the producer whose layout of the TPI and IPI hash streams the
    /// new layout follows, or `None` for the convention of the source.
    #[must_use]
    pub const fn type_hash_producer(self) -> Option<Producer> {
        self.type_hash_producer
    }

    /// Set the producer whose layout of the TPI and IPI hash streams the new
    /// layout follows (see
    /// [`TypeStreamOptions::with_producer`](crate::native::types::TypeStreamOptions::with_producer)).
    #[must_use]
    pub const fn with_type_hash_producer(mut self, producer: Producer) -> Self {
        self.type_hash_producer = Some(producer);
        self
    }

    /// Return the hash bucket count of the TPI and IPI streams of the new
    /// layout, or `None` for the convention of the source.
    #[must_use]
    pub const fn hash_buckets(self) -> Option<u32> {
        self.hash_buckets
    }

    /// Set the hash bucket count of the TPI and IPI streams of the new layout.
    #[must_use]
    pub const fn with_hash_buckets(mut self, buckets: u32) -> Self {
        self.hash_buckets = Some(buckets);
        self
    }

    /// Return the producer whose conventions of the global and public symbol
    /// indexes the new layout follows, or `None` for the convention of the
    /// source.
    #[must_use]
    pub const fn index_producer(self) -> Option<Producer> {
        self.index_producer
    }

    /// Set the producer whose conventions of the global and public symbol
    /// indexes the new layout follows (see
    /// [`PublicSymbolIndex::encode_parts`](crate::native::symbols::PublicSymbolIndex::encode_parts)).
    #[must_use]
    pub const fn with_index_producer(mut self, producer: Producer) -> Self {
        self.index_producer = Some(producer);
        self
    }

    /// Return the producer whose conventions of the named-stream map of the
    /// Info stream the new layout follows, or `None` for the convention of
    /// the source.
    #[must_use]
    pub const fn info_producer(self) -> Option<Producer> {
        self.info_producer
    }

    /// Set the producer whose conventions of the named-stream map of the
    /// Info stream the new layout follows (see
    /// [`OwnedHashTable`](crate::native::hash_table::OwnedHashTable)).
    #[must_use]
    pub const fn with_info_producer(mut self, producer: Producer) -> Self {
        self.info_producer = Some(producer);
        self
    }

    /// Return `Some(true)` if the new layout has a TPI hash stream, or
    /// `None` for the convention of the source.
    #[must_use]
    pub const fn type_hash_stream(self) -> Option<bool> {
        self.type_hash_stream
    }

    /// Select whether the new layout has a TPI hash stream. Microsoft and
    /// LLVM linkers write it.
    #[must_use]
    pub const fn with_type_hash_stream(mut self, present: bool) -> Self {
        self.type_hash_stream = Some(present);
        self
    }

    /// Return `Some(true)` if the new layout has an IPI hash stream, or
    /// `None` for the convention of the source.
    #[must_use]
    pub const fn id_hash_stream(self) -> Option<bool> {
        self.id_hash_stream
    }

    /// Select whether the new layout has an IPI hash stream.
    #[must_use]
    pub const fn with_id_hash_stream(mut self, present: bool) -> Self {
        self.id_hash_stream = Some(present);
        self
    }

    /// Return `true` if the plan strips the private symbols.
    #[must_use]
    pub const fn strip_private(self) -> bool {
        self.strip_private
    }

    /// Select whether the plan strips the private symbols as Microsoft
    /// `link /PDBSTRIPPED` does.
    ///
    /// The strip is a mode of the plan, not a change of the model. See
    /// [`PdbEditor`](super::PdbEditor#strip-of-private-symbols) for the rules
    /// and the edits that the plan rejects together with a strip.
    #[must_use]
    pub const fn with_strip_private(mut self, strip: bool) -> Self {
        self.strip_private = strip;
        self
    }
}
