//! Fuzz MSFZ chunk decoding: stored, zstd and raw DEFLATE chunks and fragments across them.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdbrs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdbrs_fuzz::run(data, pdbrs_fuzz::msfz::chunks);
});
