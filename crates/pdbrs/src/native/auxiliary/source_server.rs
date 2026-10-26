//! The `srcsrv` stream: source server data.
//!
//! A source indexing tool, for example `pdbstr -w -s:srcsrv`, stores this text stream. The
//! Debugging Tools for Windows document the format ("Language Specification 1" and
//! "Language Specification 2" of `SrcSrv`). The text has lines in sections. A marker line starts
//! each section:
//!
//! 1. `SRCSRV: ini`: lines with `KEY=value`, for example `VERSION=2`.
//! 2. `SRCSRV: variables`: lines with `KEY=value`, for example `SRCSRVTRG=...`.
//! 3. `SRCSRV: source files`: one line for each file, with fields separated by `*`. The first
//!    field is the source path in the PDB.
//! 4. `SRCSRV: end`: the end of the data.
//!
//! Characters, usually dashes, can follow the section name on a marker line. A line ends with
//! LF or CR LF. [`SourceServer`] keeps the raw bytes, so [`SourceServer::encode`] gives the same
//! bytes again. The marker comparison ignores the case of ASCII letters.
//!
//! This module only frames the text into sections and lines. It does not expand the `%...%`
//! variables and does not interpret the keys or the commands.

use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::ops::Range;

use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The UTF-8 byte order mark.
const BOM: &[u8] = b"\xef\xbb\xbf";

/// The prefix of a marker line.
const MARKER_PREFIX: &[u8] = b"SRCSRV:";

/// A section of a `srcsrv` stream.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceServerSection {
    /// The `SRCSRV: ini` section.
    Ini,
    /// The `SRCSRV: variables` section.
    Variables,
    /// The `SRCSRV: source files` section.
    SourceFiles,
    /// The `SRCSRV: end` marker.
    End,
}

impl SourceServerSection {
    /// Return the section name of the marker line.
    #[must_use]
    pub const fn name(self) -> &'static [u8] {
        match self {
            Self::Ini => b"ini",
            Self::Variables => b"variables",
            Self::SourceFiles => b"source files",
            Self::End => b"end",
        }
    }
}

/// A `KEY=value` line of the `ini` or `variables` section.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SourceServerVariable<'a> {
    /// The bytes before the first `=`.
    pub key: &'a [u8],
    /// The bytes after the first `=`, without the line end.
    pub value: &'a [u8],
}

/// A line of the `source files` section.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SourceServerFile<'a> {
    /// The line without its line end.
    pub line: &'a [u8],
}

impl<'a> SourceServerFile<'a> {
    /// Return the fields of the line: `VAR1`, `VAR2` and so on.
    #[must_use]
    pub fn fields(&self) -> SourceServerFields<'a> {
        SourceServerFields {
            rest: Some(self.line),
        }
    }

    /// Return the first field: the source path in the PDB.
    #[must_use]
    pub fn path(&self) -> &'a [u8] {
        self.fields().next().unwrap_or_default()
    }
}

/// A decoded `srcsrv` stream, borrowed from the stream bytes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceServer<'a> {
    /// The stream bytes.
    bytes: &'a [u8],
    /// The body of each section, after its marker line. The `End` slot holds the bytes after
    /// the end marker line.
    sections: [Option<Range<usize>>; 4],
}

