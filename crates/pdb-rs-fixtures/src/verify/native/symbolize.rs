//! Symbolize addresses with `llvm-symbolizer` and a selected PDB.
//!
//! `llvm-symbolizer` loads the PDB that the `RSDS` `CodeView` record of the image names. To give
//! both PDBs the same conditions, the command prepares one directory for each PDB:
//!
//! 1. Copy the PDB into the directory. Keep its file name.
//! 2. Copy the image into the directory. Replace the PDB path of its `RSDS` record with the
//!    path [`ABSENT_DIRECTORY`] and the file name of the PDB, and fill the rest of the old
//!    path with NUL bytes. If that path is longer than the old path, use the file name only.
//!    The Rust fixtures have this short form already (`/PDBALTPATH`).
//! 3. The new path does not exist, so `llvm-symbolizer` loads the file with the same name next
//!    to the image. The command runs `llvm-symbolizer` in the directory of the image, so a path
//!    with the file name only also gives this file.
//!
//! This also gives a correct comparison for the `msvc-x64-stripped` fixture, whose image names
//! the full PDB.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::consumers::run_in;
use crate::classify::codeview_pdb_path;

/// The directory of the patched PDB path. It must not exist.
pub(crate) const ABSENT_DIRECTORY: &str = r"Q:\pdb-rs-absent\";

/// Replace the PDB path of the `RSDS` record of an image with `path`.
///
/// Fill the rest of the old path with NUL bytes. Return an error if the image has no `RSDS`
/// record or if `path` is longer than the old path.
pub(crate) fn patch_pdb_path(image: &mut [u8], path: &str) -> Result<(), String> {
    let range = codeview_pdb_path(image).ok_or("the image has no RSDS CodeView record")?;
    let old = &mut image[range];
    if path.len() > old.len() || path.as_bytes().contains(&0) {
        return Err(format!(
            "the path {path} does not fit in the {} bytes of the old PDB path",
            old.len()
        ));
    }
    old.fill(0);
    old[..path.len()].copy_from_slice(path.as_bytes());
    Ok(())
}

/// Prepare `directory` for the symbolization of `image` with `pdb`. Return the path of the
/// patched image.
///
/// The function removes an old `directory` first.
pub(crate) fn prepare(image: &Path, pdb: &Path, directory: &Path) -> Result<PathBuf, String> {
    let name = |path: &Path| {
        path.file_name()
            .ok_or_else(|| format!("{} has no file name", path.display()))
            .map(ToOwned::to_owned)
    };
    let (image_name, pdb_name) = (name(image)?, name(pdb)?);
    if directory.exists() {
        fs::remove_dir_all(directory)
            .map_err(|error| format!("cannot remove {}: {error}", directory.display()))?;
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot make {}: {error}", directory.display()))?;
    let copy = directory.join(&pdb_name);
    fs::copy(pdb, &copy).map_err(|error| {
        format!(
            "cannot copy {} to {}: {error}",
            pdb.display(),
            copy.display()
        )
    })?;
    let mut bytes =
        fs::read(image).map_err(|error| format!("cannot read {}: {error}", image.display()))?;
    let old = codeview_pdb_path(&bytes).map_or(0, |range| range.len());
    let pdb_name = pdb_name.to_string_lossy();
    let absent = format!("{ABSENT_DIRECTORY}{pdb_name}");
    let path = if absent.len() <= old {
        absent.as_str()
    } else {
        &pdb_name
    };
    patch_pdb_path(&mut bytes, path).map_err(|error| format!("{}: {error}", image.display()))?;
    let patched = directory.join(image_name);
    fs::write(&patched, bytes)
        .map_err(|error| format!("cannot write {}: {error}", patched.display()))?;
    Ok(patched)
}

/// The result of one symbolizer run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frames {
    /// The output for each address: the frame lines without the empty line after them.
    pub addresses: Vec<String>,
}

impl Frames {
    /// The number of addresses without a function name (`??`).
    pub(crate) fn unresolved(&self) -> usize {
        self.addresses
            .iter()
            .filter(|frames| frames.lines().next().is_none_or(|line| line == "??"))
            .count()
    }

    /// The number of addresses whose frames differ from the frames of `other`. An address
    /// that only one output has counts as a difference.
    pub(crate) fn mismatches(&self, other: &Self) -> usize {
        let common = self
            .addresses
            .iter()
            .zip(&other.addresses)
            .filter(|(left, right)| left != right)
            .count();
        common + self.addresses.len().abs_diff(other.addresses.len())
    }
}

