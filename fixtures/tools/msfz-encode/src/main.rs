//! `msfz-encode`: write an MSFZ (PDZ) file from an MSF 7.0 PDB with the `ms-pdb-msfz` writer.
//!
//! The pdbrs fixture generator uses this tool to make PDZ fixtures with forms that
//! `pdbtool pdz-encode` does not write: raw DEFLATE chunks, a raw DEFLATE stream directory and
//! files with only uncompressed fragments. The tool reads the input with `ms-pdb-msf` and does
//! not use pdbrs.
//!
//! The tool copies each stream from stream 1. Stream 0 (the previous MSF stream directory) is
//! an empty stream, as `pdbtool pdz-encode` writes it. A nil stream stays a nil stream.

use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use ms_pdb_msf::Msf;
use ms_pdb_msfz::{
    Compression, DEFAULT_CHUNK_THRESHOLD, MIN_FILE_SIZE_16K, MsfzFinishOptions, MsfzWriter,
    StreamWriter,
};

/// The usage text.
const USAGE: &str = "\
usage: msfz-encode [options] <input.pdb> <output.pdb>
       msfz-encode --version

  --chunk-codec <zstd|deflate>
                 The codec of the chunks. The default is zstd.
  --stream-dir-codec <none|zstd|deflate>
                 The codec of the stream directory. The default is none.
  --uncompressed Write each stream as uncompressed fragments. The file then has no chunks.
  --max-chunk-size <bytes>
                 The maximum uncompressed size of a chunk. A K or M suffix multiplies by
                 1024 or 1048576. The writer clamps the value to 4K..1G. The default is 1M.
  --pad16k       Make the file at least 16 KiB long.";

/// The error type of the tool.
type Error = Box<dyn std::error::Error + Send + Sync>;

/// The options of one conversion.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    input: PathBuf,
    output: PathBuf,
    chunk_codec: Compression,
    stream_dir_codec: Option<Compression>,
    uncompressed: bool,
    max_chunk_size: u32,
    pad16k: bool,
}

/// What the tool must do.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Print the version.
    Version,
    /// Convert a file.
    Encode(Options),
}

/// The facts of a written file.
struct Written {
    streams: u32,
    chunks: u32,
    bytes: u64,
}

fn main() -> ExitCode {
    let action = match parse(std::env::args().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("msfz-encode: {error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match action {
        Action::Version => {
            println!("msfz-encode {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Action::Encode(options) => match encode(&options) {
            Ok(written) => {
                println!(
                    "{}: {} streams, {} chunks, {} bytes",
                    options.output.display(),
                    written.streams,
                    written.chunks,
                    written.bytes
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("msfz-encode: {}: {error}", options.input.display());
                ExitCode::FAILURE
            }
        },
    }
}

/// Parse the arguments that follow the program name.
fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Action, String> {
    let mut chunk_codec = Compression::Zstd;
    let mut stream_dir_codec = None;
    let mut uncompressed = false;
    let mut max_chunk_size = DEFAULT_CHUNK_THRESHOLD;
    let mut pad16k = false;
    let mut files = Vec::new();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("`{argument}` needs a value"))
        };
        match argument.as_str() {
            "--version" => return Ok(Action::Version),
            "--chunk-codec" => chunk_codec = codec(&value()?)?,
            "--stream-dir-codec" => {
                let name = value()?;
                stream_dir_codec = if name == "none" {
                    None
                } else {
                    Some(codec(&name)?)
                };
            }
            "--uncompressed" => uncompressed = true,
            "--max-chunk-size" => max_chunk_size = size(&value()?)?,
            "--pad16k" => pad16k = true,
            option if option.starts_with("--") => {
                return Err(format!("unknown option `{option}`"));
            }
            _ => files.push(PathBuf::from(argument)),
        }
    }
    let [input, output] = <[PathBuf; 2]>::try_from(files)
        .map_err(|files| format!("give one input and one output, not {} files", files.len()))?;
    Ok(Action::Encode(Options {
        input,
        output,
        chunk_codec,
        stream_dir_codec,
        uncompressed,
        max_chunk_size,
        pad16k,
    }))
}

/// Parse a codec name.
fn codec(name: &str) -> Result<Compression, String> {
    match name {
        "zstd" => Ok(Compression::Zstd),
        "deflate" => Ok(Compression::Deflate),
        _ => Err(format!("unknown codec `{name}`")),
    }
}

