//! Classify build outputs by their contents, not by their file names.

use std::ops::Range;

/// The signature at the start of an MSF 7.0 file.
pub const MSF7_SIGNATURE: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// The signature at the start of CLI metadata and of a Portable PDB.
pub const METADATA_SIGNATURE: &[u8] = b"BSJB";

/// The signature at the start of an embedded Portable PDB debug payload.
pub const EMBEDDED_SIGNATURE: &[u8] = b"MPDB";

/// The debug directory type of a `CodeView` record.
const DEBUG_TYPE_CODEVIEW: u32 = 2;

/// The debug directory type of an embedded Portable PDB.
const DEBUG_TYPE_EMBEDDED_PORTABLE_PDB: u32 = 17;

/// The offset of the PDB path in an `RSDS` `CodeView` record, after the signature, the GUID and
/// the age.
const RSDS_PATH_OFFSET: usize = 24;

/// The kind of a build output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An MSF 7.0 (native) PDB.
    NativePdb,
    /// A Portable PDB.
    PortablePdb,
    /// A PE image with an embedded Portable PDB debug payload.
    EmbeddedInImage,
    /// The `MPDB` payload of an embedded Portable PDB, taken from its image.
    EmbeddedPdbPayload,
    /// A PE image without an embedded Portable PDB.
    Image,
    /// Data with none of the known signatures.
    Unknown,
}

impl Kind {
    /// The name in the manifest.
    pub fn name(self) -> &'static str {
        match self {
            Self::NativePdb => "native-pdb",
            Self::PortablePdb => "portable-pdb",
            Self::EmbeddedInImage => "embedded-in-image",
            Self::EmbeddedPdbPayload => "embedded-pdb-payload",
            Self::Image => "image",
            Self::Unknown => "unknown",
        }
    }
}

/// Classify the contents of a file.
pub fn classify(bytes: &[u8]) -> Kind {
    if bytes.starts_with(MSF7_SIGNATURE) {
        Kind::NativePdb
    } else if bytes.starts_with(METADATA_SIGNATURE) {
        Kind::PortablePdb
    } else if bytes.starts_with(EMBEDDED_SIGNATURE) {
        Kind::EmbeddedPdbPayload
    } else if bytes.starts_with(b"MZ") {
        match embedded_pdb_payload(bytes) {
            Some(_) => Kind::EmbeddedInImage,
            None => Kind::Image,
        }
    } else {
        Kind::Unknown
    }
}

/// Find the `MPDB` payload of an image through the PE debug directory.
///
/// The payload is the data of the first debug directory entry of type 17. Its size is the
/// `SizeOfData` field of the entry.
pub fn embedded_pdb_payload(image: &[u8]) -> Option<&[u8]> {
    let (debug_offset, debug_size) = data_directory(image, 6)?;
    (0..debug_size / 28).find_map(|index| {
        let entry = debug_offset + index * 28;
        if read_u32(image, entry + 12)? != DEBUG_TYPE_EMBEDDED_PORTABLE_PDB {
            return None;
        }
        let size = to_usize(read_u32(image, entry + 16)?)?;
        let payload = to_usize(read_u32(image, entry + 24)?)?;
        let data = image.get(payload..payload.checked_add(size)?)?;
        data.starts_with(EMBEDDED_SIGNATURE).then_some(data)
    })
}

/// Find the PDB path of the `RSDS` `CodeView` record of an image through the PE debug directory.
///
/// Return the file offsets of the path bytes, without the terminating NUL byte. The record is
/// the data of the first debug directory entry of type 2 that starts with `RSDS` and has a NUL
/// byte after the path.
pub fn codeview_pdb_path(image: &[u8]) -> Option<Range<usize>> {
    let (debug_offset, debug_size) = data_directory(image, 6)?;
    (0..debug_size / 28).find_map(|index| {
        let entry = debug_offset + index * 28;
        if read_u32(image, entry + 12)? != DEBUG_TYPE_CODEVIEW {
            return None;
        }
        let size = to_usize(read_u32(image, entry + 16)?)?;
        let start = to_usize(read_u32(image, entry + 24)?)?;
        let record = image.get(start..start.checked_add(size)?)?;
        let path = record.get(RSDS_PATH_OFFSET..)?;
        if !record.starts_with(b"RSDS") {
            return None;
        }
        let length = path.iter().position(|&byte| byte == 0)?;
        let path_start = start + RSDS_PATH_OFFSET;
        Some(path_start..path_start + length)
    })
}

/// Find the CLI metadata (the `BSJB` metadata root and its streams) of a .NET image.
///
/// The CLI header is data directory 14. Its `MetaData` field (offset 8) gives the RVA and the
/// size of the metadata.
pub fn cli_metadata(image: &[u8]) -> Option<&[u8]> {
    let (header, size) = data_directory(image, 14)?;
    if size < 16 {
        return None;
    }
    let (rva, size) = (read_u32(image, header + 8)?, read_u32(image, header + 12)?);
    let start = pe_offset(image, rva)?;
    image.get(start..start.checked_add(to_usize(size)?)?)
}

/// Return the file offset and the size of data directory `index` of a PE image.
fn data_directory(image: &[u8], index: usize) -> Option<(usize, usize)> {
    let (directories, _, _) = pe_layout(image)?;
    let directory_count = to_usize(read_u32(image, directories - 4)?)?;
    if directory_count <= index {
        return None;
    }
    let rva = read_u32(image, directories + index * 8)?;
    let size = to_usize(read_u32(image, directories + index * 8 + 4)?)?;
    Some((pe_offset(image, rva)?, size))
}

