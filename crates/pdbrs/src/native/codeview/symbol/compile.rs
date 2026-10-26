//! Object name, compile and build environment records.
//!
//! The layouts follow `ObjNameSym`, `Compile2Sym`, `Compile3Sym` and
//! `EnvBlockSym` in LLVM `SymbolRecordMapping.cpp`.

use core::iter::FusedIterator;

use super::super::Fields;
use super::super::writer::FieldWriter;
use crate::{Error, ErrorKind, Result};

/// An object file name record (`S_OBJNAME`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ObjectName<'a> {
    /// The signature of the object file. Compilers usually write 0.
    pub signature: u32,
    /// The path of the object file.
    pub name: &'a [u8],
}

impl<'a> ObjectName<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let signature = fields.u32()?;
        let name = fields.name()?;
        Ok(Self { signature, name })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.signature)?;
        out.name(self.name)
    }
}

values! {
    /// The source language in the low byte of [`CompileFlags`] (`CV_CFL_LANG`).
    ///
    /// The values come from LLVM `CodeViewLanguages.def`. Other values stay raw.
    SourceLanguage(u8) {
        /// C.
        C = 0x00;
        /// C++.
        CPP = 0x01;
        /// Fortran.
        FORTRAN = 0x02;
        /// The Microsoft macro assembler.
        MASM = 0x03;
        /// Pascal.
        PASCAL = 0x04;
        /// Basic.
        BASIC = 0x05;
        /// COBOL.
        COBOL = 0x06;
        /// The linker.
        LINK = 0x07;
        /// The resource converter `cvtres`.
        CVTRES = 0x08;
        /// The PGO converter `cvtpgd`.
        CVTPGD = 0x09;
        /// C#.
        CSHARP = 0x0a;
        /// Visual Basic.
        VISUAL_BASIC = 0x0b;
        /// The IL assembler.
        ILASM = 0x0c;
        /// Java.
        JAVA = 0x0d;
        /// `JScript`.
        JSCRIPT = 0x0e;
        /// MSIL.
        MSIL = 0x0f;
        /// HLSL.
        HLSL = 0x10;
        /// Objective-C.
        OBJECTIVE_C = 0x11;
        /// Objective-C++.
        OBJECTIVE_CPP = 0x12;
        /// Swift.
        SWIFT = 0x13;
        /// An alias object.
        ALIAS_OBJECT = 0x14;
        /// Rust.
        RUST = 0x15;
        /// Go.
        GO = 0x16;
        /// D.
        D = b'D';
        /// An old Swift compiler.
        OLD_SWIFT = b'S';
    }
}

flags! {
    /// The flags of a compile record (`CompileSym3Flags` in LLVM).
    ///
    /// Bits 0 to 7 hold the [`SourceLanguage`]. [`S_COMPILE2`](super::S_COMPILE2)
    /// defines the bits up to [`CompileFlags::MSIL_MODULE`].
    CompileFlags(u32) {
        /// The module was compiled for edit and continue.
        EDIT_AND_CONTINUE = 1 << 8;
        /// The module was compiled without debug information.
        NO_DEBUG_INFO = 1 << 9;
        /// The module was compiled with link-time code generation.
        LTCG = 1 << 10;
        /// The module was compiled without data alignment.
        NO_DATA_ALIGN = 1 << 11;
        /// The module contains managed code.
        MANAGED_PRESENT = 1 << 12;
        /// The module was compiled with security checks.
        SECURITY_CHECKS = 1 << 13;
        /// The module was compiled for hot patching.
        HOT_PATCH = 1 << 14;
        /// The module was converted with `CVTCIL`.
        CVTCIL = 1 << 15;
        /// The module is an MSIL module.
        MSIL_MODULE = 1 << 16;
        /// The module was compiled with SDL checks.
        SDL = 1 << 17;
        /// The module was compiled with profile-guided optimization.
        PGO = 1 << 18;
        /// The module is an export module (`.exp`).
        EXP = 1 << 19;
    }
}

