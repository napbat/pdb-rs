//! The index values of the type map streams and of `/UDTSRCLINEUNDONE`.
//!
//! Both streams are arrays of little-endian `u32` values. The rewriters
//! write the output index into each word that holds a TPI or IPI index of
//! this PDB.

use super::map::IndexRemap;
use super::records::{Rewrite, unsupported};
use crate::Result;
use crate::native::auxiliary::{TypeMapKind, TypeMapStream, UdtSourceLineUndone};
use crate::native::msf::StreamId;

/// The size of one word.
const WORD_SIZE: usize = 4;

/// The size of the header of a version 1 type map: the version, the type
/// count and the ID count.
const TYPE_SERVER_HEADER_SIZE: usize = 12;

/// The space of the index in a word.
#[derive(Clone, Copy)]
enum Space {
    /// A TPI index.
    Type,
    /// An IPI index.
    Id,
}

/// Write the output index of the word at `offset` of `bytes`.
///
/// Return `true` if the word changed. Errors of the map get the location
/// of the word in `stream`.
fn remap_word(
    bytes: &mut [u8],
    offset: usize,
    space: Space,
    stream: StreamId,
    map: &impl IndexRemap,
) -> Result<bool> {
    let word = &mut bytes[offset..offset + WORD_SIZE];
    let source = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
    let mut value = source;
    let mut rewrite = Rewrite::new(map);
    match space {
        Space::Type => rewrite.type_value(&mut value),
        Space::Id => rewrite.id_value(&mut value),
    }
    .map_err(|error| error.with_stream(stream, u64::try_from(offset).unwrap_or(u64::MAX)))?;
    word.copy_from_slice(&value.to_le_bytes());
    Ok(value != source)
}

/// Write the output indices into the type map stream `bytes` of `stream`.
///
/// A version 1 map (a type server) holds PDB indices: each value of the
/// type map is a TPI index, each value of the ID map is an IPI index, and
/// the second value of each function ID pair is a TPI index. The first
/// value of a pair is an ID of the type server and stays. The value 0 marks
/// an input index without an output index and stays. Return `true` if a
/// byte changed.
///
/// # Errors
///
/// - The errors of `TypeMapStream::parse`.
/// - The errors of the map, with the location of the word in `stream`.
/// - [`ErrorKind::UnsupportedEdit`](crate::ErrorKind::UnsupportedEdit) at
///   offset 0 for a version 2 map (an object file).
pub(in crate::native::editor::plan) fn type_map(
    bytes: &mut [u8],
    stream: StreamId,
    map: &impl IndexRemap,
) -> Result<bool> {
    let (types, ids, pairs) = {
        let parsed = TypeMapStream::parse(bytes, stream)?;
        match parsed.kind() {
            TypeMapKind::TypeServer => (
                parsed.types().len(),
                parsed.ids().len(),
                parsed.function_types().len(),
            ),
            TypeMapKind::Object { .. } => {
                return Err(unsupported(
                    "a type map of an object file holds TPI and IPI indices that only the \
                     object file tells apart",
                )
                .with_stream(stream, 0));
            }
        }
    };
    let mut changed = false;
    let mut offset = TYPE_SERVER_HEADER_SIZE;
    for space in [Space::Type, Space::Id] {
        let count = match space {
            Space::Type => types,
            Space::Id => ids,
        };
        for _ in 0..count {
            changed |= remap_word(bytes, offset, space, stream, map)?;
            offset += WORD_SIZE;
        }
    }
    for _ in 0..pairs {
        // The first word is a function ID of the type server.
        changed |= remap_word(bytes, offset + WORD_SIZE, Space::Type, stream, map)?;
        offset += 2 * WORD_SIZE;
    }
    Ok(changed)
}

/// Write the output indices into the `/UDTSRCLINEUNDONE` stream `bytes` of
/// `stream`: an array of TPI indices. Return `true` if a byte changed.
///
/// # Errors
///
/// - The errors of `UdtSourceLineUndone::parse`.
/// - The errors of the map, with the location of the word in `stream`.
pub(in crate::native::editor::plan) fn udt_source_line_undone(
    bytes: &mut [u8],
    stream: StreamId,
    map: &impl IndexRemap,
) -> Result<bool> {
    let count = UdtSourceLineUndone::parse(bytes, stream)?.len();
    let mut changed = false;
    for position in 0..count {
        changed |= remap_word(bytes, position * WORD_SIZE, Space::Type, stream, map)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::{type_map, udt_source_line_undone};
    use crate::ErrorKind;
    use crate::native::IndexSpace;
    use crate::native::editor::plan::compact::map::IndexMaps;
    use crate::native::editor::plan::compact::records::tests::{removal, words};
    use crate::native::msf::StreamId;

    /// The stream of the tests.
    const STREAM: StreamId = StreamId(11);

    #[test]
    fn type_server_map_gets_the_output_indices() {
        // Two types, one ID, one function ID pair.
        let mut bytes = words(&[1, 2, 1, 0x1001, 0, 0x1002, 0x1005, 0x1002]);
        assert!(type_map(&mut bytes, STREAM, &removal()).unwrap());
        assert_eq!(bytes, words(&[1, 2, 1, 0x1000, 0, 0x1001, 0x1005, 0x1001]));
    }

    #[test]
    fn type_server_map_with_a_removed_index_fails_at_the_word() {
        let mut bytes = words(&[1, 1, 1, 0x1001, 0x1000]);
        let error = type_map(&mut bytes, STREAM, &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(error.index_space(), Some(IndexSpace::IdIndex));
        assert_eq!(error.stream_location(), Some((STREAM, 16)));
    }

    #[test]
    fn object_file_map_is_unsupported() {
        // Version 2: no precompiled header map, signature, first index,
        // no indices, one record offset.
        let mut bytes = words(&[2, 0xffff, 0, 0x1000, 0, 0]);
        let source = bytes.clone();
        let error = type_map(&mut bytes, STREAM, &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
        assert_eq!(error.stream_location(), Some((STREAM, 0)));
        assert_eq!(bytes, source);
    }

    #[test]
    fn udt_source_lines_get_the_output_indices() {
        let mut bytes = words(&[0x1004, 0x1001]);
        assert!(udt_source_line_undone(&mut bytes, STREAM, &removal()).unwrap());
        assert_eq!(bytes, words(&[0x1003, 0x1000]));
    }

    #[test]
    fn identity_map_changes_nothing() {
        let mut bytes = words(&[1, 1, 0, 0x1001]);
        let source = bytes.clone();
        assert!(!type_map(&mut bytes, STREAM, &IndexMaps::default()).unwrap());
        assert_eq!(bytes, source);
    }
}
