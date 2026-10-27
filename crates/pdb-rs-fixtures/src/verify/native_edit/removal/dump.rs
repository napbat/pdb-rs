//! The text of `llvm-pdbutil dump --types --ids --symbols --globals` after a removal.
//!
//! [`map_dump`] changes the dump of the source PDB to the dump that the output PDB must give.
//! A removal compacts one index space ([`Space`]):
//!
//! - In the section of the space (TPI or IPI), it removes the block of each removed record,
//!   gives each record index its output index and changes the record count. The index column
//!   gets the width of the new record count.
//! - In all sections, it changes each field of the space: see [`is_field`].
//!
//! [`reference_tokens`] gives the hexadecimal values of the IPI and symbol sections: the
//! over-approximation of the references in the IPI stream and in the symbols.

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The first index of a record.
const FIRST_RECORD: u32 = 0x1000;

/// The index space that a removal compacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Space {
    /// The type indices of the TPI stream.
    Types,
    /// The ID indices of the IPI stream.
    Ids,
}

/// One removal: the space, the removed indices and the output index of each index that is
/// not removed (`None` for another index).
pub(super) struct Removal<'r> {
    /// The compacted space.
    pub space: Space,
    /// The removed indices.
    pub removed: &'r BTreeSet<u32>,
    /// The output index of an index.
    pub map: &'r dyn Fn(u32) -> Option<u32>,
}

/// The section of a dump line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    /// The lines before the first known section title.
    Other,
    /// `Types (TPI Stream)`.
    Types,
    /// `Types (IPI Stream)`.
    Ids,
    /// `Symbols` or `Global Symbols`.
    Symbols,
}

/// Return the section of a title line, or `None` for another line.
fn title(line: &str) -> Option<Section> {
    match line.trim() {
        "Types (TPI Stream)" => Some(Section::Types),
        "Types (IPI Stream)" => Some(Section::Ids),
        "Symbols" | "Global Symbols" => Some(Section::Symbols),
        _ => None,
    }
}

/// One hexadecimal token `0x[0-9A-F]+` of a line, with no letter, digit or `_` next to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Token {
    /// The byte position of `0x`.
    start: usize,
    /// The byte position after the last digit.
    end: usize,
    /// The value.
    value: u32,
}

/// Return the tokens of `line`. A value that does not fit in 32 bits is not a token.
fn tokens(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut found = Vec::new();
    let mut position = 0;
    while let Some(offset) = line[position..].find("0x") {
        let start = position + offset;
        let digits = bytes[start + 2..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte))
            .count();
        let end = start + 2 + digits;
        position = start + 2;
        let before = start.checked_sub(1).map(|index| bytes[index]);
        if digits == 0 || before.is_some_and(word) || bytes.get(end).copied().is_some_and(word) {
            continue;
        }
        if let Ok(value) = u32::from_str_radix(&line[start + 2..end], 16) {
            found.push(Token { start, end, value });
        }
    }
    found
}

/// Return the record kind of a record header line (`0x1001 | LF_FUNC_ID ...` or
/// `204 | S_GPROC32 ...`) and the byte position of ` | `.
fn header(line: &str) -> Option<(&str, usize)> {
    let (left, right) = line.split_once(" | ")?;
    let index = left.trim_start();
    let number = index
        .strip_prefix("0x")
        .unwrap_or(index)
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit());
    let kind = right.split([' ', ']']).next()?;
    (number && !index.is_empty() && (kind.starts_with("LF_") || kind.starts_with("S_")))
        .then_some((kind, left.len()))
}

/// Return `true` if a token after `before` in an IPI record of `kind` is a type index.
fn id_type_field(kind: &str, before: &str) -> bool {
    match kind {
        "LF_FUNC_ID" | "LF_MFUNC_ID" => before.ends_with("type = "),
        "LF_UDT_SRC_LINE" | "LF_UDT_MOD_SRC_LINE" => before.ends_with("udt = "),
        _ => false,
    }
}

/// Return `true` if a token after `before` in a symbol record of `kind` is a type index.
fn symbol_type_field(kind: &str, before: &str) -> bool {
    !kind.ends_with("_ID")
        && (before.ends_with("type = ")
            || before.ends_with("type = `")
            || before.ends_with("type="))
}

/// Return `true` if a token after `before` in a symbol record of `kind` is an ID index: the
/// `type` of an `_ID` procedure, `inlinee`, `BuildId` and the callee, caller and inlinee lists.
fn symbol_id_field(kind: &str, before: &str) -> bool {
    (kind.ends_with("_ID") && (before.ends_with("type = `") || before.ends_with("type = ")))
        || [
            "inlinee = ",
            "inlinee: ",
            "callee: ",
            "caller: ",
            "BuildId = `",
        ]
        .iter()
        .any(|label| before.ends_with(label))
}

