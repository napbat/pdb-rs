//! Fuzz the whole native PDB reader.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdbrs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdbrs_fuzz::run(data, pdbrs_fuzz::native::pdb);
});
