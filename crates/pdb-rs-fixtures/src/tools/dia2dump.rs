//! `DIA2Dump`, built from a changed copy of the DIA SDK sample of Visual Studio.
//!
//! The build never writes to the Visual Studio installation. It copies the sample to
//! `dia2dump/` in the tools root, changes the copy and builds it with `MSBuild`.

use std::fs;
use std::path::Path;

use super::file_version::file_version;
use super::patch::{self, DLL};
use super::{Missing, Paths, Tool};
use crate::matrix::Studio;
use crate::step::Step;
use crate::toolchain::Toolchains;

/// The tool name.
pub const NAME: &str = "DIA2Dump";

/// The `MSBuild` platform. The sample has no ARM64 configuration, so the build is always x64.
const PLATFORM: &str = "x64";

/// The DIA SDK directory of the `lib` and `bin` files for [`PLATFORM`].
const SDK_ARCH: &str = "amd64";

/// A description of the changes to the copy.
const CHANGES: &str = "DIA2Dump.cpp: NoRegCoCreate(L\"msdia140.dll\", __uuidof(DiaSource), \
    __uuidof(IDiaDataSource), ...) replaces CoCreateInstance(__uuidof(DiaSource), ...); \
    stdafx.h: includes diacreate.h and links diaguids.lib with #pragma comment(lib)";

/// Copy, change and build the `DIA2Dump` sample, and put `msdia140.dll` next to it.
pub fn build(paths: &Paths, toolchains: &mut Toolchains) -> Tool {
    super::record(NAME, |tool| {
        let studio = toolchains
            .visual_studio(Studio::Latest)
            .map_err(Missing::Unavailable)?;
        let sdk = studio.root.join("DIA SDK");
        let sample = sdk.join("Samples").join("DIA2Dump");
        let diaguids = sdk.join("lib").join(SDK_ARCH).join("diaguids.lib");
        let dll = sdk.join("bin").join(SDK_ARCH).join(DLL);
        let msbuild = studio.root.join(r"MSBuild\Current\Bin\MSBuild.exe");
        let copy = paths.root.join("dia2dump");
        for (name, path) in [
            ("visual_studio", &studio.root),
            ("sample", &sample),
            ("diaguids.lib", &diaguids),
            (DLL, &dll),
            ("msbuild", &msbuild),
            ("copy", &copy),
        ] {
            tool.source(name, path.to_string_lossy());
        }
        tool.source("changes", CHANGES);
        for path in [
            &sample.join("DIA2Dump.vcxproj"),
            &sdk.join("include").join("diacreate.h"),
            &diaguids,
            &dll,
            &msbuild,
        ] {
            if !path.is_file() {
                return Err(Missing::Unavailable(format!(
                    "{} is missing; the Visual Studio installation has no DIA SDK or MSBuild",
                    path.display()
                )));
            }
        }

        tool.version(
            "visual_studio",
            studio
                .installation_version()
                .map_err(Missing::Unavailable)?,
        );
        let toolset = platform_toolset(&studio.root.join(r"VC\Auxiliary\Build"))?;
        tool.version("platform_toolset", toolset.as_str());
        let bytes = fs::read(&dll)
            .map_err(|error| Missing::Failed(format!("cannot read {}: {error}", dll.display())))?;
        tool.version(
            DLL,
            file_version(&bytes)
                .unwrap_or_else(|| "unknown: no VS_FIXEDFILEINFO resource".to_owned()),
        );

        prepare(&sample, &copy, &diaguids).map_err(Missing::Failed)?;
        let executable = paths.bin.join("DIA2Dump.exe");
        let dll_copy = paths.bin.join(DLL);
        for stale in [&executable, &paths.bin.join("DIA2Dump.pdb"), &dll_copy] {
            remove_file(stale).map_err(Missing::Failed)?;
        }

        // MSBuild needs a final separator in `OutDir`.
        let out_dir = format!("{}\\", paths.bin.display());
        tool.steps.push(Step::new(&msbuild, &copy).args([
            "DIA2Dump.vcxproj".to_owned(),
            "-nologo".to_owned(),
            "-nodeReuse:false".to_owned(),
            "-verbosity:minimal".to_owned(),
            "-target:Rebuild".to_owned(),
            "-property:Configuration=Release".to_owned(),
            format!("-property:Platform={PLATFORM}"),
            format!("-property:PlatformToolset={toolset}"),
            format!("-property:OutDir={out_dir}"),
        ]));
        super::run_steps(&tool.steps, &paths.log(NAME))?;
        fs::copy(&dll, &dll_copy).map_err(|error| {
            Missing::Failed(format!(
                "cannot copy {} to {}: {error}",
                dll.display(),
                dll_copy.display()
            ))
        })?;
        tool.finish(vec![executable, dll_copy])
    })
}

