//! Natvis files in injected sources.
//!
//! MSVC `link.exe /NATVIS:<file>` and LLVM `lld-link /natvis:<file>` store a Natvis file as
//! an injected source. The Rust toolchain passes its Natvis files to the linker in this way.
//! A debugger reads the XML file to show values of the types in the PDB.
//!
//! This module checks a minimal shape: the bytes are UTF-8 text, with an optional byte order
//! mark, and the root element is `AutoVisualizer`. It does not check the XML syntax or the
//! Natvis schema.

use crate::{Error, ErrorKind, Result};

/// The UTF-8 byte order mark.
const BOM: &[u8] = b"\xef\xbb\xbf";

/// The name of the Natvis root element.
const ROOT: &str = "AutoVisualizer";

/// Return `true` if `virtual_file_name` ends with `.natvis`. The comparison ignores the case
/// of ASCII letters.
#[must_use]
pub fn is_natvis_name(virtual_file_name: &[u8]) -> bool {
    const SUFFIX: &[u8] = b".natvis";
    virtual_file_name.len() >= SUFFIX.len()
        && virtual_file_name[virtual_file_name.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
}

/// The XML text of a Natvis file, borrowed from the content bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Natvis<'a> {
    /// The XML text after the byte order mark.
    xml: &'a str,
    /// `true` if the bytes start with a UTF-8 byte order mark.
    byte_order_mark: bool,
}

impl<'a> Natvis<'a> {
    /// Check the shape of the Natvis file `content`.
    ///
    /// Before the root element, the text can have white space, an XML declaration, processing
    /// instructions, comments and a document type declaration.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] without a location if the bytes are not UTF-8 or
    /// the root element is not `AutoVisualizer`.
    pub fn parse(content: &'a [u8]) -> Result<Self> {
        let invalid = |context| Error::new(ErrorKind::InvalidFormat, None).with_context(context);
        let (byte_order_mark, text) = match content.strip_prefix(BOM) {
            Some(text) => (true, text),
            None => (false, content),
        };
        let xml = core::str::from_utf8(text).map_err(|_| invalid("Natvis file is not UTF-8"))?;
        let mut rest = xml;
        loop {
            rest = rest.trim_start();
            let skipped = if rest.starts_with("<?") {
                rest.find("?>").map(|end| end + 2)
            } else if rest.starts_with("<!--") {
                rest.find("-->").map(|end| end + 3)
            } else if rest.starts_with("<!") {
                rest.find('>').map(|end| end + 1)
            } else {
                break;
            };
            let end = skipped.ok_or_else(|| invalid("Natvis file has an unterminated prolog"))?;
            rest = &rest[end..];
        }
        let root = rest
            .strip_prefix('<')
            .and_then(|tag| tag.strip_prefix(ROOT))
            .ok_or_else(|| invalid("Natvis root element is not AutoVisualizer"))?;
        if !root.starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/') {
            return Err(invalid("Natvis root element is not AutoVisualizer"));
        }
        Ok(Self {
            xml,
            byte_order_mark,
        })
    }

    /// Return the XML text, without the byte order mark.
    #[must_use]
    pub const fn xml(&self) -> &'a str {
        self.xml
    }

    /// Return `true` if the content starts with a UTF-8 byte order mark.
    #[must_use]
    pub const fn has_byte_order_mark(&self) -> bool {
        self.byte_order_mark
    }
}