/// Return `true` if a token after `before` in a record of `kind` in `section` is an index of
/// `space`.
///
/// - Types: each value of the TPI section, the type fields of `LF_FUNC_ID`, `LF_MFUNC_ID`,
///   `LF_UDT_SRC_LINE` and `LF_UDT_MOD_SRC_LINE`, and the `type` fields of the symbols that
///   are not `_ID` procedures.
/// - IDs: each other value of the IPI section, apart from the text of a string, and the ID
///   fields of the symbols (see [`symbol_id_field`]).
fn is_field(space: Space, section: Section, kind: &str, before: &str) -> bool {
    match (space, section) {
        (_, Section::Other) | (Space::Ids, Section::Types) => false,
        (Space::Types, Section::Types) => true,
        (Space::Types, Section::Ids) => id_type_field(kind, before),
        (Space::Types, Section::Symbols) => symbol_type_field(kind, before),
        (Space::Ids, Section::Ids) => {
            !id_type_field(kind, before) && !before.contains("String: ") && !before.contains('`')
        }
        (Space::Ids, Section::Symbols) => symbol_id_field(kind, before),
    }
}

/// Return the text of the index `value` in the form of `llvm-pdbutil`.
fn index_text(value: u32) -> String {
    format!("0x{value:04X}")
}

/// Replace each token of `line` at or after `from` that `select` accepts, and whose value is a
/// record index, with its output index. `select` gets the text before the token.
fn map_tokens(
    line: &str,
    from: usize,
    select: impl Fn(&str) -> bool,
    map: &dyn Fn(u32) -> Option<u32>,
) -> Result<String, String> {
    let mut mapped = String::with_capacity(line.len());
    let mut copied = 0;
    for token in tokens(line) {
        if token.start < from || token.value < FIRST_RECORD || !select(&line[..token.start]) {
            continue;
        }
        let output = map(token.value).ok_or_else(|| {
            format!(
                "the dump names the removed or unknown index {}: {}",
                index_text(token.value),
                line.trim()
            )
        })?;
        mapped.push_str(&line[copied..token.start]);
        mapped.push_str(&index_text(output));
        copied = token.end;
    }
    mapped.push_str(&line[copied..]);
    Ok(mapped)
}

/// Return the width of the index column of a record section with `count` records: the decimal
/// digits of the end index and 2 (the `llvm-pdbutil` rule).
fn index_width(count: usize) -> usize {
    let end = u64::try_from(count).map_or(u64::MAX, |count| u64::from(FIRST_RECORD) + count);
    end.to_string().len() + 2
}

/// Return `count` with a `,` between each group of three digits.
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// The state of the compacted section (TPI or IPI) of [`map_dump`].
struct Compacted {
    /// The section.
    section: Section,
    /// The indentation of a continuation line in the source dump.
    old_indent: usize,
    /// The width of the index column in the output dump.
    new_width: usize,
    /// `true` while the lines belong to a removed record.
    skipping: bool,
    /// The kind of the current record.
    kind: String,
    /// The entry lines of the current list (`LF_ARGLIST`, `LF_BUILDINFO`, `LF_SUBSTR_LIST`)
    /// after the mapping: the value of the index, the text before `: ` and the text from `: `.
    entries: Vec<(u32, String, String)>,
}

impl Compacted {
    /// Read the `Showing N records` line. Return the state and the line for the output.
    fn start(section: Section, line: &str, removed: usize) -> Result<(Self, String), String> {
        let count = line
            .trim()
            .strip_prefix("Showing ")
            .and_then(|rest| rest.strip_suffix(" records"))
            .and_then(|count| count.replace(',', "").parse::<usize>().ok())
            .ok_or_else(|| format!("the section has no record count: {line}"))?;
        let remaining = count
            .checked_sub(removed)
            .ok_or("the section has fewer records than the removal")?;
        let state = Self {
            section,
            old_indent: 2 + index_width(count) + 3,
            new_width: index_width(remaining),
            skipping: false,
            kind: String::new(),
            entries: Vec::new(),
        };
        let indent = line.len() - line.trim_start().len();
        let text = format!("{}Showing {} records", &line[..indent], grouped(remaining));
        Ok((state, text))
    }

    /// Write the buffered list entries to `out`. `llvm-pdbutil` aligns the indices of a list
    /// to the decimal digits of the largest index and 2.
    fn flush(&mut self, out: &mut String) {
        let Some(largest) = self.entries.iter().map(|(value, ..)| *value).max() else {
            return;
        };
        let width = largest.to_string().len() + 2;
        let indent = " ".repeat(2 + self.new_width + 3);
        for (_, head, tail) in self.entries.drain(..) {
            let _ = writeln!(out, "{indent}{head:>width$}{tail}");
        }
    }

