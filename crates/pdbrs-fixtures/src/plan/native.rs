//! Plans for the MSVC and `clang-cl` builds of `fixtures/native`.

mod generated;
mod variant;

use std::path::{Path, PathBuf};

use super::{Expected, Paths, Plan};
use crate::classify::Kind;
use crate::environment::Environment;
use crate::matrix::{Compiler, Linker, Native, Variant};
use crate::step::{Step, quote};
use crate::toolchain::{Toolchains, vcvars_argument};

/// The C++ translation units.
const CPP_SOURCES: [&str; 4] = ["main.cpp", "types.cpp", "control.cpp", "big_enum.cpp"];
/// The C translation units.
const C_SOURCES: [&str; 1] = ["c_api.c"];
/// The header that the precompiled header variant precompiles. Each C++ file includes it first.
const PRECOMPILED_HEADER: &str = "demo.h";

/// Make the plan of a native fixture.
pub fn plan(recipe: &Native, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    let studio = toolchains.visual_studio(recipe.studio)?;
    let environment = toolchains.vcvars(recipe.studio, recipe.arch)?;
    let label = format!(
        "{} {}",
        quote(&studio.vcvarsall().display().to_string()),
        vcvars_argument(std::env::consts::ARCH, recipe.arch).unwrap_or_default()
    );
    let mut plan = Plan::default();
    let compiler = match recipe.compiler {
        Compiler::Msvc => msvc_tool(toolchains, &environment, &label, "cl.exe", &mut plan)?,
        Compiler::ClangCl => llvm_tool(toolchains, studio.llvm_tool("clang-cl.exe")?, &mut plan),
    };
    let linker = match recipe.linker {
        Linker::Msvc => msvc_tool(toolchains, &environment, &label, "link.exe", &mut plan)?,
        Linker::Lld => llvm_tool(toolchains, studio.llvm_tool("lld-link.exe")?, &mut plan),
    };
    let build = Build {
        recipe,
        fixture: paths.fixture.clone(),
        sources: paths.source("native"),
        objects: paths.fixture.join("obj"),
        label,
        environment,
        compiler,
        linker,
    };
    plan.directories.push(build.objects.clone());
    match recipe.variant {
        Variant::Demo | Variant::PrecompiledHeader | Variant::Stripped | Variant::Natvis => {
            demo(&build, &mut plan);
        }
        Variant::SourceServer => variant::source_server(&build, &mut plan)?,
        Variant::ProfileGuided => variant::profile_guided(&build, &mut plan)?,
        Variant::Dll => variant::dll(&build, &mut plan),
        Variant::Clr => variant::clr(&build, &mut plan)?,
        Variant::Annotation => variant::annotation(&build, &mut plan),
        Variant::Generated => generated::generated(&build, &mut plan),
    }
    Ok(plan)
}

/// The tools, environment and directories of one native fixture.
struct Build<'a> {
    /// The recipe.
    recipe: &'a Native,
    /// The output directory of the fixture.
    fixture: PathBuf,
    /// The `fixtures/native` source directory.
    sources: PathBuf,
    /// The directory of the object files.
    objects: PathBuf,
    /// The source of the environment, for the manifest.
    label: String,
    /// The `vcvarsall.bat` environment.
    environment: Environment,
    /// The compiler.
    compiler: PathBuf,
    /// The linker.
    linker: PathBuf,
}

