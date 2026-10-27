//! Print the type information of a native PDB in a C-like form.
//!
//! Usage: `cargo run --example types -- [--type <name> | --functions] <path>`.
//!
//! - Without an option, the example prints each UDT definition (class,
//!   structure, interface or union) and each enum of the TPI stream, in index
//!   order.
//! - `--type <name>` prints only the UDT definitions and enums with this name.
//! - `--functions` prints the signature of each procedure symbol in the module
//!   streams, then the signature of each `LF_FUNC_ID` and `LF_MFUNC_ID` record.
//!
//! A UDT gives its size, and then one line for each member: base classes,
//! virtual base classes, the virtual function table pointer, data members with
//! their offsets (a bit field also gives its width and its first bit), static
//! members, nested types and methods with their signatures. An enum gives its
//! underlying type and each enumerator with its value. Values can be signed or
//! unsigned and up to 128 bits wide.
//!
//! The example uses only the public API: [`Pdb`], [`TypeTable`],
//! [`ForwardRefs`], `TypeLeaf`, `TypeTable::members` and `Numeric`. Names stay
//! bytes in the reader; the example prints them as UTF-8 with replacement
//! characters.

mod functions;
mod layout;
mod names;

use std::error::Error;
use std::ffi::OsString;

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::codeview::id_record::IdTable;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::types::{ForwardRefs, TypeHashes, TypeStream, TypeTable};

use layout::{print_enum, print_udt, udt};
use names::Types;

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The usage text.
const USAGE: &str = "usage: cargo run --example types -- [--type <name> | --functions] <path>";

/// What the example prints.
enum Mode {
    /// Each UDT definition and each enum.
    All,
    /// The UDT definitions and enums with this name.
    Type(Vec<u8>),
    /// The procedure and function ID signatures.
    Functions,
}

/// Parse `[--type <name> | --functions] <path>`.
fn arguments(mut arguments: impl Iterator<Item = OsString>) -> Outcome<(Mode, OsString)> {
    let mut mode = Mode::All;
    let mut path = None;
    while let Some(argument) = arguments.next() {
        if argument == "--type" {
            let name = arguments.next().ok_or(USAGE)?;
            let name = name
                .into_string()
                .map_err(|_| "the type name is not UTF-8")?;
            mode = Mode::Type(name.into_bytes());
        } else if argument == "--functions" {
            mode = Mode::Functions;
        } else if path.is_none() {
            path = Some(argument);
        } else {
            return Err(USAGE.into());
        }
    }
    Ok((mode, path.ok_or(USAGE)?))
}

fn main() -> Outcome {
    let (mode, path) = arguments(std::env::args_os().skip(1))?;
    run(&Pdb::open(path)?, &mode)
}

/// Load the TPI stream and its hash stream, and print what `mode` selects.
fn run(pdb: &Pdb<impl ReadAt>, mode: &Mode) -> Outcome {
    let tpi = pdb.types()?.ok_or("the PDB has no TPI stream")?;
    let hash_bytes = match tpi.header().hash_stream {
        Some(id) => Some(pdb.stream_bytes(id)?),
        None => None,
    };
    let hashes = hash_bytes
        .as_deref()
        .map(|bytes| TypeHashes::parse(tpi.header(), bytes))
        .transpose()?;
    let table = TypeTable::new(&tpi)?;
    let types = Types {
        table: &table,
        forward: ForwardRefs::new(&table, hashes.as_ref())?,
    };
    match mode {
        Mode::All => {
            let (udts, enums) = definitions(&types, &tpi, None)?;
            println!("UDT definitions: {udts}, enums: {enums}");
        }
        Mode::Type(name) => {
            let (udts, enums) = definitions(&types, &tpi, Some(name))?;
            if udts + enums == 0 {
                return Err(format!(
                    "no UDT definition or enum has the name `{}`",
                    String::from_utf8_lossy(name)
                )
                .into());
            }
        }
        Mode::Functions => {
            let ipi = pdb.ids()?;
            let ids = ipi.as_ref().map(IdTable::new).transpose()?;
            let procedures = functions::procedures(pdb, &types, ids.as_ref())?;
            let records = match (&ipi, &ids) {
                (Some(ipi), Some(ids)) => functions::function_ids(&types, ipi, ids)?,
                _ => 0,
            };
            println!("procedures: {procedures}, function IDs: {records}");
        }
    }
    Ok(())
}

/// Print each UDT definition and each enum, or only those with the name `only`.
///
/// An error names the type index of the UDT or enum. Return the number of UDT
/// definitions and the number of enums.
fn definitions(
    types: &Types<'_, '_>,
    tpi: &TypeStream,
    only: Option<&[u8]>,
) -> Outcome<(usize, usize)> {
    let (mut udts, mut enums) = (0, 0);
    for record in tpi.records() {
        let record = record?;
        let index = TypeIndex(record.index);
        let context = |error: pdbrs::Error| format!("type {:#x}: {error}", index.0);
        let leaf = TypeLeaf::decode(record.record).map_err(context)?;
        let Some(tag) = leaf.tag() else {
            continue;
        };
        if tag.options.is_forward_reference() || only.is_some_and(|name| name != &*tag.name) {
            continue;
        }
        if let TypeLeaf::Enum(value) = &leaf {
            print_enum(
                types,
                index,
                &value.name,
                value.underlying_type,
                value.field_list,
            )
            .map_err(context)?;
            enums += 1;
        } else if let Some(udt) = udt(&leaf) {
            print_udt(types, index, &udt).map_err(context)?;
            udts += 1;
        }
    }
    Ok((udts, enums))
}
