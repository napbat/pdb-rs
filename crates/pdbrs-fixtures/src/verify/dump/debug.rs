//! The decoded debug table sections of the dump (#23).
//!
//! The sections come after the `row` lines and before the `cdi` lines: documents, sequence
//! points, local scopes with their variables and constants, import scopes with their imports,
//! and state machine methods. `fixtures/README.md` lists the lines.

use std::fmt::Write as _;

use pdbrs::io::ReadAt;
use pdbrs::portable::Pdb;
use pdbrs::portable::debug::{
    ConstantValue, GeneralValue, Import, LocalConstantSignature, PrimitiveConstant,
};

use super::{Dump, quote};

/// Print the decoded debug table sections.
pub fn debug<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdbrs::Result<()> {
    documents(pdb, out)?;
    sequence_points(pdb, out)?;
    scopes(pdb, out)?;
    imports(pdb, out)?;
    for method in pdb.state_machine_methods() {
        let method = method?;
        out.line(format_args!(
            "state-machine move-next={} kickoff={}",
            method.move_next_method().get(),
            method.kickoff_method().get()
        ));
    }
    Ok(())
}

/// Print one `document` line for each document.
fn documents<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdbrs::Result<()> {
    for document in pdb.documents() {
        let document = document?;
        out.line(format_args!(
            "document id={} name={} language={} hash-algorithm={} hash={}",
            document.id().get(),
            text(document.name()),
            guid(document.language_guid()),
            guid(document.hash_algorithm_guid()),
            hex(document.hash())
        ));
    }
    Ok(())
}

/// Print the sequence points of each method that has a sequence point blob.
fn sequence_points<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdbrs::Result<()> {
    for method in pdb.method_debug_informations() {
        let method = method?;
        let Some(decoded) = method.sequence_points()? else {
            continue;
        };
        let signature = decoded.local_signature();
        let points = decoded
            .collect::<pdbrs::Result<Vec<_>>>()
            .map_err(|error| method.blob_error(error))?;
        let id = method.id().get();
        out.line(format_args!(
            "method-debug id={id} document={} local-signature={} points={}",
            method.document().get(),
            signature.get(),
            points.len()
        ));
        for point in points {
            let prefix = format!(
                "sequence-point method={id} il={:#x} document={}",
                point.il_offset.get(),
                point.document.get()
            );
            match point.span {
                None => out.line(format_args!("{prefix} hidden")),
                Some(span) => out.line(format_args!(
                    "{prefix} start-line={} start-column={} end-line={} end-column={}",
                    span.start_line, span.start_column, span.end_line, span.end_column
                )),
            }
        }
    }
    Ok(())
}

/// Print each local scope with its variables and constants.
fn scopes<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdbrs::Result<()> {
    for scope in pdb.local_scopes() {
        let scope = scope?;
        let id = scope.id().get();
        out.line(format_args!(
            "scope id={id} method={} import-scope={} start={:#x} end={:#x} variables={} constants={}",
            scope.method().get(),
            scope.import_scope().get(),
            scope.start_offset().get(),
            u64::from(scope.start_offset().get()) + u64::from(scope.length()),
            scope.variables().len(),
            scope.constants().len()
        ));
        for variable in scope.variables() {
            let variable = pdb.local_variable(variable)?;
            out.line(format_args!(
                "scope-variable scope={id} id={} index={} attributes={:#06x} name={}",
                variable.id().get(),
                variable.index(),
                variable.attributes(),
                text(variable.name())
            ));
        }
        for constant in scope.constants() {
            let constant = pdb.local_constant(constant)?;
            out.line(format_args!(
                "scope-constant scope={id} id={} name={} {}",
                constant.id().get(),
                text(constant.name()),
                signature(&constant.signature()?)
            ));
        }
    }
    Ok(())
}