impl CompileFlags {
    /// Return the source language in bits 0 to 7.
    #[must_use]
    pub const fn language(self) -> SourceLanguage {
        SourceLanguage(self.0.to_le_bytes()[0])
    }
}

/// A compile record, version 2 (`S_COMPILE2`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Compile2<'a> {
    /// The flags and the source language.
    pub flags: CompileFlags,
    /// The target CPU (`CV_CPU_TYPE_e`), for example `0xd0` for x64.
    pub machine: u16,
    /// The front-end version: major, minor and build.
    pub frontend: [u16; 3],
    /// The back-end version: major, minor and build.
    pub backend: [u16; 3],
    /// The compiler version string.
    pub version: &'a [u8],
    /// The optional block of extra strings after the version string.
    pub extra_strings: StringList<'a>,
}

impl<'a> Compile2<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let flags = CompileFlags(fields.u32()?);
        let machine = fields.u16()?;
        let frontend = [fields.u16()?, fields.u16()?, fields.u16()?];
        let backend = [fields.u16()?, fields.u16()?, fields.u16()?];
        let version = fields.name()?;
        let extra_strings = StringList::read(fields)?;
        Ok(Self {
            flags,
            machine,
            frontend,
            backend,
            version,
            extra_strings,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.flags.0)?;
        out.u16(self.machine)?;
        for value in self.frontend.into_iter().chain(self.backend) {
            out.u16(value)?;
        }
        out.name(self.version)?;
        self.extra_strings.write(out)
    }
}

/// A compile record, version 3 (`S_COMPILE3`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Compile3<'a> {
    /// The flags and the source language.
    pub flags: CompileFlags,
    /// The target CPU (`CV_CPU_TYPE_e`), for example `0xd0` for x64.
    pub machine: u16,
    /// The front-end version: major, minor, build and QFE.
    pub frontend: [u16; 4],
    /// The back-end version: major, minor, build and QFE.
    pub backend: [u16; 4],
    /// The compiler version string.
    pub version: &'a [u8],
    /// The number of zero bytes after the NUL terminator of the version
    /// string, with the record padding.
    ///
    /// The encoder writes these bytes and then pads the record. Most
    /// producers write only the record padding; `ml64` writes one more NUL.
    pub trailing_zeros: usize,
}

impl<'a> Compile3<'a> {
    /// Read the fields.
    ///
    /// The Microsoft macro assembler (`ml64`) writes an extra NUL byte after
    /// the version string, so the zero bytes after the string can fill four
    /// bytes or more. cvdump `C7Compile3` and LLVM `Compile3Sym` ignore the
    /// bytes after the string. This decoder accepts them only if all are zero.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let flags = CompileFlags(fields.u32()?);
        let machine = fields.u16()?;
        let frontend = [fields.u16()?, fields.u16()?, fields.u16()?, fields.u16()?];
        let backend = [fields.u16()?, fields.u16()?, fields.u16()?, fields.u16()?];
        let version = fields.name()?;
        let rest = fields.bytes(fields.rest().len())?;
        if rest.iter().any(|&byte| byte != 0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_COMPILE3 has bytes after its version string that are not zero"));
        }
        Ok(Self {
            flags,
            machine,
            frontend,
            backend,
            version,
            trailing_zeros: rest.len(),
        })
    }

    /// Write the fields and the trailing zero bytes.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.flags.0)?;
        out.u16(self.machine)?;
        for value in self.frontend.into_iter().chain(self.backend) {
            out.u16(value)?;
        }
        out.name(self.version)?;
        for _ in 0..self.trailing_zeros {
            out.u8(0)?;
        }
        Ok(())
    }
}

/// A build environment record (`S_ENVBLOCK`).
///
/// The strings are key and value pairs, for example `cwd` and a directory.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct EnvironmentBlock<'a> {
    /// The flags byte. Bit 0 is reserved (`rev`). `cvinfo.h` requires the
    /// other bits to be zero; the decoder keeps the byte raw.
    pub flags: u8,
    /// The strings.
    pub strings: StringList<'a>,
}

