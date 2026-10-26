//! Tests for positional byte access.

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;

const BYTES: [u8; 12] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
];

fn eof_offset<R: ReadAt + ?Sized>(input: &R, offset: u64, count: usize) -> Option<u64> {
    let mut buffer = vec![0; count];
    let error = input.read_exact_at(offset, &mut buffer).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    error.offset()
}

#[test]
fn slice_reads_exact_ranges() {
    let input = &BYTES[..];
    let mut buffer = [0; 3];
    input.read_exact_at(0, &mut buffer).unwrap();
    assert_eq!(buffer, [0x01, 0x02, 0x03]);
    input.read_exact_at(9, &mut buffer).unwrap();
    assert_eq!(buffer, [0x0a, 0x0b, 0x0c]);

    let mut all = [0; 12];
    input.read_exact_at(0, &mut all).unwrap();
    assert_eq!(all, BYTES);
}

#[test]
fn slice_reads_little_endian_values() {
    let input = &BYTES[..];
    assert_eq!(input.read_u16_at(0).unwrap(), 0x0201);
    assert_eq!(input.read_u16_at(10).unwrap(), 0x0c0b);
    assert_eq!(input.read_u32_at(1).unwrap(), 0x0504_0302);
    assert_eq!(input.read_u32_at(8).unwrap(), 0x0c0b_0a09);
    assert_eq!(input.read_u64_at(3).unwrap(), 0x0b0a_0908_0706_0504);
    assert_eq!(input.read_u64_at(4).unwrap(), 0x0c0b_0a09_0807_0605);
}

#[test]
fn slice_rejects_values_across_the_end() {
    let input = &BYTES[..];
    assert_eq!(input.read_u16_at(11).unwrap_err().offset(), Some(12));
    assert_eq!(input.read_u32_at(9).unwrap_err().offset(), Some(12));
    let error = input.read_u64_at(5).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.offset(), Some(12));
}

#[test]
fn empty_read_is_valid_at_the_end_only() {
    let input = &BYTES[..];
    input.read_exact_at(0, &mut []).unwrap();
    input.read_exact_at(12, &mut []).unwrap();
    assert_eq!(eof_offset(input, 13, 0), Some(13));
    assert_eq!(eof_offset(input, u64::MAX, 0), Some(u64::MAX));

    let empty: &[u8] = &[];
    empty.read_exact_at(0, &mut []).unwrap();
    assert_eq!(eof_offset(empty, 1, 0), Some(1));
}

#[test]
fn eof_reports_first_missing_byte() {
    let input = &BYTES[..];
    assert_eq!(eof_offset(input, 10, 3), Some(12));
    assert_eq!(eof_offset(input, 0, 13), Some(12));
    assert_eq!(eof_offset(input, 12, 1), Some(12));
    assert_eq!(eof_offset(input, 20, 1), Some(20));
    assert_eq!(eof_offset(input, u64::MAX - 1, 1), Some(u64::MAX - 1));
}

#[test]
fn range_end_overflow_is_rejected() {
    let input = &BYTES[..];
    let mut buffer = [0; 2];
    let error = input.read_exact_at(u64::MAX, &mut buffer).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::OffsetOverflow);
    assert_eq!(error.offset(), Some(u64::MAX));

    let error = input.read_u64_at(u64::MAX - 6).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::OffsetOverflow);
    assert_eq!(error.offset(), Some(u64::MAX - 6));

    // A range that ends exactly at `u64::MAX` does not overflow.
    let error = input.read_u64_at(u64::MAX - 8).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[cfg(all(feature = "std", any(unix, windows)))]
mod file {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;

    use pdbrs::ErrorKind;
    use pdbrs::io::{FileReader, ReadAt};

    use std::sync::atomic::{AtomicU64, Ordering};

    const FIXTURE: [u8; 12] = super::BYTES;
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    /// An owned temporary directory and its input path.
    struct TempFile(PathBuf, PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            loop {
                let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let directory = std::env::temp_dir()
                    .join(format!("pdbrs-io-{}-{name}-{serial}", std::process::id()));
                match fs::create_dir(&directory) {
                    Ok(()) => return Self(directory.join("input"), directory),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("cannot create test directory: {error}"),
                }
            }
        }

