//! The Markdown report: the environment, the record sizes and the scenario rows.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use pdbrs::native::codeview::id_record::IdRecord;
use pdbrs::native::codeview::symbol::{Public, Symbol, SymbolData};
use pdbrs::native::codeview::type_record::TypeLeaf;

use crate::scenarios::Fixture;

/// The result of one child process.
pub(crate) struct Row {
    /// The item count of each run.
    pub(crate) count: u64,
    /// The peak working set in bytes, if the query gave a value.
    pub(crate) peak: Option<u64>,
    /// The durations of the timed runs.
    pub(crate) times: Vec<Duration>,
}

/// Run `program` with `arguments` in `directory` and return the first line of its output.
fn first_line(program: &str, arguments: &[&str], directory: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(|line| line.trim().to_owned())
}

/// Return the commit of the work tree, with a note if it has uncommitted changes.
fn commit(directory: &Path) -> String {
    let Some(head) = first_line("git", &["rev-parse", "HEAD"], directory) else {
        return "unknown".to_owned();
    };
    let changed = first_line(
        "git",
        &["status", "--porcelain", "--untracked-files=no"],
        directory,
    );
    match changed {
        Some(_) => format!("{head} (with uncommitted changes)"),
        None => head,
    }
}

/// Return the operating system, the processor and the physical memory.
fn machine(directory: &Path) -> (String, String, String) {
    let unknown = || "unknown".to_owned();
    if cfg!(windows) {
        let script = "$o = Get-CimInstance Win32_OperatingSystem; \
            $p = Get-CimInstance Win32_Processor | Select-Object -First 1; \
            $m = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory; \
            \"$($o.Caption) $($o.Version)|$($p.Name.Trim())|$m\"";
        let line = first_line(
            "pwsh",
            &["-NoProfile", "-NonInteractive", "-Command", script],
            directory,
        );
        if let Some(line) = line {
            let mut parts = line.split('|').map(str::to_owned);
            let os = parts.next().unwrap_or_else(unknown);
            let cpu = parts.next().unwrap_or_else(unknown);
            let memory = parts
                .next()
                .and_then(|bytes| bytes.parse::<u64>().ok())
                .map_or_else(unknown, gibibytes);
            return (os, cpu, memory);
        }
    } else if cfg!(target_os = "linux") {
        let field = |file: &str, key: &str| {
            let text = std::fs::read_to_string(file).ok()?;
            let line = text.lines().find(|line| line.starts_with(key))?;
            Some(line.split_once(':')?.1.trim().to_owned())
        };
        let cpu = field("/proc/cpuinfo", "model name").unwrap_or_else(unknown);
        let memory = field("/proc/meminfo", "MemTotal")
            .and_then(|text| text.trim_end_matches("kB").trim().parse::<u64>().ok())
            .map_or_else(unknown, |kib| gibibytes(kib.saturating_mul(1024)));
        let os = first_line("uname", &["-sr"], directory).unwrap_or_else(unknown);
        return (os, cpu, memory);
    }
    (std::env::consts::OS.to_owned(), unknown(), unknown())
}

/// Format a byte count in GiB.
fn gibibytes(bytes: u64) -> String {
    format!("{:.1} GiB", mebibytes(bytes) / 1024.0)
}

/// Return a byte count in MiB.
fn mebibytes(bytes: u64) -> f64 {
    // The value is for display only; the precision loss above 2^52 bytes does not matter.
    let bytes = u32::try_from(bytes / 1024).map_or(f64::from(u32::MAX), f64::from);
    bytes / 1024.0
}

/// Return the environment, the record size and the fixture sections of the report, and the
/// head of the scenario table.
pub(crate) fn header(directory: &Path, fixtures: &Path, used: &[Fixture], runs: usize) -> String {
    let (os, cpu, memory) = machine(directory);
    let rustc = first_line("rustc", &["-V"], directory).unwrap_or_else(|| "unknown".to_owned());
    let processors = std::thread::available_parallelism().map_or(0, usize::from);
    let mut text = String::from("## Environment\n\n| Item | Value |\n| --- | --- |\n");
    let rows = [
        ("Commit", commit(directory)),
        ("Toolchain", rustc),
        (
            "Debug assertions",
            if cfg!(debug_assertions) { "on" } else { "off" }.to_owned(),
        ),
        ("Operating system", os),
        ("Architecture", std::env::consts::ARCH.to_owned()),
        ("Processor", cpu),
        ("Logical processors", processors.to_string()),
        ("Physical memory", memory),
        ("Fixtures", fixtures.display().to_string()),
        (
            "Runs",
            format!("1 warm-up run and {runs} timed runs in one child process for each row"),
        ),
    ];
    for (item, value) in rows {
        let _ = writeln!(text, "| {item} | {value} |");
    }
    text.push_str("\n## Record sizes\n\n| Type | `size_of` in bytes |\n| --- | ---: |\n");
    let sizes = [
        ("Symbol", size_of::<Symbol<'static>>()),
        ("SymbolData", size_of::<SymbolData<'static>>()),
        ("TypeLeaf", size_of::<TypeLeaf<'static>>()),
        ("IdRecord", size_of::<IdRecord<'static>>()),
        ("Public", size_of::<Public<'static>>()),
    ];
    for (name, size) in sizes {
        let _ = writeln!(text, "| `{name}` | {size} |");
    }
    text.push_str("\n## Fixtures\n\n| Fixture | File | Bytes |\n| --- | --- | ---: |\n");
    for fixture in used {
        let bytes = std::fs::metadata(fixtures.join(fixture.path)).map_or_else(
            |_| "missing".to_owned(),
            |metadata| metadata.len().to_string(),
        );
        let _ = writeln!(text, "| {} | `{}` | {bytes} |", fixture.name, fixture.path);
    }
    text.push_str(
        "\n## Scenarios\n\n\
         | Scenario | Fixture | Count | Min ms | Median ms | Max ms | Peak working set MiB |\n\
         | --- | --- | ---: | ---: | ---: | ---: | ---: |",
    );
    text
}

/// Format a duration in milliseconds.
fn milliseconds(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1000.0)
}

/// Return the table row of a scenario result.
pub(crate) fn row(scenario: &str, fixture: &str, row: &Row) -> String {
    let mut times = row.times.clone();
    times.sort_unstable();
    let (min, median, max) = match times.as_slice() {
        [] => ("-".to_owned(), "-".to_owned(), "-".to_owned()),
        [first, .., last] | [first @ last] => {
            let middle = times.len() / 2;
            let median = if times.len().is_multiple_of(2) {
                (times[middle - 1] + times[middle]) / 2
            } else {
                times[middle]
            };
            (
                milliseconds(*first),
                milliseconds(median),
                milliseconds(*last),
            )
        }
    };
    let peak = row.peak.map_or_else(
        || "-".to_owned(),
        |bytes| format!("{:.1}", mebibytes(bytes)),
    );
    format!(
        "| {scenario} | {fixture} | {} | {min} | {median} | {max} | {peak} |",
        row.count
    )
}

/// Return the table row of a scenario that did not run, with `reason` in the count column.
pub(crate) fn no_result(scenario: &str, fixture: &str, reason: &str) -> String {
    format!("| {scenario} | {fixture} | {reason} | - | - | - | - |")
}