/// Parse a size in bytes with an optional `K` or `M` suffix.
fn size(text: &str) -> Result<u32, String> {
    let (digits, unit) = match text.as_bytes().last() {
        Some(b'K' | b'k') => (&text[..text.len() - 1], 1 << 10),
        Some(b'M' | b'm') => (&text[..text.len() - 1], 1 << 20),
        _ => (text, 1),
    };
    digits
        .parse::<u32>()
        .ok()
        .and_then(|value| value.checked_mul(unit))
        .ok_or_else(|| format!("`{text}` is not a size in bytes"))
}

/// Read the MSF 7.0 file and write the MSFZ file with `options`.
fn encode(options: &Options) -> Result<Written, Error> {
    let msf = Msf::open(&options.input)?;
    let mut writer = MsfzWriter::create(&options.output)?;
    writer.set_chunk_compression_mode(options.chunk_codec);
    writer.set_uncompressed_chunk_size_threshold(options.max_chunk_size);
    let count = msf.num_streams();
    writer.reserve_num_streams(usize::try_from(count)?);
    let mut data = Vec::new();
    for stream in 1..count {
        // `is_stream_valid` is `false` for a nil stream. The writer keeps it nil.
        if !msf.is_stream_valid(stream) {
            continue;
        }
        msf.read_stream_to_vec_mut(stream, &mut data)?;
        let mut stream_writer = writer.stream_writer(stream)?;
        if options.uncompressed {
            stream_writer.set_compression_enabled(false);
            stream_writer.write_all(&data)?;
        } else {
            write_chunked(&mut stream_writer, &data)?;
        }
    }
    let (summary, mut file) = writer.finish_with_options(MsfzFinishOptions {
        min_file_size: if options.pad16k { MIN_FILE_SIZE_16K } else { 0 },
        stream_dir_compression: options.stream_dir_codec,
    })?;
    let bytes = file.seek(SeekFrom::End(0))?;
    Ok(Written {
        streams: summary.num_streams,
        chunks: summary.num_chunks,
        bytes,
    })
}

