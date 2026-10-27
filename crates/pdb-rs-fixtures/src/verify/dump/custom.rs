//! The decoded custom debug information section of the dump.
//!
//! Each `CustomDebugInformation` row gives one `cdi` line with its codec name, and then the
//! detail lines of the decoded value. `fixtures/README.md` lists the lines.

use std::fmt::Write as _;

use pdb_rs::io::ReadAt;
use pdb_rs::portable::Pdb;
use pdb_rs::portable::custom::{
    CustomDebugInformation, CustomDebugValue, LambdaClosure, LocalSlot,
};
use pdb_rs::portable::embedded::{EmbeddedSource, SourceCheck};
use pdb_rs::portable::metadata::CustomDebugInformationRow;

use super::{Dump, quote};

/// The largest embedded source that the dump decodes.
const MAX_SOURCE: usize = 64 << 20;

/// Print the decoded value of each `CustomDebugInformation` row.
pub fn custom<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdb_rs::Result<()> {
    for row in pdb.custom_debug_informations() {
        let (id, row) = row?;
        let raw: CustomDebugInformationRow = pdb.row(id)?;
        let value = row.decode()?;
        let id = id.0;
        out.line(format_args!(
            "cdi id={id} parent={:#010x} kind={} codec={}",
            raw.parent.raw(),
            row.kind,
            codec(&value)
        ));
        details(id, &row, &value, out)?;
    }
    Ok(())
}

/// Print a `source` line for the embedded source of each document: the check of the decoded
/// source against the document hash.
pub fn sources<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdb_rs::Result<()> {
    for document in pdb.documents() {
        let (id, document) = document?;
        let Some(source) = pdb.document_source(id, &document, MAX_SOURCE)? else {
            continue;
        };
        let check = match source.check() {
            SourceCheck::Verified => "verified",
            SourceCheck::Mismatch => "mismatch",
            SourceCheck::NotVerified => "not-verified",
            _ => "other",
        };
        out.line(format_args!(
            "source document={} cdi={} size={:#x} check={check}",
            id.0,
            source.custom_debug_information().0,
            source.content().len()
        ));
    }
    Ok(())
}