/// Make a fresh copy of the sample in `copy` and apply the two changes.
fn prepare(sample: &Path, copy: &Path, diaguids: &Path) -> Result<(), String> {
    if copy.exists() {
        fs::remove_dir_all(copy)
            .map_err(|error| format!("cannot clean {}: {error}", copy.display()))?;
    }
    copy_directory(sample, copy)?;
    edit(&copy.join("DIA2Dump.cpp"), patch::no_reg_co_create)?;
    edit(&copy.join("stdafx.h"), |header| {
        patch::link_diaguids(header, diaguids)
    })
}

/// Copy the files of `from` into the new directory `to`, with subdirectories.
///
/// The copies get default permissions, so a read-only source gives a writable copy.
fn copy_directory(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| format!("cannot make {}: {error}", to.display()))?;
    let entries =
        fs::read_dir(from).map_err(|error| format!("cannot read {}: {error}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", from.display()))?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_directory(&source, &target)?;
        } else {
            let bytes = fs::read(&source)
                .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
            fs::write(&target, bytes)
                .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
        }
    }
    Ok(())
}

/// Replace the text of `path` with the result of `change`.
fn edit(path: &Path, change: impl FnOnce(&str) -> Result<String, String>) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    fs::write(path, change(&text)?)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Remove a file if it exists.
fn remove_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("cannot remove {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

/// The platform toolset of the installation, from the names of the files in
/// `VC\Auxiliary\Build`.
fn platform_toolset(directory: &Path) -> Result<String, Missing> {
    let entries = fs::read_dir(directory).map_err(|error| {
        Missing::Unavailable(format!("cannot read {}: {error}", directory.display()))
    })?;
    let names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    newest_toolset(names.iter().map(String::as_str)).ok_or_else(|| {
        Missing::Unavailable(format!(
            "{} has no Microsoft.VCToolsVersion.v<NNN>.default.txt file",
            directory.display()
        ))
    })
}

/// The newest toolset of the `Microsoft.VCToolsVersion.v<NNN>.default.txt` file names, for
/// example `v145`.
fn newest_toolset<'a>(names: impl Iterator<Item = &'a str>) -> Option<String> {
    names
        .filter_map(|name| {
            let digits = name
                .strip_prefix("Microsoft.VCToolsVersion.v")?
                .strip_suffix(".default.txt")?;
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            digits.parse::<u32>().ok()
        })
        .max()
        .map(|number| format!("v{number}"))
}

#[cfg(test)]
mod tests {
    use super::newest_toolset;

    #[test]
    fn selects_the_newest_toolset_file() {
        let names = [
            "Microsoft.VCToolsVersion.default.txt",
            "Microsoft.VCToolsVersion.v143.default.txt",
            "Microsoft.VCToolsVersion.v145.default.txt",
            "Microsoft.VCToolsVersion.v145.default.props",
            "Microsoft.VCRedistVersion.v150.default.txt",
            "Microsoft.VCToolsVersion.vNext.default.txt",
            "vcvarsall.bat",
        ];
        assert_eq!(newest_toolset(names.into_iter()).as_deref(), Some("v145"));
        assert_eq!(
            newest_toolset(["Microsoft.VCToolsVersion.default.txt"].into_iter()),
            None
        );
    }
}