impl<'a> SourceServer<'a> {
    /// Decode the `srcsrv` stream `stream`.
    ///
    /// The first line must be the `SRCSRV: ini` marker. A UTF-8 byte order mark can come
    /// before it. The `variables` and `source files` sections are optional, but they must be
    /// in the order above. The bytes after the `SRCSRV: end` line stay available as
    /// [`SourceServer::trailing`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] with `stream` and the offset of the line if:
    ///
    /// - The stream does not start with the `SRCSRV: ini` marker.
    /// - A marker line has an unknown section name, or a section is out of order.
    /// - A line of the `ini` or `variables` section is not blank and has no `=`, or has an
    ///   empty key.
    /// - The stream has no `SRCSRV: end` marker.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Self> {
        let invalid = |offset: usize, context| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context(context)
                .with_stream(stream, u64::try_from(offset).unwrap_or(u64::MAX))
        };
        let start = if bytes.starts_with(BOM) { BOM.len() } else { 0 };
        let mut sections: [Option<Range<usize>>; 4] = Default::default();
        let mut current: Option<SourceServerSection> = None;
        let mut offset = start;
        while offset < bytes.len() {
            let (line, next) = line_at(bytes, offset);
            match marker(line) {
                Ok(Some(section)) => {
                    let in_order = match current {
                        None => section == SourceServerSection::Ini,
                        Some(previous) => section > previous,
                    };
                    if !in_order {
                        return Err(invalid(offset, "srcsrv section is out of order"));
                    }
                    if let Some(previous) = current {
                        close(&mut sections, previous, offset);
                    }
                    sections[section as usize] = Some(next..next);
                    current = Some(section);
                    if section == SourceServerSection::End {
                        sections[section as usize] = Some(next..bytes.len());
                        return Ok(Self { bytes, sections });
                    }
                }
                Ok(None) => match current {
                    None => {
                        return Err(invalid(
                            offset,
                            "srcsrv stream does not start with SRCSRV: ini",
                        ));
                    }
                    Some(SourceServerSection::Ini | SourceServerSection::Variables)
                        if variable(line).is_none() && !is_blank(line) =>
                    {
                        return Err(invalid(offset, "srcsrv line is not KEY=value"));
                    }
                    Some(_) => {}
                },
                Err(()) => return Err(invalid(offset, "srcsrv marker has an unknown section")),
            }
            offset = next;
        }
        Err(invalid(
            bytes.len(),
            if current.is_none() {
                "srcsrv stream does not start with SRCSRV: ini"
            } else {
                "srcsrv stream has no SRCSRV: end"
            },
        ))
    }

    /// Return the stream bytes.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return `true` if the stream has the marker of `section`.
    #[must_use]
    pub const fn has_section(&self, section: SourceServerSection) -> bool {
        self.sections[section as usize].is_some()
    }

    /// Return the `KEY=value` lines of the `ini` section. Blank lines are not returned.
    #[must_use]
    pub fn ini(&self) -> SourceServerVariables<'a> {
        SourceServerVariables(self.lines(SourceServerSection::Ini))
    }

    /// Return the `KEY=value` lines of the `variables` section. Blank lines are not returned.
    #[must_use]
    pub fn variables(&self) -> SourceServerVariables<'a> {
        SourceServerVariables(self.lines(SourceServerSection::Variables))
    }

    /// Return the lines of the `source files` section. Blank lines are not returned.
    #[must_use]
    pub fn source_files(&self) -> SourceServerFiles<'a> {
        SourceServerFiles(self.lines(SourceServerSection::SourceFiles))
    }

    /// Return the bytes after the `SRCSRV: end` line.
    #[must_use]
    pub fn trailing(&self) -> &'a [u8] {
        self.sections[SourceServerSection::End as usize]
            .clone()
            .map_or(&[][..], |range| &self.bytes[range])
    }

    /// Append the stream bytes to `out`. A decoded stream gives the same bytes again.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).bytes(self.bytes)
    }

    /// Return the lines of the body of `section`, without their line ends.
    fn lines(&self, section: SourceServerSection) -> Lines<'a> {
        let body = self.sections[section as usize]
            .clone()
            .map_or(&[][..], |range| &self.bytes[range]);
        let body = body.strip_suffix(b"\n").unwrap_or(body);
        Lines {
            rest: (!body.is_empty()).then_some(body),
        }
    }
}

/// Set the end of the body of `section` to `end`.
fn close(sections: &mut [Option<Range<usize>>; 4], section: SourceServerSection, end: usize) {
    if let Some(range) = &mut sections[section as usize] {
        range.end = end;
    }
}

/// Return the line at `offset` without its line end, and the offset of the next line.
fn line_at(bytes: &[u8], offset: usize) -> (&[u8], usize) {
    let rest = &bytes[offset..];
    match rest.iter().position(|&byte| byte == b'\n') {
        Some(at) => {
            let line = &rest[..at];
            (line.strip_suffix(b"\r").unwrap_or(line), offset + at + 1)
        }
        None => (rest, bytes.len()),
    }
}

