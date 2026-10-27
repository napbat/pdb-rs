//! The file version of a Windows executable or DLL.

/// The `dwSignature` and `dwStrucVersion` fields of `VS_FIXEDFILEINFO`, little-endian.
const FIXED_FILE_INFO: [u8; 8] = [0xBD, 0x04, 0xEF, 0xFE, 0x00, 0x00, 0x01, 0x00];

/// Find the `VS_FIXEDFILEINFO` structure of a version resource and return the file version
/// as `major.minor.build.revision`.
///
/// Return `None` if the bytes contain no complete structure.
pub fn file_version(bytes: &[u8]) -> Option<String> {
    let start = bytes
        .windows(FIXED_FILE_INFO.len())
        .position(|window| window == FIXED_FILE_INFO)?;
    let field = |offset: usize| {
        let at = start + offset;
        bytes
            .get(at..at + 4)
            .and_then(|slice| slice.try_into().ok())
            .map(u32::from_le_bytes)
    };
    // `dwFileVersionMS` and `dwFileVersionLS` follow the signature and the structure version.
    let most = field(8)?;
    let least = field(12)?;
    Some(format!(
        "{}.{}.{}.{}",
        most >> 16,
        most & 0xFFFF,
        least >> 16,
        least & 0xFFFF
    ))
}

#[cfg(test)]
mod tests {
    use super::{FIXED_FILE_INFO, file_version};

    fn resource(prefix: &[u8], most: u32, least: u32) -> Vec<u8> {
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(&FIXED_FILE_INFO);
        bytes.extend_from_slice(&most.to_le_bytes());
        bytes.extend_from_slice(&least.to_le_bytes());
        bytes.extend_from_slice(&[0; 36]);
        bytes
    }

    #[test]
    fn reads_the_file_version_fields() {
        let bytes = resource(b"MZ\x90\0 VS_VERSION_INFO", (14 << 16) | 0x33, 36256 << 16);
        assert_eq!(file_version(&bytes).as_deref(), Some("14.51.36256.0"));
    }

    #[test]
    fn needs_the_structure_version_after_the_signature() {
        let mut bytes = resource(b"", (1 << 16) | 2, (3 << 16) | 4);
        bytes[6] = 0x02;
        assert_eq!(file_version(&bytes), None);
    }

    #[test]
    fn rejects_a_truncated_structure() {
        let bytes = resource(b"", 1, 2);
        assert_eq!(file_version(&bytes[..FIXED_FILE_INFO.len() + 6]), None);
        assert_eq!(file_version(b"no resource"), None);
    }
}
