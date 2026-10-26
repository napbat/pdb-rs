//! Fuzz the MSFZ container reader: header, stream directory, Chunk Table and stream reads.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdbrs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdbrs_fuzz::run(data, pdbrs_fuzz::msfz::read);
});