/// Return the section of a marker line, `Ok(None)` for another line, or `Err(())` for a
/// marker line with an unknown section name.
fn marker(line: &[u8]) -> core::result::Result<Option<SourceServerSection>, ()> {
    let Some(prefix) = line.get(..MARKER_PREFIX.len()) else {
        return Ok(None);
    };
    if !prefix.eq_ignore_ascii_case(MARKER_PREFIX) {
        return Ok(None);
    }
    let rest = line[MARKER_PREFIX.len()..].trim_ascii_start();
    for section in [
        SourceServerSection::Ini,
        SourceServerSection::Variables,
        SourceServerSection::SourceFiles,
        SourceServerSection::End,
    ] {
        let name = section.name();
        let Some(head) = rest.get(..name.len()) else {
            continue;
        };
        let ends = rest
            .get(name.len())
            .is_none_or(|byte| !byte.is_ascii_alphanumeric());
        if head.eq_ignore_ascii_case(name) && ends {
            return Ok(Some(section));
        }
    }
    Err(())
}

/// Split a `KEY=value` line. The key must not be empty.
fn variable(line: &[u8]) -> Option<SourceServerVariable<'_>> {
    let at = line.iter().position(|&byte| byte == b'=')?;
    (at > 0).then(|| SourceServerVariable {
        key: &line[..at],
        value: &line[at + 1..],
    })
}

/// Return `true` if the line holds only ASCII white space.
fn is_blank(line: &[u8]) -> bool {
    line.iter().all(u8::is_ascii_whitespace)
}

/// Split `rest` at the first `separator`. Return the part before it and the rest after it, or
/// `None` as the rest after the last part.
fn split_once(rest: &[u8], separator: u8) -> (&[u8], Option<&[u8]>) {
    match rest.iter().position(|&byte| byte == separator) {
        Some(end) => (&rest[..end], Some(&rest[end + 1..])),
        None => (rest, None),
    }
}

/// An iterator over the lines of a section body, without their line ends.
#[derive(Clone, Debug)]
struct Lines<'a> {
    /// The lines that are not returned yet, or `None` after the last line.
    rest: Option<&'a [u8]>,
}

impl<'a> Iterator for Lines<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let (line, rest) = split_once(self.rest?, b'\n');
        self.rest = rest;
        Some(line.strip_suffix(b"\r").unwrap_or(line))
    }
}

impl FusedIterator for Lines<'_> {}

/// An iterator over the `KEY=value` lines of a section of a [`SourceServer`].
#[derive(Clone, Debug)]
pub struct SourceServerVariables<'a>(Lines<'a>);

impl<'a> Iterator for SourceServerVariables<'a> {
    type Item = SourceServerVariable<'a>;

    fn next(&mut self) -> Option<SourceServerVariable<'a>> {
        self.0.by_ref().find_map(variable)
    }
}

impl FusedIterator for SourceServerVariables<'_> {}

/// An iterator over the lines of the `source files` section of a [`SourceServer`].
#[derive(Clone, Debug)]
pub struct SourceServerFiles<'a>(Lines<'a>);

impl<'a> Iterator for SourceServerFiles<'a> {
    type Item = SourceServerFile<'a>;

    fn next(&mut self) -> Option<SourceServerFile<'a>> {
        let line = self.0.by_ref().find(|line| !is_blank(line))?;
        Some(SourceServerFile { line })
    }
}

impl FusedIterator for SourceServerFiles<'_> {}

/// An iterator over the fields of a [`SourceServerFile`] line.
#[derive(Clone, Debug)]
pub struct SourceServerFields<'a> {
    /// The fields that are not returned yet, or `None` after the last field.
    rest: Option<&'a [u8]>,
}

impl<'a> Iterator for SourceServerFields<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let (field, rest) = split_once(self.rest?, b'*');
        self.rest = rest;
        Some(field)
    }
}

impl FusedIterator for SourceServerFields<'_> {}
