//! Synthetic inputs: a native PDB, a Portable PDB and small inputs of single targets.
//!
//! The data comes from the formats in `docs/design/pdb.md`, not from a compiler. The native
//! PDB has one module with a procedure, line data, types, IDs, a GSI, a PSI and section
//! headers. The Portable PDB has each debug table and each custom debug information kind.

mod msfz;
mod native;
mod portable;

pub use msfz::msfz_chunk_seeds;
pub use native::native_pdb;
pub use portable::portable_pdb;

use pdb_rs::native::msf::{Msf, MsfBuilder, MsfWriteOptions};
use pdb_rs::portable::embedded::{EmbeddedFormat, EmbeddedSource};

use crate::{Corpus, Outcome};

/// Add small synthetic inputs that the PDB splits do not give.
pub fn extra_seeds(corpus: &mut Corpus) -> Outcome {
    msfz::extra_seeds(corpus)?;
    let text = b"class C { static void Main() { } }\n".repeat(4);
    for format in [EmbeddedFormat::Raw, EmbeddedFormat::Deflate] {
        let mut blob = Vec::new();
        EmbeddedSource::encode_parts(&text, format, &mut blob)?;
        corpus.add("embedded_source", &blob)?;
    }
    corpus.add("mpdb", &pdb_rs::portable::embedded::encode_embedded(&text)?)?;
    // A container with three streams, one of them nil, at two page sizes.
    let streams: [&[u8]; 3] = [b"", &[0x5a; 700], b"stream two"];
    for page_size in [512, 4096] {
        let mut builder = MsfBuilder::new();
        for stream in streams {
            builder.push(stream)?;
        }
        builder.push_nil()?;
        let bytes = builder
            .plan(MsfWriteOptions::default().with_page_size(page_size))?
            .to_vec()?;
        Msf::read(bytes.as_slice())?;
        corpus.add("msf", &bytes)?;
        corpus.add_parts("msf_repack", &[0x08], &[], &bytes)?;
        corpus.add_parts("msfz_convert", &[0x2c], &[], &bytes)?;
    }
    // The builder target reads a page size selector, then tagged streams.
    let mut build = vec![0];
    for stream in streams {
        build.push(0);
        build.extend_from_slice(&u16::try_from(stream.len())?.to_le_bytes());
        build.extend_from_slice(stream);
    }
    build.push(0x80);
    corpus.add("msf_build", &build)?;
    // One FPO record (16 bytes) and one frame data record (32 bytes).
    let fpo: Vec<u8> = [0x1000_u32, 0x20, 0, 1]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    corpus.add("native_frames", &fpo)?;
    let frame: Vec<u8> = [0x1000_u32, 0x20, 0, 0, 8, 0, 4, 0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    corpus.add("native_frames", &frame)?;
    // Compressed integers of each size, and a split document name.
    for bytes in [
        &[0x03_u8][..],
        &[0x80, 0x80],
        &[0xc0, 0x00, 0x40, 0x00],
        &[0xff],
    ] {
        corpus.add_parts("portable_blobs", &[4], &[], bytes)?;
    }
    corpus.add_parts("portable_blobs", &[5], &[], b"C:\\src\\Program.cs")?;
    // Builder commands: a document, a method with two sequence points, an import scope, a
    // local scope with a variable, and a custom debug information value.
    let mut commands = vec![0, 6];
    commands.extend_from_slice(b"/a.cs\0");
    commands.extend_from_slice(&[1, 7, 2, 32, 0, 1, 0]);
    commands.extend_from_slice(&[1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    commands.extend_from_slice(&[1, 4, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0]);
    commands.extend_from_slice(&[5, 1, 0, 6]);
    commands.extend_from_slice(b"System");
    commands.extend_from_slice(&[0, 0, 0, 0, 0]);
    commands.extend_from_slice(&[2, 0, 0, 0, 12, 0, 3, 0, 0, 1, 0, 1, b'x']);
    commands.extend_from_slice(&[6, 1, 0x11, 2, b'h', b'i']);
    corpus.add("portable_build", &commands)?;
    // Range sets of `native_remap` (see `pdb_rs_fuzz::native::remap`): `.text` at its old RVA;
    // `.text` moved; `sum` after `main`, with `main` split by a gap; `sum` removed; the block
    // of `main` split by a gap.
    for input in [
        &[0x00_u8][..],
        &[0x01],
        &[0x0d, 0x1f, 0x04, 0x00, 0x17, 0x00, 0x85, 0x17, 0x08, 0x00],
        &[0x02, 0x1f, 0x03, 0x00],
        &[0x00, 0x2f, 0x00, 0x00, 0x3f, 0x00, 0x81],
    ] {
        corpus.add("native_remap", input)?;
    }
    Ok(())
}