        fn fixture() -> Self {
            let temp = Self::new("fixture");
            fs::write(&temp.0, FIXTURE).unwrap();
            temp
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            drop(fs::remove_file(&self.0));
            drop(fs::remove_dir(&self.1));
        }
    }

    #[test]
    fn reads_file_at_independent_offsets() {
        let temp = TempFile::fixture();
        let reader = FileReader::open(&temp.0).unwrap();
        assert_eq!(reader.len().unwrap(), 12);
        assert_eq!(reader.read_u32_at(8).unwrap(), 0x0c0b_0a09);
        assert_eq!(reader.read_u16_at(0).unwrap(), 0x0201);
        assert_eq!(reader.read_u32_at(1).unwrap(), 0x0504_0302);
    }

    #[test]
    fn rejects_ranges_after_captured_length() {
        let temp = TempFile::fixture();
        let reader = FileReader::new(File::open(&temp.0).unwrap()).unwrap();
        let len = reader.len().unwrap();
        reader.read_exact_at(len, &mut []).unwrap();

        let mut buffer = [0; 4];
        let error = reader.read_exact_at(len - 2, &mut buffer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(error.offset(), Some(len));

        let error = reader.read_exact_at(u64::MAX - 1, &mut buffer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::OffsetOverflow);
        assert_eq!(error.offset(), Some(u64::MAX - 1));
    }

    #[test]
    fn length_is_captured_at_construction() {
        let temp = TempFile::new("captured-length");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp.0)
            .unwrap();
        file.write_all(&[1, 2, 3, 4]).unwrap();
        let reader = FileReader::new(file.try_clone().unwrap()).unwrap();
        file.write_all(&[5, 6, 7, 8]).unwrap();

        assert_eq!(reader.len().unwrap(), 4);
        let error = reader.read_u64_at(0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(error.offset(), Some(4));
    }

    #[test]
    fn truncated_file_reports_eof_at_missing_byte() {
        let temp = TempFile::new("truncated");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp.0)
            .unwrap();
        (&file).write_all(&[0x11; 16]).unwrap();
        let reader = FileReader::new(file.try_clone().unwrap()).unwrap();
        file.set_len(6).unwrap();

        assert_eq!(reader.len().unwrap(), 16);
        let mut buffer = [0; 8];
        let error = reader.read_exact_at(2, &mut buffer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(error.offset(), Some(6));
    }

    #[test]
    fn io_failure_keeps_source_error() {
        let temp = TempFile::new("write-only");
        fs::write(&temp.0, [0x22; 8]).unwrap();
        let file = OpenOptions::new().write(true).open(&temp.0).unwrap();

        // Some platforms reject the metadata request of a write-only handle.
        let error = match FileReader::new(file) {
            Ok(reader) => {
                let error = reader.read_u32_at(2).unwrap_err();
                assert_eq!(error.offset(), Some(2));
                error
            }
            Err(error) => {
                assert_eq!(error.offset(), None);
                error
            }
        };
        assert_eq!(error.kind(), ErrorKind::Io);
        let source = std::error::Error::source(&error).unwrap();
        assert!(source.is::<std::io::Error>());
    }

    #[test]
    fn open_failure_keeps_source_error() {
        let temp = TempFile::new("missing");
        let error = FileReader::open(&temp.0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Io);
        assert_eq!(error.offset(), None);
        let source = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn unix_reads_keep_the_cursor() {
        use std::io::{Seek, SeekFrom};

        let temp = TempFile::fixture();
        let mut file = File::open(&temp.0).unwrap();
        file.seek(SeekFrom::Start(3)).unwrap();
        let reader = FileReader::new(file).unwrap();
        reader.read_u64_at(4).unwrap();
        let mut file = reader.into_inner();
        assert_eq!(file.stream_position().unwrap(), 3);
    }
}
