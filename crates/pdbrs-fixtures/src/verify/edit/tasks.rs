//! The task cases of `verify-edit`: the editing tasks of `pdbrs::portable::PdbEditor`.
//!
//! Each case edits the source with `IdentityPolicy::ContentDerived`, writes
//! `<fixture>.task-<case>.pdb` and its verifier dump `<fixture>.task-<case>.srm.txt`, and
//! checks it as the change and removal cases do (see [`super::cases`]):
//!
//! 1. The decoded sections of the verifier dump are the expected change of the source
//!    sections: the records without custom debug information line for line, and the custom
//!    debug information blocks sorted and without row IDs. The verifier decodes the Source
//!    Link text and the embedded sources, and checks each embedded source against the
//!    SHA-256 hash of its document (`check=verified`).
//! 2. The streams whose bytes are different from the source are exactly the streams that the
//!    plan report does not give as [`StreamChange::Copied`].
//! 3. The same edit on a second editor gives the same bytes.
//!
//! A case that does not apply gives a `note:` line and no output.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pdbrs::io::ReadAt;
use pdbrs::portable::custom::{CustomDebugKind, CustomDebugParent};
use pdbrs::portable::debug::HashAlgorithm;
use pdbrs::portable::embedded::{EmbeddedFormat, EmbeddedSource};
use pdbrs::portable::metadata::DocumentId;
use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor, StreamChange};

use super::cases::{changed_streams, list};
use super::expect::{self, coded, field, keyword};
use super::{EMBEDDED_LIMIT, pdbrs, same, source, write_and_dump};
use crate::jobs::Jobs;
use crate::verify::Comparison;
use crate::verify::rebuild::yes;

/// The Source Link file of the `source-link` case.
const SOURCE_LINK: &[u8] = br#"{"documents":{"C:\\pdbrs\\*":"https://example.invalid/pdbrs/*"}}"#;
/// The kind GUID of Source Link in the text of the verifier.
const SOURCE_LINK_KIND: &str = "cc110556-a091-4d38-9fec-25ab9a351a6a";
/// The kind GUID of embedded source in the text of the verifier.
const EMBEDDED_SOURCE_KIND: &str = "0e8a571b-6926-466e-b4ad-8ab04611f5fe";
/// The table number of `Document` in a metadata token.
const DOCUMENT_TABLE: u32 = 0x30;

/// The task cases in log order.
const TASKS: [Task; 3] = [
    Task::SourceLink,
    Task::EmbedSource,
    Task::ReplaceEmbeddedSource,
];

/// One task case.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Task {
    /// `set_source_link`: the Source Link record of the module gets a new JSON text.
    SourceLink,
    /// `embed_source` (DEFLATE): the first document without embedded source whose file below
    /// `fixtures/dotnet/` has the SHA-256 hash of the document.
    EmbedSource,
    /// `embed_source`: the first document with embedded source gets its decoded source again,
    /// in the other form (raw for DEFLATE, DEFLATE for raw).
    ReplaceEmbeddedSource,
}

impl Task {
    /// Return the name of the case in the log. The output files are `<fixture>.task-<name>`.
    const fn name(self) -> &'static str {
        match self {
            Self::SourceLink => "source-link",
            Self::EmbedSource => "embed-source",
            Self::ReplaceEmbeddedSource => "replace-embedded-source",
        }
    }
}

/// The change of an applied case, for the expected dump.
enum Change {
    /// The new Source Link text.
    SourceLink,
    /// A new or a replaced embedded source of a document.
    Embedded {
        /// The document.
        document: DocumentId,
        /// The source bytes.
        source: Vec<u8>,
        /// The storage format.
        format: EmbeddedFormat,
    },
}

/// The output of an applied case.
struct Edited {
    /// The output bytes.
    bytes: Vec<u8>,
    /// The names of the streams that the plan report does not give as copied.
    reported: BTreeSet<String>,
    /// The change.
    change: Change,
}