impl Build<'_> {
    /// A step that runs `program` in the fixture directory with the `vcvarsall.bat` environment.
    fn step(&self, program: &Path) -> Step {
        Step::new(program, &self.fixture).environment(&self.label, &self.environment)
    }

    /// The absolute path of a file below `fixtures/native`. The name uses `/` separators.
    fn source(&self, relative: &str) -> PathBuf {
        let mut path = self.sources.clone();
        path.extend(relative.split('/'));
        path
    }

    /// The object file of a source file below `fixtures/native`.
    fn object(&self, source: &str) -> PathBuf {
        let name = Path::new(source).file_name().unwrap_or_default();
        self.objects.join(Path::new(name).with_extension("obj"))
    }

    /// A compile step for source files below `fixtures/native`.
    ///
    /// The `extra` options come after the recipe options and before the source files.
    fn compile(&self, language: &str, extra: &[String], files: &[&str]) -> Step {
        let mut arguments = compile_options(self.recipe, language, &self.objects);
        arguments.extend(extra.iter().cloned());
        arguments.extend(
            files
                .iter()
                .map(|file| self.source(file).display().to_string()),
        );
        self.step(&self.compiler).args(arguments)
    }

    /// A link step. The `extra` options come after the recipe options.
    fn link(&self, extra: &[String], image: &Path, pdb: &Path, inputs: &[PathBuf]) -> Step {
        let mut arguments: Vec<String> = vec!["/nologo".to_owned()];
        arguments.extend(self.recipe.link.iter().map(|option| (*option).to_owned()));
        arguments.extend(extra.iter().cloned());
        arguments.push(format!("/OUT:{}", image.display()));
        arguments.push(format!("/PDB:{}", pdb.display()));
        arguments.extend(inputs.iter().map(|input| input.display().to_string()));
        self.step(&self.linker).args(arguments)
    }

    /// The compile steps of the demo sources and the object files that they make.
    fn compile_demo(&self, plan: &mut Plan) -> Vec<PathBuf> {
        if self.recipe.variant == Variant::PrecompiledHeader {
            let header = self.objects.join("demo.pch");
            let create = vec![
                format!("/Fp{}", header.display()),
                format!("/Yc{PRECOMPILED_HEADER}"),
            ];
            let mut using = create.clone();
            using[1] = format!("/Yu{PRECOMPILED_HEADER}");
            let (first, rest) = CPP_SOURCES.split_at(1);
            plan.steps.push(self.compile("/std:c++20", &create, first));
            plan.steps.push(self.compile("/std:c++20", &using, rest));
        } else {
            plan.steps
                .push(self.compile("/std:c++20", &[], &CPP_SOURCES));
        }
        plan.steps.push(self.compile("/std:c17", &[], &C_SOURCES));
        CPP_SOURCES
            .iter()
            .chain(&C_SOURCES)
            .map(|file| self.object(file))
            .collect()
    }

    /// An expected output in the fixture directory.
    fn output(&self, file: &str, kind: Kind) -> Expected {
        Expected {
            path: self.fixture.join(file),
            kind,
        }
    }
}

