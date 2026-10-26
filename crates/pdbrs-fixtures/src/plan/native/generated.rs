//! The plan of the generated variant: a large program from C++ sources that the generator
//! writes into the fixture directory.
//!
//! An MSF free page map interval covers as many pages as a page has bytes: 4096 pages, or
//! 16 MiB, at the smallest page size that `link` accepts. The sources hold enough structures
//! and functions for a PDB of more than 4096 pages, so its free page map has a second interval.

use std::fmt::Write as _;
use std::path::PathBuf;

use super::{Build, compile_options};
use crate::classify::Kind;
use crate::plan::Plan;

/// The number of generated structures. Each one has one function.
const RECORDS: usize = 28_000;
/// The number of generated translation units, other than `main.cpp`.
const PARTS: usize = 8;

/// Plan the generated program `demo.exe` and its PDB `demo.pdb`.
pub fn generated(build: &Build<'_>, plan: &mut Plan) {
    let directory = build.fixture.join("src");
    plan.directories.push(directory.clone());
    let sources = sources();
    let objects: Vec<PathBuf> = sources.iter().map(|(name, _)| build.object(name)).collect();
    let files: Vec<(PathBuf, String)> = sources
        .into_iter()
        .map(|(name, text)| (directory.join(name), text))
        .collect();
    let mut arguments = compile_options(build.recipe, "/std:c++20", &build.objects);
    arguments.extend(files.iter().map(|(path, _)| path.display().to_string()));
    plan.steps.push(build.step(&build.compiler).args(arguments));
    plan.files = files;
    plan.steps.push(build.link(
        &[],
        &build.fixture.join("demo.exe"),
        &build.fixture.join("demo.pdb"),
        &objects,
    ));
    plan.outputs = vec![
        build.output("demo.exe", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
}

/// The file names and the contents of the generated sources.
///
/// Each part defines `RECORDS / PARTS` structures, a function that reads each structure and a
/// function `PartN` that calls them. `main.cpp` calls each `PartN`.
fn sources() -> Vec<(String, String)> {
    let per_part = RECORDS / PARTS;
    let mut files = Vec::with_capacity(PARTS + 1);
    let mut main = String::new();
    for part in 0..PARTS {
        let records = part * per_part..(part + 1) * per_part;
        let mut text = String::new();
        for index in records.clone() {
            let tag = index % 13 + 3;
            let values = index % 7 + 1;
            let _ = writeln!(
                text,
                "struct Record{index} {{ int id; long long total; double ratio; \
                 char tag[{tag}]; Record{index}* next; unsigned short flags; float weight; \
                 int values[{values}]; }};"
            );
            let _ = writeln!(
                text,
                "int Sum{index}(Record{index} const* r) {{ int s = r->id; \
                 for (int k = 0; k < {values}; ++k) {{ s += r->values[k]; }} \
                 return s + static_cast<int>(r->flags); }}"
            );
        }
        let _ = writeln!(text, "int Part{part}() {{\n  int s = 0;");
        for index in records {
            let _ = writeln!(
                text,
                "  {{ Record{index} r{{}}; r.id = {index}; s += Sum{index}(&r); }}"
            );
        }
        text.push_str("  return s;\n}\n");
        files.push((format!("part{part}.cpp"), text));
        let _ = writeln!(main, "int Part{part}();");
    }
    main.push_str("int main() {\n  int s = 0;\n");
    for part in 0..PARTS {
        let _ = writeln!(main, "  s += Part{part}();");
    }
    main.push_str("  return s & 1;\n}\n");
    files.push(("main.cpp".to_owned(), main));
    files
}

#[cfg(test)]
mod tests {
    use super::{PARTS, RECORDS, sources};

    #[test]
    fn sources_are_deterministic_and_define_each_record_once() {
        let first = sources();
        assert_eq!(first, sources());
        assert_eq!(first.len(), PARTS + 1);
        let structures: usize = first
            .iter()
            .map(|(_, text)| text.matches("struct Record").count())
            .sum();
        assert_eq!(structures, RECORDS);
        let (name, main) = first.last().unwrap();
        assert_eq!(name, "main.cpp");
        assert_eq!(main.matches("s += Part").count(), PARTS);
    }
}
