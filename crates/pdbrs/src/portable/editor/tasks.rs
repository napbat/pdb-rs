//! The editing tasks of #41 for Portable PDBs: functions over the model of the editor.
//!
//! Each task checks all its inputs before it changes the model, so an error leaves the
//! model unchanged. The tasks use only the public collections of the editor.

use alloc::vec::Vec;

use super::model::PdbEditor;
use crate::io::ReadAt;
use crate::native::auxiliary::json_object;
use crate::portable::compressed::MAX_UNSIGNED;
use crate::portable::custom::{CustomDebugInformation, CustomDebugKind, CustomDebugParent};
use crate::portable::embedded::{EmbeddedFormat, EmbeddedSource};
use crate::portable::metadata::{DocumentId, MetadataToken};
use crate::{Bytes, Error, ErrorKind, Result};

/// The metadata token of the module: the parent of the Source Link record.
const MODULE: MetadataToken = MetadataToken::from_raw(0x0000_0001);

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Set the Source Link file of the PDB: the Source Link custom debug information
    /// ([`CustomDebugKind::SOURCE_LINK`]) of the module.
    ///
    /// The task replaces the value of the record of the module, or adds a record. It checks
    /// `json` with the rules of the native `sourcelink` stream: UTF-8 text with the shape of
    /// a JSON object.
    ///
    /// ```no_run
    /// use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdbrs::Result<()> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
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
    /// - [`ErrorKind::InvalidFormat`] if `json` is not UTF-8 or does not have the shape of
    ///   a JSON object, or if the module has more than one Source Link record.
    /// - The errors of [`PdbEditor::custom_debug_information_mut`] and of
    ///   [`CustomDebugInformations::push`](super::CustomDebugInformations::push).
    ///
    /// On an error, the model does not change.
    pub fn set_source_link(&mut self, json: Vec<u8>) -> Result<()> {
        json_object(&json)
            .map_err(|context| Error::new(ErrorKind::InvalidFormat, None).with_context(context))?;
        self.set_custom(
            CustomDebugParent::Metadata(MODULE),
            CustomDebugKind::SOURCE_LINK,
            json,
        )
    }

    /// Embed the source text `source` of `document` in the PDB: the embedded source custom
    /// debug information ([`CustomDebugKind::EMBEDDED_SOURCE`]) of the document.
    ///
    /// The task replaces the value of the record of the document, or adds a record.
    /// `format` selects the storage form:
    ///
    /// - [`EmbeddedFormat::Raw`]: the blob holds the bytes.
    /// - [`EmbeddedFormat::Deflate`]: the blob holds a raw DEFLATE stream and the decoded
    ///   size. An empty source gets the raw form, because the size 0 marks the raw form.
    ///
    /// The task does not compare `source` with the hash of the document. A debugger can
    /// reject a source whose hash differs. If you embed other bytes, also change
    /// [`Document::hash`](crate::portable::debug::Document::hash).
    ///
    /// ```no_run
    /// use pdbrs::portable::embedded::EmbeddedFormat;
    /// use pdbrs::portable::metadata::DocumentId;
    /// use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    /// let source = std::fs::read("Program.cs")?;
    /// editor.embed_source(DocumentId(1), &source, EmbeddedFormat::Deflate)?;
    /// editor.plan(EditOptions::default())?.write_file("app.embedded.pdb")?;
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the editor has no document `document`.
    /// - [`ErrorKind::LimitExceeded`] if the DEFLATE form gets more than `i32::MAX` source
    ///   bytes, if the blob is longer than `0x1FFF_FFFF` bytes (the largest blob of the
    ///   `#Blob` heap), or if an allocation fails.
    /// - [`ErrorKind::InvalidFormat`] if the document has more than one embedded source
    ///   record, or if the DEFLATE encoder does not finish the stream.
    /// - The errors of [`PdbEditor::documents_mut`] and
    ///   [`PdbEditor::custom_debug_information_mut`].
    ///
    /// On an error, the model does not change.
    pub fn embed_source(
        &mut self,
        document: DocumentId,
        source: &[u8],
        format: EmbeddedFormat,
    ) -> Result<()> {
        if self.documents_mut()?.get(document).is_none() {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("the editor has no document with this ID"));
        }
        let mut blob = Vec::new();
        EmbeddedSource::encode_parts(source, format, &mut blob)?;
        if u32::try_from(blob.len()).map_or(true, |length| length > MAX_UNSIGNED) {
            return Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("embedded source blob is longer than the #Blob heap allows"));
        }
        self.set_custom(
            CustomDebugParent::Document(document),
            CustomDebugKind::EMBEDDED_SOURCE,
            blob,
        )
    }

    /// Give the one record of `kind` for `parent` the value `value`, or add a record.
    fn set_custom(
        &mut self,
        parent: CustomDebugParent,
        kind: CustomDebugKind,
        value: Vec<u8>,
    ) -> Result<()> {
        let custom = self.custom_debug_information_mut()?;
        let (first, more) = {
            let mut found = custom
                .iter()
                .filter(|(_, information)| information.parent == parent && information.kind == kind)
                .map(|(handle, _)| handle);
            (found.next(), found.next().is_some())
        };
        if more {
            return Err(Error::new(ErrorKind::InvalidFormat, None).with_context(
                "the parent has more than one custom debug information record of this kind",
            ));
        }
        if let Some(information) = first.and_then(|handle| custom.get_mut(handle)) {
            information.value = Bytes::from(value);
            return Ok(());
        }
        let mut information = CustomDebugInformation::new(parent, kind);
        information.value = Bytes::from(value);
        custom.push(information)?;
        Ok(())
    }
}
