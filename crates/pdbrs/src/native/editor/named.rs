//! The named streams of the editor model.

use alloc::vec::Vec;

use crate::native::auxiliary::NamedStreamKind;
use crate::native::info::Info;
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::{Bytes, Error, ErrorKind, Result};

/// The contents of a named stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Content {
    /// The source stream.
    Source,
    /// New bytes.
    Bytes(Vec<u8>),
    /// The source stream is removed.
    Removed,
}

/// One named stream.
#[derive(Clone, Debug)]
pub(super) struct NamedEntry {
    /// The name.
    pub(super) name: Bytes<'static>,
    /// The source stream, or `None` for a new name.
    pub(super) source: Option<StreamId>,
    /// The contents.
    pub(super) content: Content,
}

/// The named streams of a PDB other than `/names`, in the order of the
/// named-stream map.
///
/// [`PdbEditor::named_streams_mut`](super::PdbEditor::named_streams_mut)
/// gives the collection. In the copy-on-write layout, a replaced stream
/// keeps its stream ID, a new stream gets a new stream ID after the source
/// streams, and a removed stream becomes a nil stream. The plan decodes the
/// bytes of a `srcsrv`, `sourcelink` or `/LinkInfo` stream with the decoder
/// of its kind; it keeps the bytes of other names as they are.
#[derive(Clone, Debug, Default)]
pub struct NamedStreams {
    /// The entries.
    pub(super) entries: Vec<NamedEntry>,
    /// `true` if the editor keeps source streams: the collection then
    /// rejects a change of a stream whose contents other records name.
    pub(super) restricted: bool,
}

impl NamedStreams {
    /// Make the collection of the named streams of a source Info stream.
    pub(super) fn from_info(info: &Info) -> Result<Self> {
        let mut entries = Vec::new();
        for named in info.named_streams() {
            if NamedStreamKind::classify(&named.name) == NamedStreamKind::Names {
                continue;
            }
            entries.try_reserve(1).map_err(|_| no_memory())?;
            entries.push(NamedEntry {
                name: Bytes::from(named.name.as_bytes()).into_owned(),
                source: Some(named.stream),
                content: Content::Source,
            });
        }
        Ok(Self {
            entries,
            restricted: true,
        })
    }

    /// Return the position of the entry with `name`.
    fn position(&self, name: &[u8]) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.name.as_bytes() == name)
    }

    /// Return an error if the collection does not accept a change of `name`.
    fn check(&self, name: &[u8]) -> Result<()> {
        if name.is_empty() || name.contains(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("named stream name is empty or contains a NUL byte"));
        }
        let kind = NamedStreamKind::classify(name);
        if kind == NamedStreamKind::Names {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("/names is not a named stream of the collection; use names_mut"));
        }
        if self.restricted
            && matches!(
                kind,
                NamedStreamKind::TmCache
                    | NamedStreamKind::UdtSourceLineUndone
                    | NamedStreamKind::SourceHeaderBlock
                    | NamedStreamKind::SourceFile { .. }
            )
        {
            return Err(Error::new(ErrorKind::UnsupportedEdit, None)
                .with_context("other records refer to the contents of this named stream"));
        }
        Ok(())
    }

    /// Add a named stream with `bytes`, or replace the bytes of a named
    /// stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name is empty, contains a NUL
    ///   byte or is `/names`.
    /// - [`ErrorKind::UnsupportedEdit`] in the copy-on-write layout for
    ///   `/TMCache`, `/UDTSRCLINEUNDONE` or a `/src/` name. Other records
    ///   refer to the contents of these streams.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn insert(&mut self, name: &[u8], bytes: Vec<u8>) -> Result<()> {
        self.check(name)?;
        if let Some(position) = self.position(name) {
            self.entries[position].content = Content::Bytes(bytes);
            return Ok(());
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(name.len())
            .map_err(|_| no_memory())?;
        owned.extend_from_slice(name);
        self.entries.try_reserve(1).map_err(|_| no_memory())?;
        self.entries.push(NamedEntry {
            name: Bytes::from(owned),
            source: None,
            content: Content::Bytes(bytes),
        });
        Ok(())
    }

    /// Remove a named stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the collection has no stream with
    ///   this name.
    /// - The errors of [`NamedStreams::insert`] for the name.
    pub fn remove(&mut self, name: &[u8]) -> Result<()> {
        self.check(name)?;
        let position = self
            .position(name)
            .filter(|&position| self.entries[position].content != Content::Removed)
            .ok_or_else(|| {
                Error::new(ErrorKind::MissingStream, None)
                    .with_context("the PDB has no named stream with the name to remove")
            })?;
        if self.entries[position].source.is_some() {
            self.entries[position].content = Content::Removed;
        } else {
            self.entries.remove(position);
        }
        Ok(())
    }

    /// Return `true` if the collection has a stream with `name`.
    #[must_use]
    pub fn contains(&self, name: &[u8]) -> bool {
        self.position(name)
            .is_some_and(|position| self.entries[position].content != Content::Removed)
    }

    /// Return the names, in the order of the named-stream map.
    pub fn names(&self) -> impl Iterator<Item = &[u8]> + '_ {
        self.entries
            .iter()
            .filter(|entry| entry.content != Content::Removed)
            .map(|entry| entry.name.as_bytes())
    }

    /// Return the number of named streams.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names().count()
    }

    /// Return `true` if the collection has no named stream.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
