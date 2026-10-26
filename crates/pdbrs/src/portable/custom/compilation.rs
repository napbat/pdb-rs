//! Custom debug information about the compilation of a module: compilation
//! options and compilation metadata references.
//!
//! The layouts follow the "Compilation Metadata References" and "Compilation
//! Options" sections of the Portable PDB specification, and
//! `EmbedCompilationOptions` and `EmbedMetadataReferenceInformation` in Roslyn
//! `MetadataWriter.PortablePdb.cs` (commit
//! `e3c31f15b79e226656e3b082a921d4d60127b1fa`).

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::blob::{Reader, Writer, invalid};
use super::{Entries, Utf8Bytes, count_to_end};
use crate::Result;

/// The compilation options of a module.
///
/// The blob is a list of NUL-terminated UTF-8 name and value pairs. The names
/// are unique. Their order has no meaning.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompilationOptions<'a> {
    /// The pairs. [`CompilationOptions::parse`] checked them.
    bytes: &'a [u8],
    /// The number of pairs.
    len: usize,
}

/// One compilation option.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompilationOption<'a> {
    /// The name, for example `language`.
    pub name: Utf8Bytes<'a>,
    /// The value.
    pub value: Utf8Bytes<'a>,
}

impl<'a> CompilationOptions<'a> {
    /// Decode a compilation options blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if a
    ///   name or a value has no NUL terminator, for example a name without a
    ///   value.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if two
    ///   options have the same name.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let len = count_to_end(blob, 0, read_option)?;
        let options = Self { bytes: blob, len };
        unique_names(options.iter())?;
        Ok(options)
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the number of options.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no options.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the options in blob order.
    #[must_use]
    pub const fn iter(&self) -> Entries<'a, CompilationOption<'a>> {
        Entries::new(self.bytes, self.len, 0, read_option)
    }

    /// Return the value of the option `name`.
    #[must_use]
    pub fn get(&self, name: &[u8]) -> Option<Utf8Bytes<'a>> {
        self.iter()
            .find(|option| option.name.as_bytes() == name)
            .map(|option| option.value)
    }

    /// Encode a compilation options blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   name or a value has a NUL byte, or if two options have the same name.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if an
    ///   allocation fails.
    pub fn encode<'o>(
        options: impl IntoIterator<Item = CompilationOption<'o>, IntoIter: Clone>,
    ) -> Result<Vec<u8>> {
        let options = options.into_iter();
        unique_names(options.clone())?;
        let mut writer = Writer::new();
        for option in options {
            writer.c_string(
                option.name.as_bytes(),
                "compilation option name has a NUL byte",
            )?;
            writer.c_string(
                option.value.as_bytes(),
                "compilation option value has a NUL byte",
            )?;
        }
        Ok(writer.finish())
    }
}

iterable!(CompilationOptions => Entries<'a, CompilationOption<'a>>);

/// Read one compilation option.
fn read_option<'a>(reader: &mut Reader<'a>, _: i32) -> Result<CompilationOption<'a>> {
    let name = reader.c_string("compilation option name has no NUL terminator")?;
    let value = reader.c_string("compilation option value has no NUL terminator")?;
    Ok(CompilationOption {
        name: Utf8Bytes::new(name),
        value: Utf8Bytes::new(value),
    })
}

/// Check that no two options have the same name.
fn unique_names<'o>(options: impl Iterator<Item = CompilationOption<'o>>) -> Result<()> {
    let mut names = BTreeSet::new();
    for option in options {
        if !names.insert(option.name.as_bytes()) {
            return Err(invalid("two compilation options have the same name"));
        }
    }
    Ok(())
}

/// The metadata references that the compiler used for a module.
///
/// The blob is a list of [`MetadataReference`] entries to the end of the blob.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompilationMetadataReferences<'a> {
    /// The entries. [`CompilationMetadataReferences::parse`] checked them.
    bytes: &'a [u8],
    /// The number of entries.
    len: usize,
}

/// One metadata reference of a compilation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MetadataReference<'a> {
    /// The file name of the reference, with its extension.
    pub file_name: Utf8Bytes<'a>,
    /// The external aliases, separated by commas. The list can be empty.
    pub aliases: Utf8Bytes<'a>,
    /// The flags: [`MetadataReference::ASSEMBLY`] and
    /// [`MetadataReference::EMBED_INTEROP_TYPES`]. The other bits have no
    /// meaning yet; the codec keeps them.
    pub flags: u8,
    /// The `TimeDateStamp` field of the COFF header of the reference.
    pub time_stamp: u32,
    /// The `SizeOfImage` field of the PE header of the reference.
    pub file_size: u32,
    /// The module version ID (MVID) of the reference, in serialized GUID order.
    pub mvid: [u8; 16],
}

impl MetadataReference<'_> {
    /// The flag of an assembly reference. A clear flag means a module.
    pub const ASSEMBLY: u8 = 0b01;
    /// The flag of a reference that embeds its interop types.
    pub const EMBED_INTEROP_TYPES: u8 = 0b10;
}

impl<'a> CompilationMetadataReferences<'a> {
    /// Decode a compilation metadata references blob.
    ///
    /// # Errors
    ///
    /// Return [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if
    /// the blob ends inside an entry.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let len = count_to_end(blob, 0, read_reference)?;
        Ok(Self { bytes: blob, len })
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the number of references.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no references.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the references.
    #[must_use]
    pub const fn iter(&self) -> Entries<'a, MetadataReference<'a>> {
        Entries::new(self.bytes, self.len, 0, read_reference)
    }

    /// Encode a compilation metadata references blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   file name or an alias list has a NUL byte.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if an
    ///   allocation fails.
    pub fn encode<'r>(
        references: impl IntoIterator<Item = MetadataReference<'r>>,
    ) -> Result<Vec<u8>> {
        let mut writer = Writer::new();
        for reference in references {
            writer.c_string(
                reference.file_name.as_bytes(),
                "metadata reference file name has a NUL byte",
            )?;
            writer.c_string(
                reference.aliases.as_bytes(),
                "metadata reference aliases have a NUL byte",
            )?;
            writer.u8(reference.flags)?;
            writer.u32(reference.time_stamp)?;
            writer.u32(reference.file_size)?;
            writer.bytes(&reference.mvid)?;
        }
        Ok(writer.finish())
    }
}

iterable!(CompilationMetadataReferences => Entries<'a, MetadataReference<'a>>);

/// Read one metadata reference.
fn read_reference<'a>(reader: &mut Reader<'a>, _: i32) -> Result<MetadataReference<'a>> {
    const SHORT: &str = "metadata reference is truncated";
    let file_name = reader.c_string("metadata reference file name has no NUL terminator")?;
    let aliases = reader.c_string("metadata reference aliases have no NUL terminator")?;
    Ok(MetadataReference {
        file_name: Utf8Bytes::new(file_name),
        aliases: Utf8Bytes::new(aliases),
        flags: reader.u8(SHORT)?,
        time_stamp: reader.u32(SHORT)?,
        file_size: reader.u32(SHORT)?,
        mvid: reader.array(SHORT)?,
    })
}
