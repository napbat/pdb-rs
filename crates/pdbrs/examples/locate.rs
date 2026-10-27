//! Print the source location of RVAs and the symbols of names in a native PDB.
//!
//! Usage:
//!
//! ```text
//! cargo run --example locate -- <pdb> [<rva>...] [--sample <count>] [--name <name>...]
//!     [--sample-names <count>]
//! ```
//!
//! An RVA is a hexadecimal number with or without `0x`. `--sample <count>`
//! selects up to `count` addresses from the PDB: procedure starts, bytes
//! inside procedures, the last byte of procedures, the byte after procedures,
//! inline range starts, bytes inside inline ranges, line entry starts, the
//! byte before a line entry, and the start and a middle byte of separated
//! code chunks. `--sample-names <count>` selects up to `count` names of
//! procedures and of the public symbols at their starts.
//!
//! The output has one block for each address:
//!
//! ```text
//! ADDR <rva> <sample kind>
//! MODULE <index> <name>
//! PROC <name> <start rva> <length> <record kind>
//! PUBLIC <name> <public rva>
//! FRAME <name>\t<file>\t<line>\t<column>
//! END
//! ```
//!
//! The `FRAME` lines give the inline call chain from the innermost frame to
//! the procedure, as `llvm-symbolizer --inlines` does. An inline frame gives
//! the line in the inlined function. The procedure frame gives the line from
//! the `DEBUG_S_LINES` subsections. A missing value is `?`. A column is 0
//! if the data has no columns. An address outside all image sections gives
//! only `ADDR` and `NONE`.
//!
//! The output has one block for each name:
//!
//! ```text
//! NAME <name>
//! MATCH procedure <rva> <module> <name>
//! MATCH public <rva> <name>
//! MATCH global <kind> <rva> <name>
//! END
//! ```
//!
//! Names are printed with invalid UTF-8 replaced. Use the output only to
//! compare with other tools.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Write as _;

use pdbrs::native::Pdb;
use pdbrs::native::address::Rva;
use pdbrs::native::locate::{Location, NameMatch, SourceLine, SymbolLocator};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The parsed command line.
#[derive(Default)]
struct Arguments {
    path: Option<String>,
    rvas: Vec<u32>,
    sample: usize,
    names: Vec<String>,
    sample_names: usize,
}

fn main() -> Outcome {
    let arguments = parse_arguments()?;
    let path = arguments.path.ok_or(
        "usage: cargo run --example locate -- <pdb> [<rva>...] [--sample <count>] \
         [--name <name>...] [--sample-names <count>]",
    )?;
    let pdb = Pdb::open(path)?;
    let locator = SymbolLocator::new(&pdb)?;
    for &module in locator.incomplete_modules() {
        eprintln!(
            "module {} has an undecoded scope kind; later records are not indexed",
            module.0
        );
    }
    let mut out = String::new();
    let mut rvas: Vec<(u32, &str)> = arguments
        .rvas
        .iter()
        .map(|&rva| (rva, "argument"))
        .collect();
    if arguments.sample > 0 {
        rvas.extend(sample_addresses(&locator, arguments.sample)?);
    }
    for (rva, kind) in rvas {
        print_location(&mut out, &locator, rva, kind)?;
    }
    let mut names = arguments.names.clone();
    if arguments.sample_names > 0 {
        names.extend(sample_names(&locator, arguments.sample_names)?);
    }
    for name in names {
        print_name(&mut out, &locator, &name)?;
    }
    print!("{out}");
    Ok(())
}

fn parse_arguments() -> Outcome<Arguments> {
    let mut arguments = Arguments::default();
    let mut input = std::env::args().skip(1);
    while let Some(argument) = input.next() {
        match argument.as_str() {
            "--sample" => {
                arguments.sample = input.next().ok_or("--sample needs a count")?.parse()?;
            }
            "--sample-names" => {
                arguments.sample_names = input
                    .next()
                    .ok_or("--sample-names needs a count")?
                    .parse()?;
            }
            "--name" => arguments
                .names
                .push(input.next().ok_or("--name needs a name")?),
            _ if arguments.path.is_none() => arguments.path = Some(argument),
            _ => {
                let digits = argument.trim_start_matches("0x").trim_start_matches("0X");
                arguments.rvas.push(u32::from_str_radix(digits, 16)?);
            }
        }
    }
    Ok(arguments)
}

/// The most line entries that the sampling visits in one procedure.
const MAX_SAMPLED_LINES: usize = 64;

