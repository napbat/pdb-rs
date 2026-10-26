//! Shared code of the `pdbrs` fuzz targets.
//!
//! Each fuzz target in `fuzz_targets/` calls one function of this crate. The
//! functions use only the public API of `pdbrs`. A reader target ignores
//! errors: an error is a valid result for malformed input. A panic, a hang,
//! an out-of-memory condition or a failed assertion is a finding. A round-trip
//! target asserts that the output of an encoder decodes to the same value.

pub mod convert;
pub mod input;
pub mod msf;
pub mod msfz;
pub mod native;
pub mod portable;

/// The largest stream or decoded payload that a target reads (16 MiB).
pub const MAX_STREAM: usize = 16 << 20;

/// Return the value of `result`, or `None` for an error.
pub fn ok<T>(result: pdbrs::Result<T>) -> Option<T> {
    result.ok()
}

/// Run an encoder that appends to a buffer, and return the bytes.
///
/// # Errors
///
/// Returns the error of `encode`.
pub fn encoded(encode: impl FnOnce(&mut Vec<u8>) -> pdbrs::Result<()>) -> pdbrs::Result<Vec<u8>> {
    let mut out = Vec::new();
    encode(&mut out)?;
    Ok(out)
}

thread_local! {
    /// A copy of the input of the current run, for the panic hook.
    static INPUT: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Keep a copy of `data` for the panic hook, then call `target` with it.
pub fn run(data: &[u8], target: impl FnOnce(&[u8])) {
    INPUT.with_borrow_mut(|input| {
        input.clear();
        input.extend_from_slice(data);
    });
    target(data);
}

/// Replace the panic hook of `libfuzzer-sys`: print the panic, write the input and exit.
///
/// The hook of `libfuzzer-sys` calls `std::process::abort`. On Windows, `abort` ends the
/// process with `__fastfail`, so libFuzzer cannot write the input to `artifacts/`. This hook
/// writes the input that [`run`] keeps to `<artifact_prefix>crash-<FNV-1a hash>` and exits
/// with code 101. Each target calls this function in its `init` block.
pub fn exit_on_panic() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        eprintln!("{}", std::backtrace::Backtrace::force_capture());
        let prefix = std::env::args()
            .find_map(|argument| {
                argument
                    .strip_prefix("-artifact_prefix=")
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let input = INPUT.with_borrow(Clone::clone);
        let hash = input.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
        let path = format!("{prefix}crash-{hash:016x}");
        match std::fs::write(&path, &input) {
            Ok(()) => eprintln!("panic input written to {path}"),
            Err(error) => eprintln!("cannot write the panic input to {path}: {error}"),
        }
        std::process::exit(101);
    }));
}