/// Return the file offsets of the data directories and the section headers, and the
/// section count.
fn pe_layout(image: &[u8]) -> Option<(usize, usize, usize)> {
    let pe = to_usize(read_u32(image, 0x3c)?)?;
    if image.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let section_count = usize::from(read_u16(image, pe + 6)?);
    let optional_size = usize::from(read_u16(image, pe + 20)?);
    let optional = pe + 24;
    let directories = match read_u16(image, optional)? {
        0x10b => optional + 96,
        0x20b => optional + 112,
        _ => return None,
    };
    Some((directories, optional + optional_size, section_count))
}

/// Translate an RVA to a file offset through the section headers.
fn pe_offset(image: &[u8], rva: u32) -> Option<usize> {
    let (_, sections, count) = pe_layout(image)?;
    rva_to_offset(image, sections, count, rva)
}

fn rva_to_offset(image: &[u8], sections: usize, count: usize, rva: u32) -> Option<usize> {
    (0..count).find_map(|index| {
        let header = sections + index * 40;
        let virtual_size = read_u32(image, header + 8)?;
        let virtual_address = read_u32(image, header + 12)?;
        let raw_size = read_u32(image, header + 16)?;
        let raw_offset = read_u32(image, header + 20)?;
        let delta = rva.checked_sub(virtual_address)?;
        (delta < virtual_size.max(raw_size))
            .then(|| to_usize(raw_offset.checked_add(delta)?))
            .flatten()
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn to_usize(value: u32) -> Option<usize> {
    usize::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        EMBEDDED_SIGNATURE, Kind, MSF7_SIGNATURE, classify, codeview_pdb_path, embedded_pdb_payload,
    };

    fn put_u16(image: &mut [u8], offset: usize, value: u16) {
        image[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(image: &mut [u8], offset: usize, value: u32) {
        image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Build a PE32+ image with one section at RVA 0x1000 and file offset 0x200.
    /// The debug directory has one entry of `debug_type` that points at 0x300.
    fn image(debug_type: u32, payload: &[u8]) -> Vec<u8> {
        let mut image = vec![0_u8; 0x400];
        image[0..2].copy_from_slice(b"MZ");
        put_u32(&mut image, 0x3c, 0x80);
        image[0x80..0x84].copy_from_slice(b"PE\0\0");
        put_u16(&mut image, 0x86, 1);
        put_u16(&mut image, 0x94, 240);
        let optional = 0x98;
        put_u16(&mut image, optional, 0x20b);
        put_u32(&mut image, optional + 108, 16);
        put_u32(&mut image, optional + 112 + 48, 0x1010);
        put_u32(&mut image, optional + 112 + 52, 28);
        let section = optional + 240;
        put_u32(&mut image, section + 8, 0x200);
        put_u32(&mut image, section + 12, 0x1000);
        put_u32(&mut image, section + 16, 0x200);
        put_u32(&mut image, section + 20, 0x200);
        let entry = 0x210;
        put_u32(&mut image, entry + 12, debug_type);
        put_u32(&mut image, entry + 16, 8);
        put_u32(&mut image, entry + 24, 0x300);
        image[0x300..0x300 + payload.len()].copy_from_slice(payload);
        image
    }

    #[test]
    fn classifies_pdb_signatures() {
        let mut native = MSF7_SIGNATURE.to_vec();
        native.extend_from_slice(&[0; 8]);
        assert_eq!(classify(&native), Kind::NativePdb);
        assert_eq!(classify(b"BSJB\x01\x00"), Kind::PortablePdb);
        assert_eq!(
            classify(b"Microsoft C/C++ program database 2.00\r\n"),
            Kind::Unknown
        );
        assert_eq!(classify(b""), Kind::Unknown);
    }

    #[test]
    fn finds_embedded_portable_pdb_through_debug_directory() {
        let embedded = image(17, b"MPDB\x03\x00\x00\x00");
        assert_eq!(
            embedded_pdb_payload(&embedded),
            Some(&b"MPDB\x03\x00\x00\x00"[..])
        );
        assert_eq!(classify(&embedded), Kind::EmbeddedInImage);
        assert_eq!(
            classify(embedded_pdb_payload(&embedded).unwrap()),
            Kind::EmbeddedPdbPayload
        );
    }

    #[test]
    fn plain_images_and_other_debug_types_are_images() {
        assert_eq!(classify(&image(2, b"RSDS")), Kind::Image);
        assert_eq!(classify(&image(17, b"XXXX")), Kind::Image);
        assert_eq!(classify(b"MZ"), Kind::Image);
    }

    #[test]
    fn payloads_outside_the_image_are_absent() {
        let mut truncated = image(17, EMBEDDED_SIGNATURE);
        truncated.truncate(0x304);
        assert_eq!(embedded_pdb_payload(&truncated), None);
        assert_eq!(classify(&truncated), Kind::Image);
    }

    #[test]
    fn finds_the_pdb_path_of_the_codeview_record() {
        let mut record = b"RSDS".to_vec();
        record.extend_from_slice(&[0x11; 20]);
        record.extend_from_slice(b"C:\\b\\demo.pdb\0");
        let mut codeview = image(2, &record);
        let size = u32::try_from(record.len()).unwrap();
        put_u32(&mut codeview, 0x210 + 16, size);
        let path = codeview_pdb_path(&codeview).unwrap();
        assert_eq!(&codeview[path], b"C:\\b\\demo.pdb");

        // The size field cuts the path before its NUL byte.
        put_u32(&mut codeview, 0x210 + 16, size - 1);
        assert_eq!(codeview_pdb_path(&codeview), None);
        assert_eq!(codeview_pdb_path(&image(17, &record)), None);
    }
}
