//! The peak working set of the current process.
//!
//! The workspace denies `unsafe_code`, so the bench cannot call `GetProcessMemoryInfo` and
//! cannot install a counting allocator. .NET gives no `PeakWorkingSet64` value for a process
//! that has exited. Thus on Windows the child process asks PowerShell, a separate process, for
//! its own value before it exits. On Linux the kernel gives the value in `/proc/self/status`.

use std::process::Command;

/// Return the peak working set (peak resident set size) of this process in bytes.
///
/// On Windows, `pwsh` (or `powershell` if `pwsh` is not available) reads
/// `Process.PeakWorkingSet64` of this process. On Linux, the value is `VmHWM` from
/// `/proc/self/status`. Other systems, and a failed query, give `None`.
pub(crate) fn peak_working_set() -> Option<u64> {
    if cfg!(windows) {
        let script = format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id());
        ["pwsh", "powershell"].into_iter().find_map(|shell| {
            let output = Command::new(shell)
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            String::from_utf8_lossy(&output.stdout).trim().parse().ok()
        })
    } else if cfg!(target_os = "linux") {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
        let kib: u64 = line
            .trim_start_matches("VmHWM:")
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        kib.checked_mul(1024)
    } else {
        None
    }
}
