//! Fuzz the address remap (#43): OMAP output and clean output of a synthetic PDB.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdbrs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdbrs_fuzz::run(data, pdbrs_fuzz::native::remap::remap);
});