    /// Map one line after the record count and write the result to `out`. Write nothing for a
    /// line of a removed record.
    fn line(&mut self, line: &str, removal: &Removal<'_>, out: &mut String) -> Result<(), String> {
        let map = removal.map;
        let (space, section) = (removal.space, self.section);
        let indent = " ".repeat(self.old_indent);
        let continuation = line.strip_prefix(&indent);
        let entry = continuation.and_then(|rest| {
            let rest = rest.trim_start();
            let token = tokens(rest)
                .into_iter()
                .next()
                .filter(|token| token.start == 0)?;
            let colon = rest.find(": `")?;
            Some((token.value, rest, colon))
        });
        if entry.is_none() || self.skipping {
            self.flush(out);
        }
        if let Some((kind, bar)) = header(line) {
            let index = tokens(&line[..bar])
                .first()
                .map(|token| token.value)
                .ok_or_else(|| format!("the record header has no index: {line}"))?;
            kind.clone_into(&mut self.kind);
            self.skipping = removal.removed.contains(&index);
            if self.skipping {
                return Ok(());
            }
            let output = map(index)
                .ok_or_else(|| format!("the record {} has no output index", index_text(index)))?;
            let select = |before: &str| is_field(space, section, &self.kind, before);
            let rest = map_tokens(&line[bar..], 0, select, map)?;
            let width = self.new_width;
            let _ = writeln!(out, "  {:>width$}{rest}", index_text(output));
            return Ok(());
        }
        let Some(rest) = continuation else {
            self.skipping = false;
            out.push_str(line);
            out.push('\n');
            return Ok(());
        };
        if self.skipping {
            return Ok(());
        }
        let select = |before: &str| is_field(space, section, &self.kind, before);
        if let Some((value, text, colon)) = entry {
            let head = map_tokens(&text[..colon], 0, select, map)?;
            let value = if value < FIRST_RECORD {
                value
            } else {
                map(value).unwrap_or(value)
            };
            self.entries.push((value, head, text[colon..].to_owned()));
            return Ok(());
        }
        let rest = map_tokens(rest, 0, select, map)?;
        out.push_str(&" ".repeat(2 + self.new_width + 3));
        out.push_str(&rest);
        out.push('\n');
        Ok(())
    }
}

/// Map the dump `text` of the source PDB to the dump of the output PDB after `removal`.
///
/// # Errors
///
/// Returns an error if the text names a removed or unknown index in a mapped field, or if the
/// compacted section has no record count.
pub(super) fn map_dump(text: &str, removal: &Removal<'_>) -> Result<String, String> {
    let compacted = match removal.space {
        Space::Types => Section::Types,
        Space::Ids => Section::Ids,
    };
    let mut mapped = String::with_capacity(text.len());
    let mut section = Section::Other;
    let mut state: Option<Compacted> = None;
    let mut kind = String::new();
    for line in text.lines() {
        if let Some(next) = title(line) {
            if let Some(state) = &mut state {
                state.flush(&mut mapped);
            }
            section = next;
            state = None;
            kind.clear();
            mapped.push_str(line);
            mapped.push('\n');
            continue;
        }
        if section == compacted {
            if let Some(state) = &mut state {
                state.line(line, removal, &mut mapped)?;
                continue;
            }
            if line.trim_start().starts_with("Showing ") {
                let (started, line) = Compacted::start(section, line, removal.removed.len())?;
                state = Some(started);
                mapped.push_str(&line);
                mapped.push('\n');
                continue;
            }
        }
        let mut from = 0;
        if let Some((found, bar)) = header(line) {
            found.clone_into(&mut kind);
            from = bar;
        }
        let select = |before: &str| is_field(removal.space, section, &kind, before);
        mapped.push_str(&map_tokens(line, from, select, removal.map)?);
        mapped.push('\n');
    }
    if let Some(state) = &mut state {
        state.flush(&mut mapped);
    }
    Ok(mapped)
}

/// The values of the IPI and symbol sections of a dump.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReferenceTokens {
    /// Each value of at least `0x1000`, apart from the record header indices.
    pub all: BTreeSet<u32>,
    /// The values of `all` that are not in a type field (see [`is_field`]).
    pub ids: BTreeSet<u32>,
}

/// Return the values of the IPI and symbol sections of `text`.
pub(super) fn reference_tokens(text: &str) -> ReferenceTokens {
    let mut found = ReferenceTokens::default();
    let mut section = Section::Other;
    let mut kind = String::new();
    for line in text.lines() {
        if let Some(next) = title(line) {
            section = next;
            continue;
        }
        if !matches!(section, Section::Ids | Section::Symbols) {
            continue;
        }
        let mut from = 0;
        if let Some((found, bar)) = header(line) {
            found.clone_into(&mut kind);
            from = bar;
        }
        for token in tokens(line) {
            if token.start < from || token.value < FIRST_RECORD {
                continue;
            }
            found.all.insert(token.value);
            if !is_field(Space::Types, section, &kind, &line[..token.start]) {
                found.ids.insert(token.value);
            }
        }
    }
    found
}

/// Return the first different line of `expected` and `found`, with its 1-based number, or
/// `None` if the lines are equal.
pub(super) fn first_difference(expected: &str, found: &str) -> Option<String> {
    let mut left = expected.lines();
    let mut right = found.lines();
    let mut number = 1;
    loop {
        match (left.next(), right.next()) {
            (None, None) => return None,
            (a, b) if a == b => number += 1,
            (a, b) => {
                let end = "(end of the text)";
                return Some(format!(
                    "line {number}: expected {:?}, found {:?}",
                    a.unwrap_or(end),
                    b.unwrap_or(end)
                ));
            }
        }
    }
}
