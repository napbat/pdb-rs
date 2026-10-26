//! Toolchain discovery. Each probe runs once; later requests use the cached result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::environment::Environment;
use crate::matrix::{Arch, Studio};
use crate::step::last_lines;

/// A Visual Studio installation with the MSVC C++ tools.
#[derive(Debug, Clone)]
pub struct VisualStudio {
    /// The installation root.
    pub root: PathBuf,
}

impl VisualStudio {
    /// The path of `vcvarsall.bat`.
    pub fn vcvarsall(&self) -> PathBuf {
        self.root.join(r"VC\Auxiliary\Build\vcvarsall.bat")
    }

    /// The directory of the LLVM tools for the host architecture.
    pub fn llvm_bin(&self) -> PathBuf {
        let host = if std::env::consts::ARCH == "aarch64" {
            "ARM64"
        } else {
            "x64"
        };
        self.root.join(r"VC\Tools\Llvm").join(host).join("bin")
    }

    /// Find an LLVM tool such as `clang-cl.exe`.
    pub fn llvm_tool(&self, file: &str) -> Result<PathBuf, String> {
        let path = self.llvm_bin().join(file);
        if path.is_file() {
            Ok(path)
        } else {
            Err(format!(
                "{} is missing; install the Visual Studio component \"C++ Clang tools for Windows\"",
                path.display()
            ))
        }
    }
}

/// The `vcvarsall.bat` argument for a target on a host architecture.
pub fn vcvars_argument(host: &str, target: Arch) -> Option<&'static str> {
    match (host, target) {
        ("x86_64", Arch::X64) => Some("x64"),
        ("x86_64", Arch::X86) => Some("x64_x86"),
        ("x86_64", Arch::Arm64) => Some("x64_arm64"),
        ("aarch64", Arch::X64) => Some("arm64_x64"),
        ("aarch64", Arch::X86) => Some("arm64_x86"),
        ("aarch64", Arch::Arm64) => Some("arm64"),
        _ => None,
    }
}

/// The probe results of one generator run.
#[derive(Default)]
pub struct Toolchains {
    visual_studio: BTreeMap<Studio, Result<VisualStudio, String>>,
    environments: BTreeMap<(Studio, Arch), Result<Environment, String>>,
    versions: BTreeMap<String, String>,
    rust_sysroot: Option<Result<PathBuf, String>>,
    dotnet_version: Option<Result<String, String>>,
}

impl Toolchains {
    /// Find the selected Visual Studio installation with the x64 C++ tools.
    pub fn visual_studio(&mut self, studio: Studio) -> Result<VisualStudio, String> {
        self.visual_studio
            .entry(studio)
            .or_insert_with(|| find_visual_studio(studio))
            .clone()
    }

    /// Capture the `vcvarsall.bat` environment for a target.
    pub fn vcvars(&mut self, studio: Studio, arch: Arch) -> Result<Environment, String> {
        if let Some(result) = self.environments.get(&(studio, arch)) {
            return result.clone();
        }
        let result = self.visual_studio(studio).and_then(|installation| {
            let argument = vcvars_argument(std::env::consts::ARCH, arch).ok_or_else(|| {
                format!(
                    "no vcvarsall.bat argument for host {}",
                    std::env::consts::ARCH
                )
            })?;
            capture_vcvars(&installation.vcvarsall(), argument)
        });
        self.environments.insert((studio, arch), result.clone());
        result
    }

    /// Run a program once and return the first line of its output as its version.
    ///
    /// The cache key is the program path and the arguments.
    pub fn version(
        &mut self,
        program: &Path,
        arguments: &[&str],
        environment: Option<&Environment>,
    ) -> String {
        let key = format!("{} {}", program.display(), arguments.join(" "));
        if let Some(version) = self.versions.get(&key) {
            return version.clone();
        }
        let version = probe_version(program, arguments, environment);
        self.versions.insert(key, version.clone());
        version
    }

