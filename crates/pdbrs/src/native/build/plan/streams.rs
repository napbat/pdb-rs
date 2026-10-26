//! The DBI stream, the Info stream and the other streams of a write plan.

use alloc::vec::Vec;

use super::super::PdbBuilder;
use super::super::entry::Entry;
use super::super::handle::Resolver;
use super::{BuildOptions, StreamBytes, StreamIds};
use crate::native::Identity;
use crate::native::dbi::{DbiBuilder, OwnedModule, OwnedSourceInfo};
use crate::native::info::InfoBuilder;
use crate::native::msf::MsfBuilder;
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// Return the bytes of a stream entry.
///
/// A value stays borrowed. A deferred stream is encoded now.
fn stream_bytes<'b>(
    entry: &'b Entry<'_, Vec<u8>>,
    resolver: &Resolver<'_>,
) -> Result<StreamBytes<'b>> {
    match entry {
        Entry::Value(bytes) => Ok(StreamBytes::Borrowed(bytes)),
        Entry::Deferred(encode) => {
            let mut bytes = Vec::new();
            encode(resolver, &mut bytes)?;
            Ok(StreamBytes::Owned(bytes))
        }
    }
}

impl PdbBuilder<'_> {
    /// Encode the DBI stream.
    pub(super) fn dbi_stream(&self, ids: &StreamIds, modules: Vec<OwnedModule>) -> Result<Vec<u8>> {
        let mut header = self.dbi.header;
        header.age = self.dbi_age;
        header.global_stream = Some(ids.globals);
        header.public_stream = Some(ids.publics);
        header.symbol_stream = Some(ids.symbols);
        let mut dbi = DbiBuilder::new(header);
        dbi.modules = modules;
        dbi.section_contribution_version = self.dbi.contribution_version;
        dbi.section_contributions
            .try_reserve_exact(self.dbi.contributions.len())
            .map_err(|_| no_memory())?;
        dbi.section_contributions
            .extend_from_slice(&self.dbi.contributions);
        dbi.section_map.clone_from(&self.dbi.section_map);
        dbi.type_servers.clone_from(&self.dbi.type_servers);
        dbi.ec_names.clone_from(&self.dbi.ec_names);
        if !self.modules.is_empty() {
            dbi.source_info = Some(OwnedSourceInfo::from_file_names(
                self.modules
                    .iter()
                    .map(|module| module.files.iter().copied()),
            )?);
        }
        let slots = self
            .debug_streams
            .iter()
            .map(|(slot, _)| slot + 1)
            .fold(self.dbi.debug_slots, usize::max);
        dbi.debug_streams
            .try_reserve_exact(slots)
            .map_err(|_| no_memory())?;
        dbi.debug_streams.resize(slots, None);
        for ((slot, _), id) in self.debug_streams.iter().zip(&ids.debug) {
            dbi.debug_streams[*slot] = Some(*id);
        }
        dbi.to_bytes()
    }

    /// Push the optional debug streams, `/names`, the named streams and the
    /// streams without a name, in stream ID order.
    pub(super) fn push_other_streams<'b>(
        &'b self,
        resolver: &Resolver<'_>,
        msf: &mut MsfBuilder<StreamBytes<'b>>,
    ) -> Result<()> {
        for (_, entry) in &self.debug_streams {
            msf.push(stream_bytes(entry, resolver)?)?;
        }
        if let Some(names) = &self.names {
            let mut bytes = Vec::new();
            names.encode(&mut bytes)?;
            msf.push(StreamBytes::Owned(bytes))?;
        }
        for (_, entry) in &self.named {
            msf.push(stream_bytes(entry, resolver)?)?;
        }
        for entry in &self.streams {
            msf.push(stream_bytes(entry, resolver)?)?;
        }
        Ok(())
    }

    /// Encode the Info stream with the named-stream map.
    ///
    /// `/names` comes first in the map, then the named streams in builder
    /// order.
    pub(super) fn info_stream(&self, ids: &StreamIds, options: &BuildOptions) -> Result<Vec<u8>> {
        let Identity::Modern { guid, age } = self.info.identity() else {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("PDB Info stream has no GUID identity"));
        };
        let mut info = InfoBuilder::new(options.info_producer(), self.info.signature(), guid, age)?;
        let mut features = Vec::new();
        features
            .try_reserve_exact(self.info.features().len())
            .map_err(|_| no_memory())?;
        features.extend(self.info.features());
        info.set_features(&features)?;
        if let Some(names) = ids.names {
            info.set_named_stream(b"/names", names)?;
        }
        for ((name, _), id) in self.named.iter().zip(&ids.named) {
            info.set_named_stream(name, *id)?;
        }
        let mut bytes = Vec::new();
        info.encode(&mut bytes)?;
        Ok(bytes)
    }
}