/// Return the codec name of a value.
fn codec(value: &CustomDebugValue<'_>) -> &'static str {
    match value {
        CustomDebugValue::AsyncMethodSteppingInformation(_) => "async-stepping",
        CustomDebugValue::StateMachineHoistedLocalScopes(_) => "hoisted-scopes",
        CustomDebugValue::DynamicLocalVariables(_) => "dynamic-locals",
        CustomDebugValue::TupleElementNames(_) => "tuple-names",
        CustomDebugValue::DefaultNamespace(_) => "default-namespace",
        CustomDebugValue::EncLocalSlotMap(_) => "enc-local-slots",
        CustomDebugValue::EncLambdaAndClosureMap(_) => "enc-lambdas",
        CustomDebugValue::EncStateMachineStateMap(_) => "enc-states",
        CustomDebugValue::SourceLink(_) => "source-link",
        CustomDebugValue::EmbeddedSource(_) => "embedded-source",
        CustomDebugValue::CompilationMetadataReferences(_) => "metadata-references",
        CustomDebugValue::CompilationOptions(_) => "compilation-options",
        CustomDebugValue::TypeDefinitionDocuments(_) => "type-documents",
        CustomDebugValue::PrimaryConstructorInformation => "primary-constructor",
        _ => "unknown",
    }
}

/// Print the detail lines of a decoded value.
fn details(
    id: u32,
    row: &CustomDebugInformation<'_>,
    value: &CustomDebugValue<'_>,
    out: &mut Dump,
) -> pdb_rs::Result<()> {
    match value {
        CustomDebugValue::AsyncMethodSteppingInformation(info) => {
            let catch = info
                .catch_handler_offset()
                .map_or_else(|| String::from("none"), |offset| format!("{:#x}", offset.0));
            out.line(format_args!("cdi-async id={id} catch-handler={catch}"));
            for step in info {
                out.line(format_args!(
                    "cdi-async-step id={id} yield={:#x} resume={:#x} method={}",
                    step.yield_offset.0, step.resume_offset.0, step.move_next_method.0
                ));
            }
        }
        CustomDebugValue::StateMachineHoistedLocalScopes(scopes) => {
            for scope in scopes {
                out.line(format_args!(
                    "cdi-hoisted-scope id={id} start={:#x} length={:#x}",
                    scope.start.0, scope.length
                ));
            }
        }
        CustomDebugValue::DynamicLocalVariables(flags) => {
            let bits: String = flags
                .iter()
                .map(|flag| if flag { '1' } else { '0' })
                .collect();
            out.line(format_args!("cdi-dynamic id={id} flags={bits}"));
        }
        CustomDebugValue::TupleElementNames(names) => {
            for name in names {
                out.line(format_args!("cdi-tuple-name id={id} name={}", text(&name)));
            }
        }
        CustomDebugValue::DefaultNamespace(value) | CustomDebugValue::SourceLink(value) => {
            out.line(format_args!("cdi-text id={id} text={}", text(value)));
        }
        CustomDebugValue::EmbeddedSource(source) => embedded_source(id, source, out)?,
        CustomDebugValue::CompilationMetadataReferences(references) => {
            for reference in references {
                out.line(format_args!(
                    "cdi-reference id={id} file={} aliases={} flags={:#04x} time-stamp={:#010x} file-size={:#010x} mvid={}",
                    text(&reference.file_name),
                    text(&reference.aliases),
                    reference.flags,
                    reference.time_stamp,
                    reference.file_size,
                    hex(&reference.mvid)
                ));
            }
        }
        CustomDebugValue::CompilationOptions(options) => {
            for option in options {
                out.line(format_args!(
                    "cdi-option id={id} name={} value={}",
                    text(&option.name),
                    text(&option.value)
                ));
            }
        }
        CustomDebugValue::TypeDefinitionDocuments(documents) => {
            for document in documents {
                out.line(format_args!(
                    "cdi-type-document id={id} document={}",
                    document.0
                ));
            }
        }
        CustomDebugValue::PrimaryConstructorInformation => {}
        CustomDebugValue::EncLocalSlotMap(_)
        | CustomDebugValue::EncLambdaAndClosureMap(_)
        | CustomDebugValue::EncStateMachineStateMap(_) => edit_and_continue(id, value, out),
        _ => out.line(format_args!(
            "cdi-raw id={id} size={:#x} bytes={}",
            row.value.len(),
            hex(&row.value)
        )),
    }
    Ok(())
}

/// Print the lines of an Edit and Continue map.
fn edit_and_continue(id: u32, value: &CustomDebugValue<'_>, out: &mut Dump) {
    match value {
        CustomDebugValue::EncLocalSlotMap(slots) => {
            for slot in slots {
                match slot {
                    LocalSlot::Temporary => {
                        out.line(format_args!("cdi-enc-slot id={id} temporary"));
                    }
                    LocalSlot::Variable {
                        kind,
                        syntax_offset,
                        ordinal,
                    } => {
                        let ordinal =
                            ordinal.map_or_else(|| String::from("none"), |o| o.to_string());
                        out.line(format_args!(
                            "cdi-enc-slot id={id} kind={kind} offset={syntax_offset} ordinal={ordinal}"
                        ));
                    }
                }
            }
        }
        CustomDebugValue::EncLambdaAndClosureMap(map) => {
            out.line(format_args!(
                "cdi-enc-lambda-map id={id} method-ordinal={}",
                map.method_ordinal()
            ));
            for offset in map.closures() {
                out.line(format_args!("cdi-enc-closure id={id} offset={offset}"));
            }
            for lambda in map.lambdas() {
                let closure = match lambda.closure {
                    LambdaClosure::ThisOnly => String::from("this"),
                    LambdaClosure::Static => String::from("static"),
                    LambdaClosure::Closure(index) => index.to_string(),
                };
                out.line(format_args!(
                    "cdi-enc-lambda id={id} offset={} closure={closure}",
                    lambda.syntax_offset
                ));
            }
        }
        CustomDebugValue::EncStateMachineStateMap(states) => {
            for state in states {
                out.line(format_args!(
                    "cdi-enc-state id={id} number={} offset={}",
                    state.state_number, state.syntax_offset
                ));
            }
        }
        _ => {}
    }
}

/// Print the format, the decoded size and the FNV-1a hash of an embedded source.
fn embedded_source(id: u32, source: &EmbeddedSource<'_>, out: &mut Dump) -> pdb_rs::Result<()> {
    let format = match source {
        EmbeddedSource::Raw(_) => "raw",
        EmbeddedSource::Deflate { .. } => "deflate",
        _ => "other",
    };
    let bytes = source.decode(MAX_SOURCE)?;
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    out.line(format_args!(
        "cdi-embedded-source id={id} format={format} size={:#x} fnv1a={hash:#018x}",
        bytes.len()
    ));
    Ok(())
}

/// Quote text bytes.
fn text(value: &[u8]) -> String {
    quote(value)
}

/// Return bytes as lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}
