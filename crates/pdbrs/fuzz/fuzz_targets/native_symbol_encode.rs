//! Fuzz the symbol record encoders and the symbol stream writers (round trip).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdbrs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdbrs_fuzz::run(data, pdbrs_fuzz::native::encode::symbols::stream);
});
