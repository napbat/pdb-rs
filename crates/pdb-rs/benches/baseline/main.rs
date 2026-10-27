//! The baseline benchmark of `pdb-rs`.
//!
//! Run all scenarios:
//!
//! ```text
//! cargo bench -p pdb-rs --bench baseline
//! cargo bench -p pdb-rs --bench baseline -- --runs 10 --filter native/open
//! ```
//!
//! The options are:
//!
//! - `--runs <n>`: the number of timed runs of each scenario (default 10).
//! - `--filter <text>`: run only the scenarios whose `<name>@<fixture>` contains the text.
//! - `--fixtures <directory>`: the fixture directory (default `target/fixtures` of the
//!   workspace).
//!
//! The driver runs each scenario in a child process: the same executable with `--child <id>`.
//! The child does 1 warm-up run and the timed runs, then gets its own peak working set (see
//! [`memory`]). The driver prints one Markdown report in a fixed order. `docs/benchmarks/`
//! describes the method and the limits of each number.

mod edit;
mod full_read;
mod lookup;
mod measure;
mod memory;
mod portable;
mod report;
mod scenarios;

use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use measure::Bench;
use report::Row;
use scenarios::Scenario;

/// The result type of the bench functions.
type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The default number of timed runs.
const DEFAULT_RUNS: usize = 10;

/// The prefix of the result line of a child process.
const RESULT: &str = "pdb-rs-bench-result";

/// The command-line options.
struct Options {
    /// The number of timed runs.
    runs: usize,
    /// The fixture directory.
    fixtures: PathBuf,
    /// The text that the selected scenario identifiers contain.
    filter: Option<String>,
    /// The scenario identifier of a child process.
    child: Option<String>,
}

/// Return the workspace root: two directories above the `pdb-rs` manifest.
fn workspace() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest)
        .to_path_buf()
}

/// Parse the command line. Cargo adds `--bench`; the bench ignores it.
fn options() -> Outcome<Options> {
    let mut options = Options {
        runs: DEFAULT_RUNS,
        fixtures: workspace().join("target").join("fixtures"),
        filter: None,
        child: None,
    };
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let name = argument.to_string_lossy().into_owned();
        let mut value = || arguments.next().ok_or(format!("{name} needs a value"));
        match argument.to_str() {
            Some("--bench") => {}
            Some("--runs") => options.runs = value()?.to_str().ok_or("bad --runs")?.parse()?,
            Some("--fixtures") => options.fixtures = PathBuf::from(value()?),
            Some("--filter") => {
                options.filter = Some(value()?.into_string().map_err(|_| "bad --filter")?);
            }
            Some("--child") => {
                options.child = Some(value()?.into_string().map_err(|_| "bad --child")?);
            }
            _ => return Err(format!("unknown argument {name}").into()),
        }
    }
    if options.runs == 0 {
        return Err("--runs must be 1 or more".into());
    }
    Ok(options)
}

fn main() -> ExitCode {
    let result = options().and_then(|options| match &options.child {
        Some(id) => child(&options, id),
        None => driver(&options),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run the scenario `id` in this process and print its result line.
fn child(options: &Options, id: &str) -> Outcome {
    let scenario = scenarios::all()
        .into_iter()
        .find(|scenario| scenario.id() == id)
        .ok_or_else(|| format!("unknown scenario {id}"))?;
    let path = scenario
        .fixture
        .map(|fixture| options.fixtures.join(fixture.path))
        .unwrap_or_default();
    let mut bench = Bench::new(options.runs);
    (scenario.run)(&path, &mut bench)?;
    let (count, times) = bench.finish()?;
    let peak = memory::peak_working_set().map_or_else(|| "-".to_owned(), |bytes| bytes.to_string());
    let times: Vec<String> = times
        .iter()
        .map(|time| time.as_nanos().to_string())
        .collect();
    println!("{RESULT} {count} {peak} {}", times.join(","));
    Ok(())
}

/// Parse the result line of a child process.
fn parse_result(stdout: &str) -> Outcome<Row> {
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(RESULT))
        .ok_or("the child process printed no result")?;
    let mut fields = line.split_whitespace();
    let mut field = || fields.next().ok_or("the result line is too short");
    let count = field()?.parse()?;
    let peak = match field()? {
        "-" => None,
        bytes => Some(bytes.parse()?),
    };
    let times = field()?
        .split(',')
        .map(|nanos| Ok(Duration::from_nanos(nanos.parse()?)))
        .collect::<Outcome<_>>()?;
    Ok(Row { count, peak, times })
}

/// Run one scenario in a child process and return its table row, or an error message.
fn run_child(options: &Options, scenario: &Scenario) -> Outcome<Row> {
    let output = Command::new(std::env::current_exe()?)
        .arg("--child")
        .arg(scenario.id())
        .arg("--runs")
        .arg(options.runs.to_string())
        .arg("--fixtures")
        .arg(&options.fixtures)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_owned().into());
    }
    parse_result(&String::from_utf8_lossy(&output.stdout))
}

/// Run each selected scenario in a child process and print the report.
fn driver(options: &Options) -> Outcome {
    let selected: Vec<Scenario> = scenarios::all()
        .into_iter()
        .filter(|scenario| {
            let id = scenario.id();
            options
                .filter
                .as_ref()
                .is_none_or(|text| id.contains(text.as_str()))
        })
        .collect();
    let mut fixtures = Vec::new();
    for fixture in selected.iter().filter_map(|scenario| scenario.fixture) {
        if !fixtures
            .iter()
            .any(|known: &scenarios::Fixture| known.path == fixture.path)
        {
            fixtures.push(fixture);
        }
    }
    println!(
        "{}",
        report::header(&workspace(), &options.fixtures, &fixtures, options.runs)
    );
    let mut missing = Vec::new();
    let mut failed = Vec::new();
    for scenario in selected {
        let id = scenario.id();
        let fixture = scenario.fixture.map_or("-", |fixture| fixture.name);
        if let Some(path) = scenario
            .fixture
            .map(|fixture| options.fixtures.join(fixture.path))
            && !path.is_file()
        {
            println!("{}", report::no_result(scenario.name, fixture, "missing"));
            if !missing.contains(&path) {
                missing.push(path);
            }
            continue;
        }
        match run_child(options, &scenario) {
            Ok(row) => println!("{}", report::row(scenario.name, fixture, &row)),
            Err(error) => {
                println!("{}", report::no_result(scenario.name, fixture, "failed"));
                failed.push(format!("{id}: {error}"));
            }
        }
        std::io::stdout().flush()?;
    }
    if !missing.is_empty() {
        println!("\nMissing fixtures (no numbers):\n");
        for path in &missing {
            println!("- `{}`", path.display());
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        println!("\nFailed scenarios:\n");
        for line in &failed {
            println!("- {line}");
        }
        Err(format!("{} scenarios failed", failed.len()).into())
    }
}