/// Format a local constant signature.
fn signature(signature: &LocalConstantSignature<'_>) -> String {
    let mut text = String::new();
    for modifier in signature.modifiers() {
        let kind = if modifier.required {
            "required"
        } else {
            "optional"
        };
        let _ = write!(text, "modifier={kind}:{:#010x} ", modifier.modifier.get());
    }
    match signature.value() {
        ConstantValue::Primitive(value) => text.push_str(&primitive(value)),
        ConstantValue::Enum { value, enum_type } => {
            let _ = write!(text, "{} enum={:#010x}", primitive(value), enum_type.get());
        }
        ConstantValue::General {
            value_type,
            constant_type,
            value,
        } => {
            let kind = if value_type { "valuetype" } else { "class" };
            let _ = write!(text, "{kind}={:#010x} ", constant_type.get());
            match value {
                GeneralValue::Default => text.push_str("default"),
                GeneralValue::DateTime(ticks) => {
                    let _ = write!(text, "datetime={ticks}");
                }
                GeneralValue::Decimal(decimal) => {
                    let _ = write!(
                        text,
                        "decimal={}:{}:{:#010x}:{:#010x}:{:#010x}",
                        u8::from(decimal.negative),
                        decimal.scale,
                        decimal.high,
                        decimal.mid,
                        decimal.low
                    );
                }
            }
        }
        ConstantValue::Object => text.push_str("object"),
    }
    text
}

/// Format a primitive constant.
fn primitive(value: PrimitiveConstant<'_>) -> String {
    match value {
        PrimitiveConstant::Boolean(value) => format!("boolean={value}"),
        PrimitiveConstant::Char(value) => format!("char={value:#06x}"),
        PrimitiveConstant::I1(value) => format!("i1={value}"),
        PrimitiveConstant::U1(value) => format!("u1={value}"),
        PrimitiveConstant::I2(value) => format!("i2={value}"),
        PrimitiveConstant::U2(value) => format!("u2={value}"),
        PrimitiveConstant::I4(value) => format!("i4={value}"),
        PrimitiveConstant::U4(value) => format!("u4={value}"),
        PrimitiveConstant::I8(value) => format!("i8={value}"),
        PrimitiveConstant::U8(value) => format!("u8={value}"),
        PrimitiveConstant::R4(value) => format!("r4={:#010x}", value.to_bits()),
        PrimitiveConstant::R8(value) => format!("r8={:#018x}", value.to_bits()),
        PrimitiveConstant::String(None) => "string=null".to_owned(),
        PrimitiveConstant::String(Some(bytes)) => {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|unit| u16::from_le_bytes(*unit))
                .collect();
            format!(
                "string={}",
                quote(String::from_utf16_lossy(&units).as_bytes())
            )
        }
    }
}

/// Print each import scope with its imports.
fn imports<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdbrs::Result<()> {
    for scope in pdb.import_scopes() {
        let scope = scope?;
        let id = scope.id().get();
        let imports = scope
            .imports()
            .collect::<pdbrs::Result<Vec<Import>>>()
            .map_err(|error| scope.blob_error(error))?;
        out.line(format_args!(
            "import-scope id={id} parent={} imports={}",
            scope.parent().get(),
            imports.len()
        ));
        for import in imports {
            let mut line = format!("import scope={id} kind={}", import.kind());
            if let Some(alias) = import.alias() {
                let _ = write!(line, " alias={}", quote(&pdb.blob(alias)?));
            }
            if let Some(assembly) = import.assembly() {
                let _ = write!(line, " assembly={}", assembly.get());
            }
            if let Some(namespace) = import.namespace() {
                let _ = write!(line, " namespace={}", quote(&pdb.blob(namespace)?));
            }
            if let Some(target) = import.target_type() {
                let _ = write!(line, " type={:#010x}", target.get());
            }
            out.line(format_args!("{line}"));
        }
    }
    Ok(())
}

/// Quote UTF-8 text the way .NET decodes it: invalid bytes become U+FFFD.
fn text(bytes: &[u8]) -> String {
    quote(String::from_utf8_lossy(bytes).as_bytes())
}

/// Format GUID bytes in serialized order as lowercase text. `None` gives the empty GUID.
fn guid(bytes: Option<&[u8; 16]>) -> String {
    let b = bytes.copied().unwrap_or_default();
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Return bytes as lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}
