//! Inspect native PDB streams and enumerate their records.

use pdbrs::native::Pdb;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::symbol::SymbolData;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: cargo run --example native -- <path>")?;
    if arguments.next().is_some() {
        return Err("expected exactly one input path".into());
    }
    let pdb = Pdb::open(path)?;
    println!("Block size: {}", pdb.msf().block_size());
    println!("Streams: {}", pdb.msf().stream_count());
    println!("Signature: {}", pdb.info().signature());
    println!("Identity: {:?}", pdb.info().identity());
    for named in pdb.info().named_streams() {
        println!(
            "Named stream {}: {}",
            named.stream.0,
            String::from_utf8_lossy(named.name)
        );
    }
    for (label, stream) in [("TPI", pdb.types()?), ("IPI", pdb.ids()?)] {
        if let Some(stream) = stream {
            let count = stream
                .records()
                .try_fold(0_usize, |count, record| record.map(|_| count + 1))?;
            println!("{label} records: {count}");
        } else {
            println!("{label}: absent");
        }
    }
    let Some(dbi) = pdb.debug_info()? else {
        println!("DBI: absent");
        return Ok(());
    };
    println!("Machine: {:#06x}", dbi.header().machine);
    let mut shared_count = 0_usize;
    let mut shared_decoded = 0_usize;
    if let Some(id) = dbi.header().symbol_stream {
        let stream = pdb
            .msf()
            .stream(id)?
            .ok_or("missing shared symbol stream")?;
        let bytes = stream.read_all(64 * 1024 * 1024)?;
        for record in Records::new(&bytes) {
            let record = record?;
            shared_count += 1;
            let symbol = record.symbol()?;
            if !matches!(symbol.data(), SymbolData::Unsupported(_)) {
                if shared_decoded < 8
                    && let Some(name) = symbol.name()
                {
                    println!("Symbol: {}", String::from_utf8_lossy(name));
                }
                shared_decoded += 1;
            }
        }
    }
    println!("Shared symbol records: {shared_count}");
    println!("Decoded shared symbols: {shared_decoded}");
    let mut module_count = 0_usize;
    let mut module_symbols = 0_usize;
    let mut decoded_module_symbols = 0_usize;
    let mut subsections = 0_usize;
    for module in dbi.modules() {
        let module = module?;
        module_count += 1;
        if let Some(stream) = pdb.module_stream(&module)? {
            for record in stream.symbols() {
                module_symbols += 1;
                if !matches!(record?.symbol()?.data(), SymbolData::Unsupported(_)) {
                    decoded_module_symbols += 1;
                }
            }
            for subsection in stream.c13_subsections() {
                subsection?;
                subsections += 1;
            }
        }
    }
    println!("Modules: {module_count}");
    println!("Module symbol records: {module_symbols}");
    println!("Decoded module symbols: {decoded_module_symbols}");
    println!("C13 subsections: {subsections}");
    println!(
        "Record framing is checked; unknown kinds remain raw. Full semantic validation is not implied."
    );
    Ok(())
}
