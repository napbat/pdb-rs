//! The editing tasks: functions over the model of the editor.
//!
//! Each task checks all its inputs before it changes the model, so an error
//! leaves the model unchanged. The plan has no separate path for a task.

mod rename;

use alloc::vec::Vec;

use super::PdbEditor;
use crate::io::ReadAt;
use crate::native::auxiliary::{SourceServer, json_object};
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The name of the Source Link stream.
const SOURCE_LINK: &[u8] = b"sourcelink";
/// The name of the source server stream.
const SOURCE_SERVER: &[u8] = b"srcsrv";

/// Return `error` with the stream `stream`, or without a stream location if
/// the stream is new.
fn locate(error: Error, stream: Option<StreamId>) -> Error {
    match (stream, error.stream_location()) {
        (Some(_), _) | (None, None) => error,
        (None, Some(_)) => {
            let plain = Error::new(error.kind(), None);
            match error.context() {
                Some(context) => plain.with_context(context),
                None => plain,
            }
        }
    }
}

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Return the source stream of the named stream `name`, if the model has
    /// the name and the source has its stream.
    fn named_source(&mut self, name: &[u8]) -> Result<Option<StreamId>> {
        Ok(self
            .named_streams_mut()?
            .entries
            .iter()
            .find(|entry| entry.name.as_bytes() == name)
            .and_then(|entry| entry.source))
    }

    /// Set the Source Link file of the PDB: add or replace the `sourcelink`
    /// named stream.
    ///
    /// The task checks `json` with the rules of
    /// [`SourceLink::parse`](crate::native::auxiliary::SourceLink::parse):
    /// UTF-8 text with the shape of a JSON object. It does not change the
    /// symbols, so [`IdentityPolicy::Preserve`](crate::native::IdentityPolicy::Preserve)
    /// keeps the match with the image.
    ///
    /// ```no_run
    /// use pdbrs::native::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdbrs::Result<()> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::Preserve);
    /// let json = br#"{"documents":{"C:\\src\\*":"https://example.com/src/*"}}"#;
    /// editor.set_source_link(json.to_vec())?;
    /// editor.plan(EditOptions::default())?.write_file("app.linked.pdb")?;
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `json` is not UTF-8 or does not have
    ///   the shape of a JSON object. The error has the source stream of
    ///   `sourcelink`, if the source has one.
    /// - The errors of [`PdbEditor::named_streams_mut`] and
    ///   [`NamedStreams::insert`](super::NamedStreams::insert).
    ///
    /// On an error, the model does not change.
    pub fn set_source_link(&mut self, json: Vec<u8>) -> Result<()> {
        let stream = self.named_source(SOURCE_LINK)?;
        if let Err(context) = json_object(&json) {
            let error = Error::new(ErrorKind::InvalidFormat, None).with_context(context);
            return Err(match stream {
                Some(stream) => error.with_stream(stream, 0),
                None => error,
            });
        }
        self.named_streams_mut()?.insert(SOURCE_LINK, json)
    }

    /// Set the source server data of the PDB: add or replace the `srcsrv`
    /// named stream.
    ///
    /// The task checks `text` with
    /// [`SourceServer::parse`](crate::native::auxiliary::SourceServer::parse).
    /// `pdbstr -r -s:srcsrv` reads the same text. The task does not change the
    /// symbols, so [`IdentityPolicy::Preserve`](crate::native::IdentityPolicy::Preserve)
    /// keeps the match with the image.
    ///
    /// ```no_run
    /// use pdbrs::native::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdbrs::Result<()> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::Preserve);
    /// let text = "SRCSRV: ini ------------------------------------------------\r\n\
    ///             VERSION=2\r\n\
    ///             SRCSRV: variables ------------------------------------------\r\n\
    ///             SRCSRVTRG=%targ%\\%var2%\r\n\
    ///             SRCSRV: source files ---------------------------------------\r\n\
    ///             C:\\src\\app.cpp*app.cpp\r\n\
    ///             SRCSRV: end ------------------------------------------------\r\n";
    /// editor.set_source_server(text.as_bytes().to_vec())?;
    /// editor.plan(EditOptions::default())?.write_file("app.indexed.pdb")?;
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Errors
    ///
    /// - The errors of [`SourceServer::parse`] for `text`. The error has the
    ///   source stream of `srcsrv`, if the source has one, and no stream
    ///   location otherwise.
    /// - The errors of [`PdbEditor::named_streams_mut`] and
    ///   [`NamedStreams::insert`](super::NamedStreams::insert).
    ///
    /// On an error, the model does not change.
    pub fn set_source_server(&mut self, text: Vec<u8>) -> Result<()> {
        let stream = self.named_source(SOURCE_SERVER)?;
        SourceServer::parse(&text, stream.unwrap_or(StreamId(0)))
            .map_err(|error| locate(error, stream))?;
        self.named_streams_mut()?.insert(SOURCE_SERVER, text)
    }

    /// Remove the named stream `name` and its entry of the Info named-stream
    /// map.
    ///
    /// In the copy-on-write layout, the plan writes a nil stream in place of
    /// the source stream, so the other stream IDs stay.
    ///
    /// ```no_run
    /// use pdbrs::native::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdbrs::Result<()> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::Preserve);
    /// editor.remove_named_stream(b"/LinkInfo")?;
    /// editor.plan(EditOptions::default())?.write_file("app.small.pdb")?;
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the model has no named stream `name`.
    /// - [`ErrorKind::InvalidFormat`] for an empty name, a name with a NUL
    ///   byte, or `/names`.
    /// - [`ErrorKind::UnsupportedEdit`] in the copy-on-write layout for
    ///   `/TMCache`, `/UDTSRCLINEUNDONE` or a `/src/` name: other records refer
    ///   to their contents.
    /// - The errors of [`PdbEditor::named_streams_mut`].
    ///
    /// On an error, the model does not change.
    pub fn remove_named_stream(&mut self, name: &[u8]) -> Result<()> {
        self.named_streams_mut()?.remove(name)
    }
}