/// Write `data` to chunks and fill each chunk to the maximum chunk size.
///
/// The writer puts all bytes of one write call into one chunk. It ends the current chunk
/// first if the call fills the chunk buffer. Thus each call writes at most one byte less than
/// the free space, and a large stream gets one fragment in each chunk.
fn write_chunked<F: Write + Seek>(
    writer: &mut StreamWriter<'_, F>,
    mut data: &[u8],
) -> std::io::Result<()> {
    while !data.is_empty() {
        let room = writer.bytes_available_in_chunk_buffer();
        if room <= 1 {
            writer.end_chunk()?;
            continue;
        }
        let (part, rest) = data.split_at(data.len().min(room - 1));
        writer.write_all(part)?;
        data = rest;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Action, Compression, Msf, Options, parse};
    use ms_pdb_msf::CreateOptions;
    use ms_pdb_msfz::{COMPRESSION_DEFLATE, COMPRESSION_NONE, COMPRESSION_ZSTD, Msfz};
    use std::path::{Path, PathBuf};

    /// Return the path of `name` in a new scratch directory for a test.
    fn scratch(test: &str, name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("msfz-encode-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(name)
    }

    /// Return the codec code of the stream directory from the MSFZ file header.
    fn stream_dir_code(path: &Path) -> u32 {
        let bytes = std::fs::read(path).unwrap();
        u32::from_le_bytes(bytes[60..64].try_into().unwrap())
    }

    fn arguments(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn options(input: &str, output: &str) -> Options {
        Options {
            input: PathBuf::from(input),
            output: PathBuf::from(output),
            chunk_codec: Compression::Zstd,
            stream_dir_codec: None,
            uncompressed: false,
            max_chunk_size: 0x10_0000,
            pad16k: false,
        }
    }

    #[test]
    fn parses_the_options() {
        assert_eq!(
            parse(arguments("a.pdb b.pdb")),
            Ok(Action::Encode(options("a.pdb", "b.pdb")))
        );
        let mut expected = options("a.pdb", "b.pdb");
        expected.chunk_codec = Compression::Deflate;
        expected.stream_dir_codec = Some(Compression::Deflate);
        expected.uncompressed = true;
        expected.max_chunk_size = 4096;
        expected.pad16k = true;
        assert_eq!(
            parse(arguments(
                "--chunk-codec deflate --stream-dir-codec deflate --uncompressed \
                 --max-chunk-size 4K --pad16k a.pdb b.pdb"
            )),
            Ok(Action::Encode(expected))
        );
        assert_eq!(parse(arguments("--version")), Ok(Action::Version));
    }

    #[test]
    fn rejects_bad_arguments() {
        for (text, message) in [
            ("a.pdb", "not 1 files"),
            ("--chunk-codec lz4 a b", "unknown codec `lz4`"),
            ("--stream-dir-codec", "needs a value"),
            ("--max-chunk-size 5G a b", "not a size"),
            ("--max-chunk-size 4194304M a b", "not a size"),
            ("--fast a b", "unknown option `--fast`"),
        ] {
            let error = parse(arguments(text)).unwrap_err();
            assert!(error.contains(message), "{text}: {error}");
        }
    }

    /// The streams that the test adds: a small stream, a large stream, a nil stream and an
    /// empty stream, with their stream indexes.
    type Added = Vec<(u32, Option<Vec<u8>>)>;

    /// Write an MSF 7.0 file with the streams of [`Added`] after the fixed streams.
    fn source(test: &str) -> (PathBuf, Added) {
        let path = scratch(test, "source.pdb");
        let large: Vec<u8> = (0..20_000u32).map(|value| value.to_le_bytes()[0]).collect();
        let mut msf = Msf::create(&path, CreateOptions::default()).unwrap();
        let added = [Some(b"info".to_vec()), Some(large), None, Some(Vec::new())]
            .into_iter()
            .map(|stream| match stream {
                Some(data) => (msf.new_stream_data(&data).unwrap(), Some(data)),
                None => (msf.nil_stream().unwrap(), None),
            })
            .collect();
        msf.commit().unwrap();
        (path, added)
    }

    /// Encode the source with `options` and check the streams with the `ms-pdb-msfz` reader.
    ///
    /// Return the output path, the reader and the indexes of the added streams.
    fn round_trip(test: &str, change: impl FnOnce(&mut Options)) -> (PathBuf, Msfz, Vec<u32>) {
        let (input, added) = source(test);
        let output = scratch(test, "output.pdb");
        let mut options = options("", "");
        options.input.clone_from(&input);
        options.output.clone_from(&output);
        options.max_chunk_size = 4096;
        change(&mut options);
        let written = super::encode(&options).unwrap();
        let msf = Msf::open(&input).unwrap();
        let msfz = Msfz::open(&output).unwrap();
        assert_eq!(written.streams, msfz.num_streams());
        assert_eq!(msfz.num_streams(), msf.num_streams());
        assert_eq!(msfz.read_stream(0).unwrap().as_slice(), b"");
        for stream in 1..msf.num_streams() {
            if msf.is_stream_valid(stream) {
                let data = msf.read_stream_to_vec(stream).unwrap();
                assert_eq!(msfz.read_stream(stream).unwrap().as_slice(), data);
            } else {
                assert!(!msfz.is_stream_valid(stream), "{stream}");
            }
        }
        for (stream, expected) in &added {
            match expected {
                Some(data) => assert_eq!(msfz.read_stream(*stream).unwrap().as_slice(), data),
                None => assert!(!msfz.is_stream_valid(*stream)),
            }
        }
        let indexes = added.iter().map(|(stream, _)| *stream).collect();
        (output, msfz, indexes)
    }

    #[test]
    fn writes_deflate_chunks_across_a_large_stream() {
        let (output, msfz, _) = round_trip("deflate", |options| {
            options.chunk_codec = Compression::Deflate;
        });
        assert!(msfz.chunks().len() >= 5, "{}", msfz.chunks().len());
        for chunk in msfz.chunks() {
            assert_eq!(chunk.compression.get(), COMPRESSION_DEFLATE);
            assert!(chunk.uncompressed_size.get() < 4096);
        }
        assert_eq!(stream_dir_code(&output), COMPRESSION_NONE);
    }

    #[test]
    fn writes_a_deflate_stream_directory() {
        let (output, msfz, _) = round_trip("deflate-dir", |options| {
            options.stream_dir_codec = Some(Compression::Deflate);
        });
        assert!(
            msfz.chunks()
                .iter()
                .all(|chunk| chunk.compression.get() == COMPRESSION_ZSTD)
        );
        assert_eq!(stream_dir_code(&output), COMPRESSION_DEFLATE);
    }

    #[test]
    fn writes_only_uncompressed_fragments() {
        let (_, msfz, added) = round_trip("uncompressed", |options| options.uncompressed = true);
        assert!(msfz.chunks().is_empty());
        // The small stream, the large stream and the empty stream; the third one is nil.
        for (stream, fragments) in [(added[0], 1), (added[1], 1), (added[3], 0)] {
            let found = msfz.stream_fragments(stream).unwrap();
            assert_eq!(found.len(), fragments, "{stream}");
            assert!(
                found
                    .iter()
                    .all(|fragment| !fragment.location.is_compressed())
            );
        }
    }

    #[test]
    fn pads_to_16_kib() {
        let (output, _, _) = round_trip("pad16k", |options| options.pad16k = true);
        assert!(std::fs::metadata(output).unwrap().len() >= 0x4000);
    }
}
