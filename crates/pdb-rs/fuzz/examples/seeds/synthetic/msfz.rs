//! Seeds of the MSFZ targets: chunks of each compression code and containers around them.
//!
//! `pdb-rs` has no public zstd or raw DEFLATE encoder, so the synthetic chunks use the simplest
//! frames: a zstd frame with one raw block and a raw DEFLATE stream with one stored block. The
//! chunks of PDZ fixtures give compressed seeds.

use pdb_rs_fuzz::msfz::{chunk_container, chunk_input};

use crate::{Corpus, Outcome};

/// The decompressed bytes of a synthetic chunk.
fn data() -> Vec<u8> {
    b"MSFZ chunk seed: stream bytes in the virtual byte array. "
        .iter()
        .copied()
        .cycle()
        .take(200)
        .collect()
}

/// Return a zstd frame with one raw block of `data` (at most 255 bytes).
fn zstd_raw(data: &[u8]) -> Outcome<Vec<u8>> {
    let length = u8::try_from(data.len())?;
    // The magic number, a single-segment frame header with a 1-byte content size, and a raw
    // block header with the last-block bit.
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, length];
    let header = 1_u32 | (u32::from(length) << 3);
    frame.extend_from_slice(&header.to_le_bytes()[..3]);
    frame.extend_from_slice(data);
    Ok(frame)
}

/// Return a raw DEFLATE stream with one stored block of `data`.
fn deflate_stored(data: &[u8]) -> Outcome<Vec<u8>> {
    let length = u16::try_from(data.len())?;
    let mut stream = vec![0x01];
    stream.extend_from_slice(&length.to_le_bytes());
    stream.extend_from_slice(&(!length).to_le_bytes());
    stream.extend_from_slice(data);
    Ok(stream)
}

/// Add the synthetic seeds of `msfz_chunks` and `msfz`.
pub fn extra_seeds(corpus: &mut Corpus) -> Outcome {
    let data = data();
    let size = u32::try_from(data.len())?;
    let zstd = zstd_raw(&data)?;
    let deflate = deflate_stored(&data)?;
    for (code, stored) in [(0, data.as_slice()), (1, &zstd), (2, &deflate)] {
        let input = chunk_input(code, size, stored);
        corpus.add("msfz_chunks", &input)?;
        corpus.add("msfz", &chunk_container(&input))?;
    }
    // Two chunks (zstd, then raw DEFLATE) and a fragment from the middle of the first chunk.
    let mut input = vec![0x10 | 1 | (2 << 2)];
    for value in [size, size, size / 2, u32::try_from(zstd.len())?] {
        input.extend_from_slice(&value.to_le_bytes());
    }
    input.extend_from_slice(&zstd);
    input.extend_from_slice(&deflate);
    corpus.add("msfz_chunks", &input)?;
    corpus.add("msfz", &chunk_container(&input))?;
    Ok(())
}

/// Add each chunk of the MSFZ container in `bytes` as a `msfz_chunks` seed.
pub fn msfz_chunk_seeds(bytes: &[u8], corpus: &mut Corpus) -> Outcome {
    let u32_at = |offset: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    };
    let u64_at = |offset: usize| -> Option<u64> {
        Some(u64::from_le_bytes(
            bytes.get(offset..offset + 8)?.try_into().ok()?,
        ))
    };
    let (Some(table), Some(count)) = (u64_at(48), u32_at(72)) else {
        return Ok(());
    };
    let table = usize::try_from(table)?;
    for index in 0..usize::try_from(count)? {
        let entry = table + index * 20;
        let (Some(offset), Some(code), Some(stored), Some(size)) = (
            u64_at(entry),
            u32_at(entry + 8),
            u32_at(entry + 12),
            u32_at(entry + 16),
        ) else {
            break;
        };
        let start = usize::try_from(offset)?;
        let Some(data) = bytes.get(start..start + usize::try_from(stored)?) else {
            break;
        };
        corpus.add("msfz_chunks", &chunk_input(u8::try_from(code)?, size, data))?;
    }
    Ok(())
}