/// Return the file below `dotnet` of a document name below a `fixtures\dotnet\` directory.
fn document_file(dotnet: &Path, name: &[u8]) -> Option<PathBuf> {
    let name = std::str::from_utf8(name).ok()?;
    let marker = "\\fixtures\\dotnet\\";
    let start = name.rfind(marker)? + marker.len();
    Some(
        name[start..]
            .split('\\')
            .fold(dotnet.to_path_buf(), |path, part| path.join(part)),
    )
}

/// Return the documents of the editor that have an embedded source.
fn embedded<S: ReadAt>(editor: &mut PdbEditor<'_, S>) -> pdbrs::Result<Vec<(DocumentId, Vec<u8>)>> {
    let mut found = Vec::new();
    for (_, information) in editor.custom_debug_information_mut()?.iter() {
        if let CustomDebugParent::Document(id) = information.parent
            && information.kind == CustomDebugKind::EMBEDDED_SOURCE
        {
            found.push((id, information.value.to_vec()));
        }
    }
    found.sort_by_key(|(id, _)| id.0);
    Ok(found)
}

/// Return the first document without embedded source whose file has the SHA-256 hash of the
/// document, with the file bytes.
fn candidate<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    dotnet: &Path,
) -> pdbrs::Result<Option<(DocumentId, Vec<u8>)>> {
    let embedded: Vec<DocumentId> = embedded(editor)?.into_iter().map(|(id, _)| id).collect();
    let sha256 = HashAlgorithm::Sha256.guid();
    for (id, document) in editor.documents_mut()?.iter() {
        if embedded.contains(&id) || document.hash_algorithm != Some(sha256) {
            continue;
        }
        let Some(source) =
            document_file(dotnet, &document.name).and_then(|path| std::fs::read(path).ok())
        else {
            continue;
        };
        if crate::sha256::digest(&source).as_slice() == &*document.hash {
            return Ok(Some((id, source)));
        }
    }
    Ok(None)
}

/// Apply `task` to the model. Return `None` if the task does not apply.
fn apply<S: ReadAt>(
    editor: &mut PdbEditor<'_, S>,
    task: Task,
    dotnet: &Path,
) -> pdbrs::Result<Option<Change>> {
    match task {
        Task::SourceLink => {
            editor.set_source_link(SOURCE_LINK.to_vec())?;
            Ok(Some(Change::SourceLink))
        }
        Task::EmbedSource => {
            let Some((document, source)) = candidate(editor, dotnet)? else {
                return Ok(None);
            };
            let format = EmbeddedFormat::Deflate;
            editor.embed_source(document, &source, format)?;
            Ok(Some(Change::Embedded {
                document,
                source,
                format,
            }))
        }
        Task::ReplaceEmbeddedSource => {
            let Some((document, blob)) = embedded(editor)?.into_iter().next() else {
                return Ok(None);
            };
            let current = EmbeddedSource::parse(&blob)?;
            let source = current.decode(EMBEDDED_LIMIT)?.into_owned();
            // The other form, so that the blob changes.
            let format = if current.format() == EmbeddedFormat::Raw {
                EmbeddedFormat::Deflate
            } else {
                EmbeddedFormat::Raw
            };
            editor.embed_source(document, &source, format)?;
            Ok(Some(Change::Embedded {
                document,
                source,
                format,
            }))
        }
    }
}

/// Apply `task` to a new editor of `pdb` and plan. Return `None` if the task does not apply.
fn edited<S: ReadAt>(pdb: &Pdb<S>, task: Task, dotnet: &Path) -> Result<Option<Edited>, String> {
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let Some(change) = apply(&mut editor, task, dotnet).map_err(pdbrs)? else {
        return Ok(None);
    };
    let plan = editor.plan(EditOptions::default()).map_err(pdbrs)?;
    let reported = plan
        .report()
        .streams()
        .iter()
        .filter(|(_, change)| *change != StreamChange::Copied)
        .map(|(stream, _)| stream.name().to_owned())
        .collect();
    let bytes = plan.to_vec().map_err(pdbrs)?;
    Ok(Some(Edited {
        bytes,
        reported,
        change,
    }))
}

