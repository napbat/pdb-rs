//! The named stream edits: the Info named-stream map and the stream bytes.

use super::NamedChange;
use super::plan::Changes;
use crate::native::auxiliary::{LinkInfo, NamedStreamKind, SourceLink, SourceServer};
use crate::native::info::InfoBuilder;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// Decode `bytes` with the decoder of the kind of `name`, if the kind has one.
///
/// The edit API rejects the kinds that other records refer to, so only
/// `srcsrv`, `sourcelink`, `sourcelink$<n>`, `/LinkInfo` and names without a
/// decoder come here.
fn check_contents(name: &[u8], bytes: &[u8], stream: StreamId) -> Result<()> {
    match NamedStreamKind::classify(name) {
        NamedStreamKind::SourceServer => SourceServer::parse(bytes, stream).map(|_| ()),
        NamedStreamKind::SourceLink => SourceLink::parse(bytes, stream).map(|_| ()),
        NamedStreamKind::NumberedSourceLink { .. } => {
            SourceLink::parse_numbered(bytes, stream).map(|_| ())
        }
        NamedStreamKind::LinkInfo => LinkInfo::parse(bytes, stream).map(|_| ()),
        _ => Ok(()),
    }
}

/// Apply the named stream changes in call order.
///
/// A replaced stream keeps its stream ID. A new stream gets the next new
/// stream ID. A removed stream becomes a nil stream.
pub(super) fn plan(
    named: &[NamedChange],
    info: &mut InfoBuilder,
    changes: &mut Changes,
) -> Result<()> {
    for change in named {
        let existing = info.named_stream(&change.name)?;
        match (&change.bytes, existing) {
            (Some(bytes), Some(stream)) => {
                check_contents(&change.name, bytes, stream)?;
                changes.replace(stream, bytes.clone())?;
            }
            (Some(bytes), None) => {
                let stream = changes.add(bytes.clone())?;
                check_contents(&change.name, bytes, stream)?;
                info.set_named_stream(&change.name, stream)?;
            }
            (None, Some(stream)) => {
                info.remove_named_stream(&change.name)?;
                changes.remove(stream)?;
            }
            (None, None) => {
                return Err(Error::new(ErrorKind::MissingStream, None)
                    .with_context("the PDB has no named stream with the name to remove"));
            }
        }
    }
    Ok(())
}