/// Select up to `count` addresses, spread over the sample kinds.
fn sample_addresses(locator: &SymbolLocator, count: usize) -> Outcome<Vec<(u32, &'static str)>> {
    let mut kinds: Vec<(&'static str, Vec<u32>)> = [
        "procedure-start",
        "procedure-middle",
        "procedure-last",
        "procedure-after",
        "inline-start",
        "inline-middle",
        "line-start",
        "line-before",
        "separated-start",
        "separated-middle",
    ]
    .into_iter()
    .map(|kind| (kind, Vec::new()))
    .collect();
    for procedure in locator.procedures() {
        let procedure = procedure?;
        let (Some(start), true) = (procedure.start, procedure.length > 0) else {
            continue;
        };
        let start = start.0;
        let end = start + procedure.length;
        kinds[0].1.push(start);
        kinds[1].1.push(start + procedure.length / 2);
        kinds[2].1.push(end - 1);
        kinds[3].1.push(end);
        let chunks = locator.code_chunks(&procedure)?;
        for chunk in &chunks[1..] {
            if let (Some(chunk_start), true) = (chunk.start, chunk.length > 0) {
                kinds[8].1.push(chunk_start.0);
                kinds[9].1.push(chunk_start.0 + chunk.length / 2);
            }
        }
        for site in locator.inline_sites(&procedure)? {
            for range in site.ranges {
                let Some((range_start, length)) =
                    range.rva_range(&chunks).filter(|&(_, length)| length > 0)
                else {
                    continue;
                };
                kinds[4].1.push(range_start.0);
                kinds[5].1.push(range_start.0 + length / 2);
            }
        }
        let mut rva = start;
        for _ in 0..MAX_SAMPLED_LINES {
            let Some(Location {
                line: Some(line), ..
            }) = locator.locate(Rva(rva))?
            else {
                break;
            };
            let next = line.start.0 + line.length.unwrap_or(0).max(1);
            if next >= end {
                break;
            }
            kinds[6].1.push(next);
            kinds[7].1.push(next - 1);
            rva = next;
        }
    }
    // Take the addresses of each kind in a fixed pseudo-random order, one
    // kind after the other, until `count` addresses are selected.
    for (_, addresses) in &mut kinds {
        addresses.sort_unstable();
        addresses.dedup();
        addresses.sort_by_key(|&rva| (rva.wrapping_mul(0x9e37_79b1), rva));
    }
    let mut selected = BTreeSet::new();
    let mut result = Vec::new();
    let longest = kinds.iter().map(|(_, addresses)| addresses.len()).max();
    for position in 0..longest.unwrap_or(0) {
        for (kind, addresses) in &kinds {
            if let Some(&rva) = addresses.get(position)
                && result.len() < count
                && selected.insert(rva)
            {
                result.push((rva, *kind));
            }
        }
    }
    Ok(result)
}

/// Select up to `count` names of procedures and of their public symbols.
fn sample_names(locator: &SymbolLocator, count: usize) -> Outcome<Vec<String>> {
    let mut names = BTreeSet::new();
    let procedures: Vec<_> = locator.procedures().collect::<Result<_, _>>()?;
    let stride = procedures.len().div_ceil(count.div_ceil(2).max(1)).max(1);
    for procedure in procedures.iter().step_by(stride) {
        names.insert(text(procedure.name));
        if let Some(start) = procedure.start
            && let Some(public) = locator.public_at(start)?
            && public.displacement == 0
        {
            names.insert(text(&public.public.name));
        }
    }
    Ok(names
        .into_iter()
        .filter(|name| !name.is_empty())
        .take(count)
        .collect())
}

fn print_location(out: &mut String, locator: &SymbolLocator, rva: u32, kind: &str) -> Outcome {
    writeln!(out, "ADDR {rva:#010x} {kind}")?;
    let Some(location) = locator.locate(Rva(rva))? else {
        writeln!(out, "NONE")?;
        return Ok(());
    };
    if let Some(module) = location.module {
        writeln!(out, "MODULE {} {}", module.index.0, text(module.name))?;
    }
    if let Some(procedure) = location.procedure {
        let start = procedure.start.map_or(0, |start| start.0);
        writeln!(
            out,
            "PROC {} {start:#010x} {:#x} {:#06x}",
            text(procedure.name),
            procedure.length,
            procedure.kind
        )?;
    }
    if let Some(public) = locator.public_at(Rva(rva))? {
        writeln!(
            out,
            "PUBLIC {} {:#010x}",
            text(&public.public.name),
            public.rva.0
        )?;
    }
    for frame in &location.inline_frames {
        frame_line(out, frame.qualified_name, frame.line.as_ref())?;
    }
    if let Some(procedure) = location.procedure {
        frame_line(out, procedure.name, location.line.as_ref())?;
    } else if location.line.is_some() {
        frame_line(out, b"?", location.line.as_ref())?;
    }
    writeln!(out, "END")?;
    Ok(())
}

fn frame_line(out: &mut String, name: &[u8], line: Option<&SourceLine<'_>>) -> Outcome {
    match line {
        Some(line) => writeln!(
            out,
            "FRAME {}\t{}\t{}\t{}",
            text(name),
            text(line.file),
            line.line,
            line.column.unwrap_or(0)
        )?,
        None => writeln!(out, "FRAME {}\t?\t?\t?", text(name))?,
    }
    Ok(())
}

fn print_name(out: &mut String, locator: &SymbolLocator, name: &str) -> Outcome {
    writeln!(out, "NAME {name}")?;
    for found in locator.find_name(name.as_bytes())? {
        match found {
            NameMatch::Procedure { procedure, .. } => writeln!(
                out,
                "MATCH procedure {:#010x} {} {}",
                procedure.start.map_or(0, |start| start.0),
                procedure.module.0,
                text(procedure.name)
            )?,
            NameMatch::Public { public, rva, .. } => writeln!(
                out,
                "MATCH public {:#010x} {}",
                rva.map_or(0, |rva| rva.0),
                text(&public.name)
            )?,
            NameMatch::Global { symbol, .. } => {
                let rva = match symbol.address() {
                    Some(address) => locator.rva(address)?.map_or(0, |rva| rva.0),
                    None => 0,
                };
                writeln!(
                    out,
                    "MATCH global {:#06x} {rva:#010x} {}",
                    symbol.kind(),
                    text(symbol.name().map_or(&[], |name| name))
                )?;
            }
            _ => writeln!(out, "MATCH other")?,
        }
    }
    writeln!(out, "END")?;
    Ok(())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