    /// Find the sysroot of the default `rustc`.
    pub fn rust_sysroot(&mut self) -> Result<PathBuf, String> {
        self.rust_sysroot
            .get_or_insert_with(|| {
                let text = capture(Command::new("rustc").args(["--print", "sysroot"]))?;
                Ok(PathBuf::from(text.trim()))
            })
            .clone()
    }

    /// Return the version of the .NET SDK that `dotnet` selects in `directory`.
    pub fn dotnet_version(&mut self, directory: &Path) -> Result<String, String> {
        self.dotnet_version
            .get_or_insert_with(|| {
                let text = capture(
                    Command::new("dotnet")
                        .arg("--version")
                        .current_dir(directory),
                )
                .map_err(|error| format!("the .NET SDK is not available: {error}"))?;
                Ok(text.trim().to_owned())
            })
            .clone()
    }
}

fn probe_version(program: &Path, arguments: &[&str], environment: Option<&Environment>) -> String {
    let mut command = Command::new(program);
    command.args(arguments);
    if let Some(environment) = environment {
        command.env_clear().envs(environment.iter());
    }
    match command.output() {
        Ok(output) => {
            // MSVC `cl.exe` prints its banner on standard error and its usage on standard output.
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            text.lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("no output")
                .to_owned()
        }
        Err(error) => format!("cannot run {}: {error}", program.display()),
    }
}

/// Run a command and return its standard output if it succeeds.
pub fn capture(command: &mut Command) -> Result<String, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} failed with {}: {}",
            output.status,
            last_lines(&String::from_utf8_lossy(&output.stderr), 3)
        ))
    }
}

fn find_visual_studio(studio: Studio) -> Result<VisualStudio, String> {
    let program_files = std::env::var_os("ProgramFiles(x86)")
        .map_or_else(|| PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from);
    let vswhere = program_files.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    if !vswhere.is_file() {
        return Err(format!(
            "{} is missing; Visual Studio is not installed",
            vswhere.display()
        ));
    }
    let text = capture(Command::new(&vswhere).args(studio.vswhere_filter()).args([
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "-property",
        "installationPath",
    ]))?;
    let root = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| {
            format!(
                "vswhere found no {} installation with the MSVC x64 C++ tools",
                studio.label()
            )
        })?;
    Ok(VisualStudio {
        root: PathBuf::from(root),
    })
}

#[cfg(windows)]
fn capture_vcvars(vcvarsall: &Path, argument: &str) -> Result<Environment, String> {
    use std::os::windows::process::CommandExt as _;

    // With /s, cmd removes the outer quotes and keeps the quotes around the path.
    let script = format!(
        "\"call \"{}\" {argument} >nul && set\"",
        vcvarsall.display()
    );
    let text = capture(Command::new("cmd").args(["/d", "/s", "/c"]).raw_arg(script))
        .map_err(|error| format!("vcvarsall.bat {argument}: {error}"))?;
    let environment = Environment::parse_set_output(&text);
    if environment.is_empty() {
        Err(format!("vcvarsall.bat {argument} printed no environment"))
    } else {
        Ok(environment)
    }
}

#[cfg(not(windows))]
fn capture_vcvars(_vcvarsall: &Path, argument: &str) -> Result<Environment, String> {
    Err(format!("vcvarsall.bat {argument} needs Windows"))
}

#[cfg(test)]
mod tests {
    use super::vcvars_argument;
    use crate::matrix::Arch;

    #[test]
    fn maps_host_and_target_to_vcvars_arguments() {
        assert_eq!(vcvars_argument("x86_64", Arch::X64), Some("x64"));
        assert_eq!(vcvars_argument("x86_64", Arch::X86), Some("x64_x86"));
        assert_eq!(vcvars_argument("x86_64", Arch::Arm64), Some("x64_arm64"));
        assert_eq!(vcvars_argument("aarch64", Arch::Arm64), Some("arm64"));
        assert_eq!(vcvars_argument("riscv64", Arch::X64), None);
    }
}