/// Return `text` in the quoted form of the verifier.
pub(super) fn quote(text: &[u8]) -> String {
    let mut quoted = String::from("\"");
    for &value in text {
        if (0x20..0x7f).contains(&value) && value != b'"' && value != b'\\' {
            quoted.push(char::from(value));
        } else {
            let _ = write!(quoted, "\\x{value:02x}");
        }
    }
    quoted.push('"');
    quoted
}

/// Return the 64-bit FNV-1a hash of `bytes`, as the verifier computes it.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &value| {
        (hash ^ u64::from(value)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Return the row ID of the `cdi` line with `parent` and `kind`, if one exists.
fn custom_row(lines: &[String], parent: &str, kind: &str) -> Option<String> {
    lines
        .iter()
        .find(|line| {
            keyword(line) == "cdi"
                && field(line, "parent") == Some(parent)
                && field(line, "kind") == Some(kind)
        })
        .and_then(|line| field(line, "id"))
        .map(str::to_owned)
}

/// Add a custom debug information value of `parent` with the lines `cdi` and `detail` to
/// `lines`. Give the `cdi` field of each `source` line its new row ID, and return the row ID
/// of the new value.
fn add_custom(
    lines: &mut Vec<String>,
    parent: u32,
    cdi: &str,
    detail: &str,
) -> Result<u32, String> {
    let token = format!("{parent:#010x}");
    let coded_parent = coded(&token).ok_or_else(|| format!("{token} is not a coded parent"))?;
    let before = lines
        .iter()
        .filter(|line| keyword(line) == "cdi")
        .filter(|line| {
            field(line, "parent")
                .and_then(coded)
                .is_some_and(|row| row <= coded_parent)
        })
        .count();
    expect::shift_sources(lines, parent)?;
    lines.push(format!("cdi id=0 parent={token} {cdi}"));
    lines.push(detail.to_owned());
    u32::try_from(before + 1).map_err(|error| error.to_string())
}

/// Replace the detail line of `keyword_name` of the value `row` with `detail`.
fn replace_detail(
    lines: &mut [String],
    keyword_name: &str,
    row: &str,
    detail: &str,
) -> Result<(), String> {
    let index = lines
        .iter()
        .position(|line| keyword(line) == keyword_name && field(line, "id") == Some(row))
        .ok_or_else(|| format!("the value {row} has no {keyword_name} line"))?;
    lines[index] = detail.replacen("id=0", &format!("id={row}"), 1);
    Ok(())
}

/// Return the expected decoded lines after a new embedded source of `document`.
fn embedded_lines(
    lines: &mut Vec<String>,
    document: DocumentId,
    source: &[u8],
    format: EmbeddedFormat,
) -> Result<(), String> {
    let name = if format == EmbeddedFormat::Raw {
        "raw"
    } else {
        "deflate"
    };
    let size = format!("size={:#x}", source.len());
    let detail = format!(
        "cdi-embedded-source id=0 format={name} {size} fnv1a={:#018x}",
        fnv1a(source)
    );
    let parent = (DOCUMENT_TABLE << 24) | document.0;
    if let Some(row) = custom_row(lines, &format!("{parent:#010x}"), EMBEDDED_SOURCE_KIND) {
        return replace_detail(lines, "cdi-embedded-source", &row, &detail);
    }
    let cdi = format!("kind={EMBEDDED_SOURCE_KIND} codec=embedded-source");
    let row = add_custom(lines, parent, &cdi, &detail)?;
    let line = format!(
        "source document={} cdi={row} {size} check=verified",
        document.0
    );
    let after = |line: &String| {
        keyword(line) == "source"
            && field(line, "document")
                .and_then(|value| value.parse::<u32>().ok())
                .is_some_and(|value| value > document.0)
    };
    match lines.iter().position(after) {
        Some(index) => lines.insert(index, line),
        None => lines.push(line),
    }
    Ok(())
}

/// Return the expected decoded lines of the output.
fn expected(change: &Change, lines: &[String]) -> Result<Vec<String>, String> {
    let mut result = lines.to_vec();
    match change {
        Change::SourceLink => {
            let detail = format!("cdi-text id=0 text={}", quote(SOURCE_LINK));
            if let Some(row) = custom_row(&result, "0x00000001", SOURCE_LINK_KIND) {
                replace_detail(&mut result, "cdi-text", &row, &detail)?;
            } else {
                let cdi = format!("kind={SOURCE_LINK_KIND} codec=source-link");
                add_custom(&mut result, 1, &cdi, &detail)?;
            }
        }
        Change::Embedded {
            document,
            source,
            format,
        } => embedded_lines(&mut result, *document, source, *format)?,
    }
    Ok(result)
}

/// Run each task on one Portable PDB. Write the results to `log`. Return the applied cases,
/// the notes and the problems.
fn fixture(
    assembly: &Path,
    dotnet: &Path,
    comparison: &Comparison,
    out: &Path,
    log: &mut String,
) -> Result<(usize, usize, usize), String> {
    let fixture = &comparison.fixture;
    let (original, theirs) = source(assembly, comparison)?;
    let pdb = Pdb::parse(&original).map_err(pdbrs)?;
    let lines = expect::decoded(&theirs);
    let _ = writeln!(log, "{fixture}:");
    let (mut applied, mut notes, mut problems) = (0, 0, 0);
    for task in TASKS {
        let name = task.name();
        let Some(Edited {
            bytes,
            reported,
            change,
        }) = edited(&pdb, task, dotnet)?
        else {
            let note = match task {
                Task::SourceLink => "the task does not apply",
                Task::EmbedSource => {
                    "the PDB has no document without embedded source whose file has its hash"
                }
                Task::ReplaceEmbeddedSource => "the PDB has no embedded source",
            };
            let _ = writeln!(log, "  note: {name}: {note}");
            notes += 1;
            continue;
        };
        applied += 1;
        let dump = write_and_dump(assembly, out, &format!("{fixture}.task-{name}"), &bytes)?;
        let expected = expected(&change, &lines)?;
        let actual = expect::decoded(&dump);
        let records = same(
            log,
            &format!("{name}: decoded dump (without custom debug information)"),
            &expect::without_custom(&expected),
            &expect::without_custom(&actual),
        );
        let custom = same(
            log,
            &format!("{name}: custom debug information (sorted, without row IDs)"),
            &expect::custom_blocks(&expected),
            &expect::custom_blocks(&actual),
        );
        let changed = changed_streams(&original, &theirs, &bytes, &dump)?;
        let agree = changed == reported;
        let _ = writeln!(
            log,
            "  {name}: streams with other bytes: {}; the report gives the same streams: {}",
            list(&changed),
            yes(agree)
        );
        if !agree {
            let _ = writeln!(log, "    the report gives: {}", list(&reported));
        }
        let again = edited(&pdb, task, dotnet)?.is_some_and(|repeated| repeated.bytes == bytes);
        let _ = writeln!(
            log,
            "  {name}: the same edit again gives the same bytes: {}",
            yes(again)
        );
        if !(records && custom && agree && again) {
            problems += 1;
        }
    }
    Ok((applied, notes, problems))
}

/// Run the tasks on each comparison and print the section `task cases:`.
///
/// Return `false` if a check fails or gives an error.
pub(super) fn run(
    jobs: &Jobs,
    assembly: &Path,
    workspace: &Path,
    comparisons: &[Comparison],
    out: &Path,
) -> bool {
    println!("task cases:");
    let dotnet = workspace.join("fixtures").join("dotnet");
    let results = jobs.map(comparisons, |comparison| {
        let mut log = String::new();
        let result = fixture(assembly, &dotnet, comparison, out, &mut log);
        (log, result)
    });
    let (mut applied, mut notes, mut problems) = (0, 0, 0);
    for (comparison, (log, result)) in comparisons.iter().zip(results) {
        print!("{log}");
        match result {
            Ok((fixture_applied, fixture_notes, fixture_problems)) => {
                applied += fixture_applied;
                notes += fixture_notes;
                problems += fixture_problems;
            }
            Err(error) => {
                println!("{}: error: {error}", comparison.fixture);
                problems += 1;
            }
        }
    }
    println!(
        "task section: portable PDBs: {}, cases: {}, applied: {applied}, notes: {notes}, problems: {problems}",
        comparisons.len(),
        comparisons.len() * TASKS.len()
    );
    problems == 0
}
