//! The `sourcelink` stream: Source Link JSON.
//!
//! The .NET symbol writers store the Source Link file of the build in the named stream
//! `sourcelink`. The Portable PDB Source Link custom debug information
//! (`cc110556-a091-4d38-9fec-25ab9a351a6a`) holds the same UTF-8 JSON text. The JSON maps
//! local document paths to URLs. This reader keeps the bytes. It does not parse the JSON and
//! does not fetch the URLs.

use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// A decoded `sourcelink` stream, borrowed from the stream bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceLink<'a> {
    /// The JSON text.
    json: &'a str,
}

impl<'a> SourceLink<'a> {
    /// Decode the `sourcelink` stream `stream`.
    ///
    /// The bytes must be UTF-8 text. After white space and an optional byte order mark, the
    /// text must start with `{` and end with `}`: the shape of a JSON object. The function does
    /// not check the JSON syntax inside the object.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] with the stream ID and logical offset 0 if the bytes
    /// are not UTF-8 or do not have the shape of a JSON object.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Self> {
        let invalid = |context| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context(context)
                .with_stream(stream.0, 0)
        };
        let json =
            core::str::from_utf8(bytes).map_err(|_| invalid("Source Link stream is not UTF-8"))?;
        let object = json.trim_start_matches('\u{feff}').trim();
        if !(object.starts_with('{') && object.ends_with('}')) {
            return Err(invalid("Source Link stream is not a JSON object"));
        }
        Ok(Self { json })
    }

    /// Return the JSON text with its original bytes.
    #[must_use]
    pub const fn json(&self) -> &'a str {
        self.json
    }
}
