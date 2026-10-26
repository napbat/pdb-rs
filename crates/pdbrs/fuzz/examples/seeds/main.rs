//! Write the seed corpora of the fuzz targets.
//!
//! Usage:
//!
//! ```text
//! cargo +nightly run --release --example seeds -- [--out <dir>] [<fixture file or directory>...]
//! ```
//!
//! The generator builds a synthetic native PDB, a synthetic Portable PDB and small synthetic
//! inputs with the public API of `pdbrs`. It checks that each synthetic PDB reads back. Then it
//! splits each PDB into the inputs of the targets: streams, records, blobs and values. Each
//! fixture path adds the PDB files (`.pdb`, `.mpdb`) in it in the same way. The fixtures are
//! local inputs: do not commit them or the corpora.
//!
//! The default output directory is `corpus/` in the fuzz package. Each target gets a
//! directory. The name of a seed file is the FNV-1a hash of its bytes, so a second run does not
//! add a copy.

mod native;
mod portable;
mod synthetic;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The most seeds that one PDB gives to one target, to keep the corpora small.
const MAX_SEEDS_PER_PDB: usize = 48;

/// The seeds of all targets, with a cap for each source PDB.
pub(crate) struct Corpus {
    /// The output directory.
    root: PathBuf,
    /// The number of seeds of each target from the current source.
    per_source: BTreeMap<&'static str, usize>,
    /// The number of new seed files of each target.
    written: BTreeMap<&'static str, usize>,
}

impl Corpus {
    /// Write seeds below `root`.
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            per_source: BTreeMap::new(),
            written: BTreeMap::new(),
        }
    }

    /// Start the seeds of a new source PDB.
    fn start_source(&mut self) {
        self.per_source.clear();
    }

    /// Add one seed for `target`, unless the current source has given enough seeds.
    pub(crate) fn add(&mut self, target: &'static str, bytes: &[u8]) -> Outcome {
        let count = self.per_source.entry(target).or_default();
        if *count >= MAX_SEEDS_PER_PDB {
            return Ok(());
        }
        *count += 1;
        let directory = self.root.join(target);
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{:016x}", fnv1a(bytes)));
        if !path.exists() {
            std::fs::write(path, bytes)?;
            *self.written.entry(target).or_default() += 1;
        }
        Ok(())
    }

    /// Add one seed for `target` from parts with a 32-bit length prefix, and then the rest.
    pub(crate) fn add_parts(
        &mut self,
        target: &'static str,
        prefix: &[u8],
        parts: &[&[u8]],
        rest: &[u8],
    ) -> Outcome {
        let mut bytes = prefix.to_vec();
        for part in parts {
            bytes.extend_from_slice(&u32::try_from(part.len())?.to_le_bytes());
            bytes.extend_from_slice(part);
        }
        bytes.extend_from_slice(rest);
        self.add(target, &bytes)
    }
}

/// Return the 64-bit FNV-1a hash of `bytes`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn main() -> Outcome {
    let mut arguments = std::env::args().skip(1);
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let mut fixtures = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--out" {
            root = arguments.next().ok_or("--out needs a directory")?.into();
        } else {
            fixtures.push(PathBuf::from(argument));
        }
    }
    let mut corpus = Corpus::new(root);
    corpus.start_source();
    let native_pdb = synthetic::native_pdb()?;
    native::seeds(&native_pdb, &mut corpus)?;
    corpus.start_source();
    let portable_pdb = synthetic::portable_pdb()?;
    portable::seeds(&portable_pdb, &mut corpus)?;
    corpus.start_source();
    synthetic::extra_seeds(&mut corpus)?;
    let mut files = Vec::new();
    for path in fixtures {
        collect(&path, &mut files)?;
    }
    files.sort();
    for file in &files {
        let bytes = std::fs::read(file)?;
        corpus.start_source();
        let result = if bytes.starts_with(b"Microsoft C/C++ MSF 7.00") {
            native::seeds(&bytes, &mut corpus)
        } else {
            portable::seeds(&bytes, &mut corpus)
        };
        if let Err(error) = result {
            println!("skipped {}: {error}", file.display());
        }
    }
    println!("fixture files: {}", files.len());
    for (target, count) in &corpus.written {
        println!("{target}: {count} new seeds");
    }
    Ok(())
}

/// Add `path` if it is a PDB file, or the PDB files below it if it is a directory.
fn collect(path: &Path, files: &mut Vec<PathBuf>) -> Outcome {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect(&entry?.path(), files)?;
        }
    } else if path
        .extension()
        .is_some_and(|extension| extension == "pdb" || extension == "mpdb")
    {
        files.push(path.to_path_buf());
    }
    Ok(())
}
