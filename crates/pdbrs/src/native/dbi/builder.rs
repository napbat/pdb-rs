//! An owned DBI stream that a PDB writer can fill.
//!
//! [`DbiBuilder`] holds the header fields and an owned form of each
//! substream. [`DbiBuilder::encode`] computes the substream sizes and checks
//! the width of each field. [`DbiBuilder::from_dbi`] copies a decoded stream,
//! so that a writer can change one value and keep all other bytes.

use alloc::vec::Vec;

use super::debug_header::encode_debug_streams;
use super::section_contribution::encode_section_contributions;
use super::write::encode_stream;
use super::{
    Dbi, DbiHeader, Module, SectionContribution, SectionContributionVersion, SectionMap,
    SourceInfo, Substream, TypeServer, to_u16,
};
use crate::native::msf::StreamId;
use crate::native::names::StringTableBuilder;
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// An owned DBI stream with the header fields and all substreams.
///
/// An empty `Vec` or `None` gives an empty substream. A
/// [`SectionContributionVersion::V60`] or [`SectionContributionVersion::V2`]
/// substream without records still has its 4-byte version value.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DbiBuilder {
    /// The header fields.
    pub header: DbiHeader,
    /// The module info records, in module order.
    pub modules: Vec<Module<'static>>,
    /// The layout of the section contribution records.
    pub section_contribution_version: SectionContributionVersion,
    /// The section contributions. The fields must match the layout.
    pub section_contributions: Vec<SectionContribution>,
    /// The section map, or `None` for an empty substream.
    pub section_map: Option<SectionMap>,
    /// The source file info, or `None` for an empty substream.
    pub source_info: Option<SourceInfo<'static>>,
    /// The type server map records.
    pub type_servers: Vec<TypeServer>,
    /// The EC name table, or `None` for an empty substream.
    pub ec_names: Option<StringTableBuilder>,
    /// The slots of the optional debug header, with the slots after the named slots.
    pub debug_streams: Vec<Option<StreamId>>,
}

impl DbiBuilder {
    /// Create a stream with `header` and empty substreams.
    ///
    /// The section contribution substream gets the
    /// [`SectionContributionVersion::V60`] layout, as Microsoft and LLVM
    /// linkers write it.
    #[must_use]
    pub const fn new(header: DbiHeader) -> Self {
        Self {
            header,
            modules: Vec::new(),
            section_contribution_version: SectionContributionVersion::V60,
            section_contributions: Vec::new(),
            section_map: None,
            source_info: None,
            type_servers: Vec::new(),
            ec_names: None,
            debug_streams: Vec::new(),
        }
    }

    /// Copy all values of a decoded DBI stream.
    ///
    /// [`DbiBuilder::encode`] of the copy gives the bytes of `dbi` again.
    ///
    /// # Errors
    ///
    /// - The errors of the substream decoders of [`Dbi`].
    /// - The errors of [`StringTableBuilder::from_table`] for the EC names.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn from_dbi(dbi: &Dbi) -> Result<Self> {
        let mut builder = Self::new(*dbi.header());
        for module in dbi.modules() {
            let module = module?.1.into_owned();
            builder.modules.try_reserve(1).map_err(|_| no_memory())?;
            builder.modules.push(module);
        }
        builder.section_contribution_version = SectionContributionVersion::V40;
        if let Some(contributions) = dbi.section_contributions()? {
            builder.section_contribution_version = contributions.version();
            builder
                .section_contributions
                .try_reserve_exact(contributions.len())
                .map_err(|_| no_memory())?;
            builder.section_contributions.extend(contributions.iter());
        }
        builder.section_map = dbi.section_map()?;
        builder.source_info = dbi.source_info()?.map(SourceInfo::into_owned);
        builder.type_servers = dbi.type_server_map()?;
        if let Some(names) = dbi.ec_names()? {
            builder.ec_names = Some(StringTableBuilder::from_table(&names)?);
        }
        let slots = dbi.debug_streams();
        builder
            .debug_streams
            .try_reserve_exact(slots.len())
            .map_err(|_| no_memory())?;
        builder.debug_streams.extend(slots);
        Ok(builder)
    }

    /// Append the complete DBI stream to `out`.
    ///
    /// The function writes the header with the computed substream sizes and
    /// the substreams in disk order. If it returns an error, it removes the
    /// bytes that it appended.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if there are more than 65535 modules,
    ///   if a substream is larger than `i32::MAX` bytes, if a stream index is
    ///   larger than `0xfffe`, if a count does not fit its field or if the
    ///   output cannot grow.
    /// - [`ErrorKind::InvalidFormat`] if the source info module count is not
    ///   the module count, if a name contains a NUL byte or if a section
    ///   contribution does not match its layout.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        to_u16(self.modules.len(), "DBI has more than 65535 modules")?;
        if let Some(info) = &self.source_info
            && info.modules.len() != self.modules.len()
        {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("DBI source info module count differs from the module info count"));
        }
        encode_stream(&self.header, out, |kind, out| {
            self.encode_substream(kind, out)
        })
    }

    /// Return the complete DBI stream.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`DbiBuilder::encode`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.encode(&mut bytes)?;
        Ok(bytes)
    }

    /// Append one substream to `out`.
    ///
    /// # Errors
    ///
    /// Returns the errors of the encoders of the substream records.
    pub fn encode_substream(&self, kind: Substream, out: &mut Vec<u8>) -> Result<()> {
        match kind {
            Substream::ModuleInfo => {
                for module in &self.modules {
                    module.encode(out)?;
                }
            }
            Substream::SectionContributions => encode_section_contributions(
                self.section_contribution_version,
                self.section_contributions.iter().copied(),
                out,
            )?,
            Substream::SectionMap => {
                if let Some(map) = &self.section_map {
                    map.encode(out)?;
                }
            }
            Substream::SourceInfo => {
                if let Some(info) = &self.source_info {
                    info.encode(out)?;
                }
            }
            Substream::TypeServerMap => {
                for server in &self.type_servers {
                    server.encode(out)?;
                }
            }
            Substream::Ec => {
                if let Some(names) = &self.ec_names {
                    names.encode(out)?;
                }
            }
            Substream::OptionalDebugHeader => {
                encode_debug_streams(self.debug_streams.iter().copied(), out)?;
            }
        }
        Ok(())
    }
}