impl<'a> EnvironmentBlock<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let flags = fields.u8()?;
        let strings = StringList::read(fields)?;
        Ok(Self { flags, strings })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u8(self.flags)?;
        self.strings.write(out)
    }
}

/// A sequence of non-empty NUL-terminated strings.
///
/// In the record, an empty string ends the sequence. The value keeps the
/// bytes before the empty string. The sequence can also be absent: the
/// record then ends without the empty string. [`StringList::is_terminated`]
/// tells the two forms apart.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct StringList<'a> {
    /// The strings with their terminators, without the final empty string.
    bytes: &'a [u8],
    /// `true` if the record holds the empty string that ends the sequence.
    terminated: bool,
}

impl<'a> StringList<'a> {
    /// Read the strings and the empty string that ends them.
    ///
    /// If no bytes are left, the sequence is absent. This follows
    /// `mapStringZVectorZ` in LLVM and the "optional block" in `cvinfo.h`.
    fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let rest = fields.rest();
        let mut length = 0;
        loop {
            let Some(tail) = rest.get(length..).filter(|tail| !tail.is_empty()) else {
                if length == 0 {
                    return Ok(Self::default());
                }
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("symbol string list has no empty terminating string"));
            };
            let end = tail.iter().position(|&byte| byte == 0).ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("symbol string list has no NUL terminator")
            })?;
            if end == 0 {
                break;
            }
            length += end + 1;
        }
        let bytes = fields.bytes(length)?;
        // Consume the empty string that ends the sequence.
        fields.bytes(1)?;
        Ok(Self {
            bytes,
            terminated: true,
        })
    }

    /// Create a list from strings with their NUL terminators.
    ///
    /// The encoder writes the strings and the empty string that ends them.
    /// An empty `bytes` gives an absent sequence.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `bytes` does not end with a
    /// NUL byte or holds an empty string.
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        let valid = bytes.is_empty()
            || (bytes.first() != Some(&0)
                && bytes.last() == Some(&0)
                && !bytes.windows(2).any(|pair| pair == [0, 0]));
        if !valid {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("symbol string list holds an empty or unterminated string"));
        }
        Ok(Self {
            bytes,
            terminated: !bytes.is_empty(),
        })
    }

    /// Write the strings and, if the sequence is present, the empty string
    /// that ends it.
    fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.bytes(self.bytes)?;
        if self.terminated || !self.bytes.is_empty() {
            out.u8(0)?;
        }
        Ok(())
    }

    /// Return the raw bytes of the strings with their terminators.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return `true` if the record holds the empty string that ends the
    /// sequence. An absent sequence gives `false`.
    #[must_use]
    pub const fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// Return `true` if the list has no strings.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the strings, without their terminators.
    #[must_use]
    pub const fn iter(&self) -> Strings<'a> {
        Strings { bytes: self.bytes }
    }
}

impl<'a> IntoIterator for StringList<'a> {
    type Item = &'a [u8];
    type IntoIter = Strings<'a>;

    fn into_iter(self) -> Strings<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &StringList<'a> {
    type Item = &'a [u8];
    type IntoIter = Strings<'a>;

    fn into_iter(self) -> Strings<'a> {
        self.iter()
    }
}

/// An iterator over the strings of a [`StringList`].
#[derive(Clone, Debug)]
pub struct Strings<'a> {
    /// The strings that are not returned yet.
    bytes: &'a [u8],
}

impl<'a> Iterator for Strings<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        // A `StringList` ends each string with a NUL byte.
        let end = self.bytes.iter().position(|&byte| byte == 0)?;
        let (string, rest) = self.bytes.split_at(end);
        self.bytes = &rest[1..];
        Some(string)
    }
}

impl FusedIterator for Strings<'_> {}
