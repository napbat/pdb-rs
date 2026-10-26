//! Shared parts of the commands that compare native PDBs through the consumer tools.
//!
//! The `verify-native-build` command (see [`build`]) uses these parts. A later command that
//! edits native PDBs can use them too:
//!
//! - [`native_fixtures`] lists the native PDBs of the fixture manifest and the skipped fixtures.
//! - [`consumers`] finds and runs `pdbtool`, `llvm-pdbutil` and `DIA2Dump`, and normalizes their
//!   output.
//! - [`sample`] selects the addresses and the type names for the DIA queries.
//! - [`symbolize`] runs `llvm-symbolizer` with a patched copy of the image.
//! - [`diff`] compares two dumps line by line and classifies the differences.

pub mod build;
pub(crate) mod consumers;
pub(crate) mod diff;
pub(crate) mod sample;
pub(crate) mod symbolize;

use std::path::{Path, PathBuf};

use super::outputs;

/// The fixtures that the native commands skip, with the reason.
pub(crate) const SKIPPED_FIXTURES: [(&str, &str); 1] = [(
    "hlsl-dxc-debug",
    "pdbtool, llvm-pdbutil and DIA2Dump reject the original PDBs",
)];

/// One native PDB of the fixture manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeFixture {
    /// The fixture name.
    pub fixture: String,
    /// The path of the PDB.
    pub pdb: PathBuf,
    /// The path of the first `image` output of the fixture, if the fixture has one.
    pub image: Option<PathBuf>,
}

impl NativeFixture {
    /// The file name of the PDB, for example `demo.pdb`.
    pub(crate) fn file_name(&self) -> String {
        self.pdb
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    }

    /// The name in the output: the fixture name and the file name of the PDB, for example
    /// `msvc-x64-srcsrv/demo.srcsrv.pdb`.
    pub(crate) fn label(&self) -> String {
        format!("{}/{}", self.fixture, self.file_name())
    }

    /// The work directory of the PDB below `root`: `<root>/<fixture>/<file stem>`.
    pub(crate) fn work_directory(&self, root: &Path) -> PathBuf {
        let stem = self
            .pdb
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        root.join(&self.fixture).join(stem)
    }
}

/// A fixture that a native command does not check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedFixture {
    /// The fixture name.
    pub fixture: String,
    /// The reason from [`SKIPPED_FIXTURES`].
    pub reason: &'static str,
}

/// Read the native PDBs of `manifest.json` in `fixtures`.
///
/// Return each `native-pdb` output with the first `image` output of its fixture, in manifest
/// order. A fixture with more than one native PDB, for example `msvc-x64-srcsrv`, gives one
/// entry for each PDB. Return the fixtures of [`SKIPPED_FIXTURES`] that have a native PDB
/// separately.
pub(crate) fn native_fixtures(
    fixtures: &Path,
) -> Result<(Vec<NativeFixture>, Vec<SkippedFixture>), String> {
    let outputs = outputs(fixtures)?;
    let mut found = Vec::new();
    let mut skipped: Vec<SkippedFixture> = Vec::new();
    for output in outputs.iter().filter(|output| output.kind == "native-pdb") {
        if let Some((_, reason)) = SKIPPED_FIXTURES
            .iter()
            .find(|(name, _)| *name == output.fixture)
        {
            if !skipped.iter().any(|entry| entry.fixture == output.fixture) {
                skipped.push(SkippedFixture {
                    fixture: output.fixture.clone(),
                    reason,
                });
            }
            continue;
        }
        let image = outputs
            .iter()
            .find(|other| other.fixture == output.fixture && other.kind == "image")
            .map(|other| other.path.clone());
        found.push(NativeFixture {
            fixture: output.fixture.clone(),
            pdb: output.path.clone(),
            image,
        });
    }
    Ok((found, skipped))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{NativeFixture, native_fixtures};

    #[test]
    fn lists_each_native_pdb_with_the_image_of_its_fixture() {
        let root =
            std::env::temp_dir().join(format!("pdbrs-native-fixtures-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let manifest = r#"{"fixtures": [
            {"name": "a", "outputs": [
                {"kind": "native-pdb", "path": "a/demo.pdb"},
                {"kind": "image", "path": "a/demo.exe"},
                {"kind": "native-pdb", "path": "a/demo.srcsrv.pdb"}]},
            {"name": "hlsl-dxc-debug", "outputs": [
                {"kind": "native-pdb", "path": "h/vertex.pdb"},
                {"kind": "native-pdb", "path": "h/pixel.pdb"}]},
            {"name": "s", "outputs": [{"kind": "native-pdb", "path": "s/vertex.pdb"}]},
            {"name": "p", "outputs": [{"kind": "portable-pdb", "path": "p/p.pdb"}]}]}"#;
        fs::write(root.join("manifest.json"), manifest).unwrap();
        let (found, skipped) = native_fixtures(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let image = Some(root.join("a/demo.exe"));
        let expected = [
            ("a", "a/demo.pdb", image.clone()),
            ("a", "a/demo.srcsrv.pdb", image),
            ("s", "s/vertex.pdb", None),
        ]
        .map(|(fixture, pdb, image)| NativeFixture {
            fixture: fixture.to_owned(),
            pdb: root.join(pdb),
            image,
        });
        assert_eq!(found, expected);
        assert_eq!(found[1].label(), "a/demo.srcsrv.pdb");
        assert_eq!(
            found[1].work_directory(std::path::Path::new("w")),
            std::path::Path::new("w").join("a").join("demo.srcsrv")
        );
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].fixture, "hlsl-dxc-debug");
    }
}
