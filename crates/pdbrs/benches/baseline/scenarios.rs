//! The fixtures and the scenarios, in the order of the output.

use std::path::Path;

use pdbrs::native::Pdb;

use crate::measure::Bench;
use crate::{Outcome, edit, full_read, lookup, portable};

/// A fixture file below `target/fixtures`.
#[derive(Clone, Copy)]
pub(crate) struct Fixture {
    /// The name in the output.
    pub(crate) name: &'static str,
    /// The path relative to the fixture directory.
    pub(crate) path: &'static str,
}

/// The large native fixture.
const NATIVE_LARGE: Fixture = Fixture {
    name: "msvc-x64-large",
    path: "msvc-x64-large/demo.pdb",
};

/// The small native fixture. It has the compiler and linker options of the large fixture.
const NATIVE_SMALL: Fixture = Fixture {
    name: "msvc-x64-debug",
    path: "msvc-x64-debug/demo.pdb",
};

/// The largest standalone Portable PDB.
const PORTABLE: Fixture = Fixture {
    name: "dotnet-csharp-embed-sources",
    path: "dotnet-csharp-embed-sources/artifacts/bin/CSharpDemo/debug/CSharpDemo.pdb",
};

/// The MSFZ form of the large native fixture that `pdbtool pdz-encode` writes.
const MSFZ: Fixture = Fixture {
    name: "pdz-default/msvc-x64-large",
    path: "pdz-default/msvc-x64-large/demo.pdb",
};

/// The function of a scenario: it loads the fixture and calls [`Bench::measure`] once.
type Run = fn(&Path, &mut Bench) -> Outcome;

/// One scenario on one fixture.
pub(crate) struct Scenario {
    /// The scenario name, for example `native/open`.
    pub(crate) name: &'static str,
    /// The fixture, or `None` for a scenario without input.
    pub(crate) fixture: Option<Fixture>,
    /// The function that does the scenario.
    pub(crate) run: Run,
}

impl Scenario {
    /// Return the identifier of the child process argument: `<name>@<fixture>`.
    pub(crate) fn id(&self) -> String {
        match self.fixture {
            Some(fixture) => format!("{}@{}", self.name, fixture.name),
            None => self.name.to_owned(),
        }
    }
}

/// The native scenarios. The output has them for each native fixture in this order.
const NATIVE: [(&str, Run); 16] = [
    ("native/open", open),
    ("native/full-read", read),
    ("native/type-table", lookup::type_table),
    ("native/type-by-index", lookup::type_by_index),
    ("native/type-by-name", lookup::type_by_name),
    ("native/symbol-locator", lookup::symbol_locator),
    ("native/symbol-by-name", lookup::symbol_by_name),
    ("native/symbol-by-address", lookup::symbol_by_address),
    ("native/line-lookup", lookup::line_lookup),
    ("native/editor-find-name", edit::editor_find_name),
    ("native/edit-named-stream", edit::named_stream),
    ("native/edit-rename-public", edit::rename_public),
    ("native/rebuilt-rename-public", edit::rebuilt_rename_public),
    ("native/edit-append-type", edit::append_type),
    ("native/edit-strip-private", edit::strip_private),
    ("native/rebuild", edit::rebuild),
];

/// Return all scenarios in the order of the output.
pub(crate) fn all() -> Vec<Scenario> {
    let mut scenarios = vec![Scenario {
        name: "process/idle",
        fixture: None,
        run: idle,
    }];
    for fixture in [NATIVE_LARGE, NATIVE_SMALL] {
        for (name, run) in NATIVE {
            scenarios.push(Scenario {
                name,
                fixture: Some(fixture),
                run,
            });
        }
    }
    let other: [(&str, Fixture, Run); 8] = [
        ("portable/read", PORTABLE, portable::read),
        ("portable/rebuild", PORTABLE, portable::rebuild),
        ("portable/edit", PORTABLE, portable::edit),
        ("portable/edit-document", PORTABLE, portable::edit_document),
        ("portable/edit-method", PORTABLE, portable::edit_method),
        ("msfz/open", MSFZ, open),
        ("msfz/full-read", MSFZ, read),
        ("msfz/write-default", NATIVE_LARGE, edit::msfz_write),
    ];
    for (name, fixture, run) in other {
        scenarios.push(Scenario {
            name,
            fixture: Some(fixture),
            run,
        });
    }
    scenarios
}

/// Do nothing. The peak working set of this scenario is the base value of a child process.
fn idle(_: &Path, bench: &mut Bench) -> Outcome {
    bench.measure(|| Ok(0))
}

/// Open the file with [`Pdb::open`]: the container directory and the Info stream. The count is
/// the number of streams.
fn open(path: &Path, bench: &mut Bench) -> Outcome {
    bench.measure(|| {
        let pdb = Pdb::open(path)?;
        Ok(u64::from(pdb.container().stream_count()))
    })
}

/// Open the file with [`Pdb::open`] and decode it as the `native` example does. The count is
/// the number of decoded records and streams.
fn read(path: &Path, bench: &mut Bench) -> Outcome {
    bench.measure(|| full_read::full_read(&Pdb::open(path)?))
}