/// Run `llvm-symbolizer --inlines --relative-address --functions=linkage` for `image` with
/// the RVAs on the standard input, in the directory of the image.
pub(crate) fn symbolize(symbolizer: &Path, image: &Path, rvas: &[u32]) -> Result<Frames, String> {
    let input = rvas.iter().fold(String::new(), |mut input, rva| {
        let _ = writeln!(input, "0x{rva:x}");
        input
    });
    let mut object = std::ffi::OsString::from("--obj=");
    object.push(image);
    let output = run_in(
        symbolizer,
        [
            object.as_os_str(),
            "--inlines".as_ref(),
            "--relative-address".as_ref(),
            "--functions=linkage".as_ref(),
        ],
        Some(input.as_bytes()),
        image.parent(),
    )?;
    if !output.success() {
        return Err(format!(
            "llvm-symbolizer failed with exit {:?}: {}",
            output.code,
            crate::step::last_lines(&output.text, 3)
        ));
    }
    Ok(Frames {
        addresses: frames(&output.text),
    })
}

/// Split a symbolizer output into the frames of each address. An empty line ends the frames
/// of an address.
fn frames(text: &str) -> Vec<String> {
    let mut addresses = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.is_empty() {
            addresses.push(std::mem::take(&mut current));
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.is_empty() {
        addresses.push(current);
    }
    addresses
}

#[cfg(test)]
mod tests {
    use super::{Frames, frames, patch_pdb_path};

    /// Build a PE32+ image with one section and an `RSDS` record with `path`.
    fn image(path: &str) -> Vec<u8> {
        let mut image = vec![0_u8; 0x400];
        let mut put = |offset: usize, value: u32| {
            image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(0x3c, 0x80);
        put(0x80, u32::from_le_bytes(*b"PE\0\0"));
        put(0x84, 1 << 16);
        put(0x94, 240);
        put(0x98, 0x20b);
        put(0x98 + 108, 16);
        put(0x98 + 112 + 48, 0x1010);
        put(0x98 + 112 + 52, 28);
        let section = 0x98 + 240;
        for (offset, value) in [(8, 0x200), (12, 0x1000), (16, 0x200), (20, 0x200)] {
            put(section + offset, value);
        }
        put(0x210 + 12, 2);
        put(0x210 + 16, u32::try_from(24 + path.len() + 1).unwrap());
        put(0x210 + 24, 0x300);
        image[0x300..0x304].copy_from_slice(b"RSDS");
        image[0x318..0x318 + path.len()].copy_from_slice(path.as_bytes());
        image
    }

    #[test]
    fn patches_the_pdb_path_with_nul_padding() {
        let mut bytes = image(r"C:\build\out\demo.pdb");
        patch_pdb_path(&mut bytes, r"Q:\x\demo.pdb").unwrap();
        assert_eq!(
            &bytes[0x318..0x318 + 22],
            b"Q:\\x\\demo.pdb\0\0\0\0\0\0\0\0\0"
        );

        let mut same = image(r"C:\a.pdb");
        patch_pdb_path(&mut same, r"Q:\b.pdb").unwrap();
        assert_eq!(&same[0x318..0x318 + 9], b"Q:\\b.pdb\0");
    }

    #[test]
    fn rejects_a_longer_path_and_an_image_without_a_record() {
        let mut bytes = image(r"C:\a.pdb");
        let before = bytes.clone();
        assert!(patch_pdb_path(&mut bytes, r"Q:\longer.pdb").is_err());
        assert_eq!(bytes, before);
        assert!(patch_pdb_path(&mut [0_u8; 64], "x").is_err());
    }

    #[test]
    fn compares_the_frames_of_each_address() {
        let first = Frames {
            addresses: frames("main\nC:\\a.c:3:1\n\n??\n??:0:0\n\n"),
        };
        assert_eq!(first.addresses, vec!["main\nC:\\a.c:3:1\n", "??\n??:0:0\n"]);
        assert_eq!(first.unresolved(), 1);
        let second = Frames {
            addresses: frames("main\nC:\\a.c:4:1\n\n??\n??:0:0\n\nf\nC:\\a.c:1:1\n\n"),
        };
        assert_eq!(first.mismatches(&second), 2);
        assert_eq!(first.mismatches(&first), 0);
    }
}
