//! Behavior of seekable input adapters in both feature modes.

use pdbrs::ErrorKind;
use pdbrs::io::{Cursor, IoError, Read, ReadAt, Seek, SeekFrom, SeekReader};

#[test]
fn positional_reads_ignore_initial_and_previous_positions() {
    let mut cursor = Cursor::new([1, 2, 3, 4, 5, 6, 7, 8]);
    cursor.set_position(5);
    let reader = SeekReader::new(cursor).unwrap();
    assert_eq!(reader.into_inner().position(), 5);

    let reader = SeekReader::new(Cursor::new([1, 2, 3, 4, 5, 6, 7, 8])).unwrap();
    assert_eq!(reader.read_u32_at(4).unwrap(), 0x0807_0605);
    assert_eq!(reader.read_u16_at(0).unwrap(), 0x0201);
    reader.read_exact_at(8, &mut []).unwrap();
    let error = reader.read_exact_at(9, &mut []).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.offset(), Some(9));
    let error = reader.read_u32_at(u64::MAX).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::OffsetOverflow);
    assert_eq!(reader.into_inner().position(), 2);
}

struct Chunked {
    cursor: Cursor<[u8; 8]>,
    interrupted: bool,
    limit: u64,
    fail_seek: bool,
}

impl Chunked {
    fn new(limit: u64) -> Self {
        Self {
            cursor: Cursor::new([1, 2, 3, 4, 5, 6, 7, 8]),
            interrupted: true,
            limit,
            fail_seek: false,
        }
    }
}

impl Read for Chunked {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        if self.interrupted {
            self.interrupted = false;
            return Err(nostdio::interrupted());
        }
        let available = usize::try_from(self.limit.saturating_sub(self.cursor.position())).unwrap();
        let count = buffer.len().min(2).min(available);
        self.cursor.read(&mut buffer[..count])
    }
}

impl Seek for Chunked {
    fn seek(&mut self, position: SeekFrom) -> Result<u64, IoError> {
        if self.fail_seek && position == SeekFrom::Start(2) {
            return Err(nostdio::invalid_seek());
        }
        self.cursor.seek(position)
    }
}

#[test]
fn short_and_interrupted_reads_complete_at_the_requested_offset() {
    let reader = SeekReader::new(Chunked::new(8)).unwrap();
    assert_eq!(reader.read_u32_at(3).unwrap(), 0x0706_0504);
}

#[test]
fn premature_end_reports_progress_and_preserves_typed_source() {
    let reader = SeekReader::new(Chunked::new(5)).unwrap();
    let error = reader.read_u64_at(0).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.offset(), Some(5));
    let source = core::error::Error::source(&error)
        .unwrap()
        .downcast_ref::<IoError>()
        .unwrap();
    assert!(nostdio::is_unexpected_eof(source));
}

#[test]
fn seek_failure_keeps_the_requested_offset_and_source() {
    let mut input = Chunked::new(8);
    input.fail_seek = true;
    let reader = SeekReader::new(input).unwrap();
    let error = reader.read_u32_at(2).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(error.offset(), Some(2));
    let source = core::error::Error::source(&error)
        .unwrap()
        .downcast_ref::<IoError>()
        .unwrap();
    #[cfg(feature = "std")]
    assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
    #[cfg(not(feature = "std"))]
    assert_eq!(*source, IoError::InvalidSeek);
}
