//! The `sourcelink` and `sourcelink$<n>` streams: Source Link JSON.
//!
//! The .NET symbol writers store the Source Link file of the build in the named stream
//! `sourcelink`. The Portable PDB Source Link custom debug information
//! (`cc110556-a091-4d38-9fec-25ab9a351a6a`) holds the same UTF-8 JSON text. The JSON maps
//! local document paths to URLs. This reader keeps the bytes. It does not parse the JSON and
//! does not fetch the URLs.
//!
//! `link /SOURCELINK:<file>` (MSVC 14.51) writes the streams `sourcelink$1`, `sourcelink$2`
//! and so on: one stream for each `/SOURCELINK` file, in the order of the options. After
//! the last file, it writes one more empty stream with the next number.

use alloc::vec::Vec;

use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// A decoded `sourcelink` stream, borrowed from the stream bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceLink<'a> {
    /// The JSON text.
    json: &'a str,
}

/// Return the bytes of a Source Link file as text if they have the shape that
/// [`SourceLink::parse`] accepts. Else return the reason.
///
/// The Portable PDB editor uses the same rules for the Source Link custom
/// debug information.
pub(crate) fn json_object(bytes: &[u8]) -> core::result::Result<&str, &'static str> {
    let json = core::str::from_utf8(bytes).map_err(|_| "Source Link stream is not UTF-8")?;
    let object = json.trim_start_matches('\u{feff}').trim();
    if object.starts_with('{') && object.ends_with('}') {
        Ok(json)
    } else {
        Err("Source Link stream is not a JSON object")
    }
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
        let json = json_object(bytes).map_err(|context| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context(context)
                .with_stream(stream, 0)
        })?;
        Ok(Self { json })
    }

    /// Decode the numbered Source Link stream `stream` (`sourcelink$<n>`).
    ///
    /// The bytes must have the shape that [`SourceLink::parse`] accepts, or be empty. The
    /// empty stream is the end marker of `link /SOURCELINK`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`SourceLink::parse`] for bytes that are not empty.
    pub fn parse_numbered(bytes: &'a [u8], stream: StreamId) -> Result<Self> {
        if bytes.is_empty() {
            return Ok(Self { json: "" });
        }
        Self::parse(bytes, stream)
    }

    /// Return `true` for the empty end marker of the numbered streams.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.json.is_empty()
    }

    /// Return the JSON text with its original bytes.
    #[must_use]
    pub const fn json(&self) -> &'a str {
        self.json
    }

    /// Append the stream bytes, the JSON text, to `out`.
    ///
    /// A decoded stream gives the same bytes again. To make a new stream, decode the JSON
    /// text with [`SourceLink::parse`] first: it checks the shape.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).bytes(self.json.as_bytes())
    }
}
