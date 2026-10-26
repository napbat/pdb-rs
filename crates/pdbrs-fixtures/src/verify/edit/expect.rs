//! The decoded sections of a verifier dump and the expected change of an edit.

use pdbrs::portable::metadata::{CodedIndex, MetadataToken};

/// The keywords of the lines that show the physical layout, not decoded records.
const LAYOUT: [&str; 10] = [
    "portable-dump",
    "metadata",
    "stream",
    "heap",
    "pdb",
    "pdb-rows",
    "tables",
    "index",
    "table",
    "row",
];

/// The edit that the expected dump shows.
pub(super) struct Edit {
    /// The suffix of the name of document 1.
    pub(super) rename_suffix: String,
    /// The number of lines that each visible sequence point moves.
    pub(super) line_shift: u32,
    /// The name and value fields of the constant that the edit adds to the last scope.
    pub(super) constant: String,
    /// The lines of the added custom debug information, without row IDs.
    pub(super) custom: Vec<String>,
    /// The parent token of the added custom debug information.
    pub(super) custom_parent: u32,
}

/// Return the coded `HasCustomDebugInformation` value of a parent token in hexadecimal text.
fn coded(token: &str) -> Option<u32> {
    let token = u32::from_str_radix(token.strip_prefix("0x")?, 16).ok()?;
    CodedIndex::HasCustomDebugInformation.encode(MetadataToken::from_raw(token))
}

/// Give the `cdi` field of each `source` line the row ID after the added custom debug
/// information. The table is sorted by the coded parent, and the added row comes after the
/// rows with the same parent.
fn shift_sources(lines: &mut [String], parent: u32) -> Result<(), String> {
    let parent = CodedIndex::HasCustomDebugInformation
        .encode(MetadataToken::from_raw(parent))
        .ok_or_else(|| "the added parent is not a coded parent".to_owned())?;
    let before = lines
        .iter()
        .filter(|line| keyword(line) == "cdi")
        .filter(|line| {
            field(line, "parent")
                .and_then(coded)
                .is_some_and(|row| row <= parent)
        })
        .count();
    for line in lines.iter_mut().filter(|line| keyword(line) == "source") {
        let Some(row) = field(line, "cdi").and_then(|row| row.parse::<usize>().ok()) else {
            continue;
        };
        if row > before {
            *line = line.replace(&format!(" cdi={row} "), &format!(" cdi={} ", row + 1));
        }
    }
    Ok(())
}

/// Return the keyword of a line.
fn keyword(line: &str) -> &str {
    line.split(' ').next().unwrap_or("")
}

/// Return the value of the field `name` of a line.
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(name)?.strip_prefix('='))
}

/// Return the lines of a dump that show decoded records.
pub(super) fn decoded(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !LAYOUT.contains(&keyword(line)))
        .map(str::to_owned)
        .collect()
}

/// Return the lines without the custom debug information lines.
pub(super) fn without_custom(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| !keyword(line).starts_with("cdi"))
        .cloned()
        .collect()
}

/// Return each custom debug information value (its `cdi` line and the detail lines) as one
/// line without row IDs, sorted.
pub(super) fn custom_blocks(lines: &[String]) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    for line in lines.iter().filter(|line| keyword(line).starts_with("cdi")) {
        let text: Vec<&str> = line
            .split(' ')
            .enumerate()
            .filter(|&(index, part)| !(index == 1 && part.starts_with("id=")))
            .map(|(_, part)| part)
            .collect();
        let text = text.join(" ");
        match blocks.last_mut() {
            Some(block) if keyword(line) != "cdi" => {
                block.push_str(" | ");
                block.push_str(&text);
            }
            _ => blocks.push(text),
        }
    }
    blocks.sort();
    blocks
}

/// Add `shift` to the `start-line` and `end-line` fields of a sequence point line.
fn shift_lines(line: &str, shift: u32) -> Result<String, String> {
    let parts: Result<Vec<String>, String> = line
        .split(' ')
        .map(|part| {
            for name in ["start-line=", "end-line="] {
                if let Some(value) = part.strip_prefix(name) {
                    let value: u32 = value
                        .parse()
                        .map_err(|error| format!("bad line number in `{line}`: {error}"))?;
                    return Ok(format!("{name}{}", value + shift));
                }
            }
            Ok(part.to_owned())
        })
        .collect();
    Ok(parts?.join(" "))
}

/// Return the expected decoded lines after the edit.
pub(super) fn edited(lines: &[String], edit: &Edit) -> Result<Vec<String>, String> {
    let mut result = Vec::with_capacity(lines.len() + 4);
    for line in lines {
        if line.starts_with("document id=1 ") {
            let start = line
                .find(" name=\"")
                .ok_or_else(|| format!("no name in `{line}`"))?
                + " name=\"".len();
            let end = start
                + line[start..]
                    .find('"')
                    .ok_or_else(|| format!("no end of the name in `{line}`"))?;
            result.push(format!(
                "{}{}{}",
                &line[..end],
                edit.rename_suffix,
                &line[end..]
            ));
        } else if line.starts_with("sequence-point ") && line.contains(" start-line=") {
            result.push(shift_lines(line, edit.line_shift)?);
        } else {
            result.push(line.clone());
        }
    }
    if let Some(index) = result.iter().rposition(|line| keyword(line) == "scope") {
        let id = field(&result[index], "id")
            .ok_or_else(|| "a scope line has no id".to_owned())?
            .to_owned();
        let constants = result
            .iter()
            .filter(|line| keyword(line) == "scope-constant")
            .count();
        let count: usize = field(&result[index], "constants")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| "a scope line has no constant count".to_owned())?;
        result[index] = result[index].replace(
            &format!(" constants={count}"),
            &format!(" constants={}", count + 1),
        );
        let members = [
            format!("scope-variable scope={id} "),
            format!("scope-constant scope={id} "),
        ];
        let after = result[index + 1..]
            .iter()
            .take_while(|line| {
                members
                    .iter()
                    .any(|member| line.starts_with(member.as_str()))
            })
            .count();
        result.insert(
            index + 1 + after,
            format!(
                "scope-constant scope={id} id={} {}",
                constants + 1,
                edit.constant
            ),
        );
    }
    shift_sources(&mut result, edit.custom_parent)?;
    result.extend(edit.custom.iter().cloned());
    Ok(result)
}
