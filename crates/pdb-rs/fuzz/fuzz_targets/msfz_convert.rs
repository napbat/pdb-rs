//! Fuzz the conversion round trip between MSF 7.0 and MSFZ.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdb_rs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdb_rs_fuzz::run(data, pdb_rs_fuzz::convert::round_trip);
});