/// Plan the demo program: compile, link `demo.exe` and `demo.pdb`.
///
/// The `Stripped` and `Natvis` variants add linker options and outputs.
fn demo(build: &Build<'_>, plan: &mut Plan) {
    let objects = build.compile_demo(plan);
    let image = build.fixture.join("demo.exe");
    let pdb = build.fixture.join("demo.pdb");
    let mut extra = Vec::new();
    match build.recipe.variant {
        Variant::Stripped => extra.push(format!(
            "/PDBSTRIPPED:{}",
            build.fixture.join("demo.stripped.pdb").display()
        )),
        Variant::Natvis => extra.push(format!("/NATVIS:{}", build.source("demo.natvis").display())),
        _ => {}
    }
    let mut link = build.link(&extra, &image, &pdb, &objects);
    if build.recipe.link.contains(&"/DEBUG:FASTLINK") {
        // MSVC 14.50 and later ignore /DEBUG:FASTLINK, warn with LNK4315 and write a full PDB.
        link = link.unsupported_when(
            "LNK4315",
            "the selected link.exe ignores /DEBUG:FASTLINK (warning LNK4315) and writes a \
             /DEBUG:FULL PDB; a Visual Studio 2022 toolset (MSVC 14.4x or older) is needed",
        );
    }
    plan.steps.push(link);
    plan.outputs = vec![
        build.output("demo.exe", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
    if build.recipe.variant == Variant::Stripped {
        plan.outputs
            .push(build.output("demo.stripped.pdb", Kind::NativePdb));
    }
}

/// The options of one compile step, before the source files.
fn compile_options(recipe: &Native, language: &str, objects: &Path) -> Vec<String> {
    // C++/CLI code needs the asynchronous exception model.
    let exceptions = if recipe.variant == Variant::Clr {
        "/EHa"
    } else {
        "/EHsc"
    };
    let mut options: Vec<String> = ["/nologo", "/c", "/W3", exceptions, "/utf-8"]
        .iter()
        .map(|option| (*option).to_owned())
        .collect();
    options.extend(recipe.compile.iter().map(|option| (*option).to_owned()));
    options.push(language.to_owned());
    match recipe.compiler {
        Compiler::Msvc => options.push(format!("/Fd{}", objects.join("compile.pdb").display())),
        Compiler::ClangCl => options.push(format!("--target={}", recipe.arch.clang_target())),
    }
    // A trailing separator tells the compiler that /Fo names a directory.
    options.push(format!(
        "/Fo{}{}",
        objects.display(),
        std::path::MAIN_SEPARATOR
    ));
    options
}

/// Find an MSVC tool on the `PATH` of the environment and record its banner.
fn msvc_tool(
    toolchains: &mut Toolchains,
    environment: &Environment,
    label: &str,
    file: &str,
    plan: &mut Plan,
) -> Result<PathBuf, String> {
    let path = environment
        .find_program(file)
        .ok_or_else(|| format!("{file} is not on the PATH of {label}"))?;
    let name = file.trim_end_matches(".exe").to_owned();
    plan.tools
        .push((name, toolchains.version(&path, &[], Some(environment))));
    Ok(path)
}

/// Record the version of an LLVM tool.
fn llvm_tool(toolchains: &mut Toolchains, path: PathBuf, plan: &mut Plan) -> PathBuf {
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    plan.tools
        .push((name, toolchains.version(&path, &["--version"], None)));
    path
}

#[cfg(test)]
mod tests {
    use super::{Build, demo};
    use crate::environment::Environment;
    use crate::matrix::{Arch, Compiler, Linker, Native, Studio, Variant};
    use crate::plan::Plan;
    use std::path::{Path, PathBuf};

    fn recipe(variant: Variant) -> Native {
        Native {
            studio: Studio::Latest,
            compiler: Compiler::Msvc,
            linker: Linker::Msvc,
            arch: Arch::X64,
            compile: &["/Zi"],
            link: &["/DEBUG:FULL"],
            variant,
        }
    }

    fn build(recipe: &Native) -> Build<'_> {
        let fixture = PathBuf::from("out");
        Build {
            recipe,
            objects: fixture.join("obj"),
            fixture,
            sources: PathBuf::from("src"),
            label: "test".to_owned(),
            environment: Environment::default(),
            compiler: PathBuf::from("cl.exe"),
            linker: PathBuf::from("link.exe"),
        }
    }

    fn plan_demo(variant: Variant) -> Plan {
        let recipe = recipe(variant);
        let mut plan = Plan::default();
        demo(&build(&recipe), &mut plan);
        plan
    }

    #[test]
    fn precompiled_header_creates_once_and_uses_it_in_the_other_files() {
        let plan = plan_demo(Variant::PrecompiledHeader);
        assert_eq!(plan.steps.len(), 4);
        let create = &plan.steps[0].arguments;
        assert!(create.contains(&"/Ycdemo.h".to_owned()));
        assert!(create.last().unwrap().ends_with("main.cpp"));
        let using = &plan.steps[1].arguments;
        assert!(using.contains(&"/Yudemo.h".to_owned()));
        let sources: Vec<String> = ["types.cpp", "control.cpp", "big_enum.cpp"]
            .iter()
            .map(|file| Path::new("src").join(file).display().to_string())
            .collect();
        assert!(using.ends_with(&sources));
        assert!(!plan.steps[2].arguments.iter().any(|a| a.starts_with("/Y")));
        let header = Path::new("out").join("obj").join("demo.pch");
        assert!(create.contains(&format!("/Fp{}", header.display())));
    }

    #[test]
    fn stripped_and_natvis_add_link_options_and_outputs() {
        let stripped = plan_demo(Variant::Stripped);
        let link = &stripped.steps.last().unwrap().arguments;
        assert!(link.iter().any(|a| a.starts_with("/PDBSTRIPPED:")));
        assert_eq!(stripped.outputs.len(), 3);
        let natvis = plan_demo(Variant::Natvis);
        let link = &natvis.steps.last().unwrap().arguments;
        let file = Path::new("src").join("demo.natvis");
        assert!(link.contains(&format!("/NATVIS:{}", file.display())));
        assert_eq!(natvis.outputs.len(), 2);
    }

    #[test]
    fn clr_uses_asynchronous_exceptions_and_nested_sources() {
        let clr = recipe(Variant::Clr);
        let build = build(&clr);
        let step = build.compile("/std:c++20", &[], &["clr/managed.cpp"]);
        assert!(step.arguments.contains(&"/EHa".to_owned()));
        assert!(!step.arguments.contains(&"/EHsc".to_owned()));
        assert_eq!(
            step.arguments.last().unwrap(),
            &Path::new("src")
                .join("clr")
                .join("managed.cpp")
                .display()
                .to_string()
        );
        assert_eq!(
            build.object("clr/managed.cpp"),
            Path::new("out").join("obj").join("managed.obj")
        );
        let demo = recipe(Variant::Demo);
        let step = super::compile_options(&demo, "/std:c17", Path::new("obj"));
        assert!(step.contains(&"/EHsc".to_owned()));
    }
}
