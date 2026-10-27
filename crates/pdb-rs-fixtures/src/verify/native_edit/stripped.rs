//! The comparison of a PDB that `EditOptions::with_strip_private` stripped with the PDB that
//! `link /PDBSTRIPPED` wrote for the same link (`msvc-x64-stripped`).
//!
//! The comparison uses `llvm-pdbutil` sections. The two PDBs have different stream IDs, because
//! the editor keeps the source stream IDs, and a different symbol record order, because
//! `link.exe` writes the stripped symbol records in its own order. So the comparison removes the
//! stream numbers and compares the symbol records of the indexes as sets without offsets.

use std::path::Path;

use super::super::native::consumers::normalize_pdbutil;
use super::Tools;
use super::checks::pdbutil_text;

/// The `llvm-pdbutil dump` sections that must be equal.
const SAME: [&str; 9] = [
    "--symbols",
    "--section-contribs",
    "--section-map",
    "--types",
    "--ids",
    "--files",
    "--section-headers",
    "--fpo",
    "--type-stats",
];

/// The result of the comparison.
#[derive(Debug, Default)]
pub(crate) struct Comparison {
    /// The sections that are equal.
    pub same: Vec<String>,
    /// The explained differences.
    pub explained: Vec<String>,
    /// The differences without an explanation.
    pub unexplained: Vec<String>,
}

/// Remove the lines that `skip` selects and the stream numbers.
fn without(text: &str, skip: impl Fn(&str) -> bool) -> String {
    normalize_pdbutil(text)
        .lines()
        .filter(|line| !skip(line.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Return the symbol records of a `--globals` or `--publics` dump without their offsets, with
/// their attribute lines, sorted.
fn record_set(text: &str) -> Vec<String> {
    let mut records: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some((offset, rest)) = line.split_once(" | ")
            && offset.chars().all(|c| c.is_ascii_digit())
            && rest.starts_with("S_")
        {
            records.push(rest.to_owned());
        } else if let Some(last) = records.last_mut()
            && !line.is_empty()
        {
            last.push_str(" ## ");
            last.push_str(line);
        }
    }
    records.sort();
    records
}

/// Compare the stripped PDB `ours` with the `link.exe` output `theirs`.
pub(crate) fn compare(tools: &Tools, ours: &Path, theirs: &Path) -> Result<Comparison, String> {
    let mut result = Comparison::default();
    let mut check = |name: &str, left: String, right: String, explanation: Option<&str>| {
        if left == right {
            result.same.push(name.to_owned());
            if let Some(explanation) = explanation {
                result.explained.push(format!("{name}: {explanation}"));
            }
        } else {
            result.unexplained.push(format!("{name} differs"));
        }
    };
    for section in SAME {
        let left = without(&pdbutil_text(tools, section, ours)?, |_| false);
        let right = without(&pdbutil_text(tools, section, theirs)?, |_| false);
        check(section, left, right, None);
    }
    let summary = |path| -> Result<String, String> {
        Ok(without(&pdbutil_text(tools, "--summary", path)?, |line| {
            line.starts_with("Number of streams:")
        }))
    };
    check(
        "--summary",
        summary(ours)?,
        summary(theirs)?,
        Some("equal without the stream count: the editor keeps the source stream IDs"),
    );
    let named = |path| -> Result<String, String> {
        Ok(without(
            &pdbutil_text(tools, "--named-streams", path)?,
            |line| line.starts_with("Index:"),
        ))
    };
    check(
        "--named-streams",
        named(ours)?,
        named(theirs)?,
        Some("equal without the stream indices"),
    );
    let modules = |path| -> Result<String, String> {
        Ok(without(&pdbutil_text(tools, "--modules", path)?, |_| false))
    };
    check(
        "--modules",
        modules(ours)?,
        modules(theirs)?,
        Some("equal without the module stream numbers"),
    );
    for section in ["--globals", "--publics"] {
        let left = record_set(&pdbutil_text(tools, section, ours)?).join("\n");
        let right = record_set(&pdbutil_text(tools, section, theirs)?).join("\n");
        check(
            section,
            left,
            right,
            Some(
                "the same records; link.exe writes them in another order, so the symbol \
                 record offsets differ",
            ),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::record_set;

    #[test]
    fn record_sets_ignore_the_offsets_and_the_order() {
        let left = "  260 | S_LPROCREF [size = 52] `b`\n        module = 2, offset = 4\n\
                    \n   12 | S_PUB32 [size = 20] `a`\n";
        let right = "   8 | S_PUB32 [size = 20] `a`\n  999 | S_LPROCREF [size = 52] `b`\n\
                     module = 2, offset = 4\n";
        assert_eq!(record_set(left), record_set(right));
        assert_eq!(record_set(left).len(), 2);
    }
}
