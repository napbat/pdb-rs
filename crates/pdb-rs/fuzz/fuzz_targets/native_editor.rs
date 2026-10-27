//! Fuzz the native editor: edits of a PDB, the write plan and its output.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(init: pdb_rs_fuzz::exit_on_panic(), |data: &[u8]| {
    pdb_rs_fuzz::run(data, pdb_rs_fuzz::native::editor::editor);
});
