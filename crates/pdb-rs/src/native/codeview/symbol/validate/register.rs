//! The known register numbers (`CV_HREG_e`) for each CPU family.
//!
//! The ranges come from `CV_HREG_e` in `cvconst.h` of the DIA SDK in Visual
//! Studio 18. `CodeViewRegisters.def` in LLVM (revision `b9f2235f`) lists
//! fewer registers: for example, it does not have `CV_REG_XMM6L` (200), which
//! MSVC writes for x64 code. Each range holds consecutive register numbers.
//! Every table also holds 0 (`CV_REG_NONE`) and the pseudo registers that all
//! CPUs share, `CV_ALLREG_ERR` (30000) to `CV_ALLREG_CMDLN` (30012).
//!
//! The CPU values come from `CPUType` in LLVM `CodeView.h`.

use super::super::CpuType;

/// The x86 register numbers (`CV_REG_*`).
const X86: &[(u16, u16)] = &[
    (0, 34),
    (40, 47),
    (80, 84),
    (90, 97),
    (110, 124),
    (128, 209),
    (211, 212),
    (220, 426),
    (30000, 30012),
];

/// The x64 register numbers (`CV_AMD64_*`).
const X64: &[(u16, u16)] = &[
    (0, 30),
    (32, 34),
    (80, 84),
    (88, 88),
    (90, 105),
    (110, 115),
    (128, 209),
    (211, 211),
    (220, 919),
    (30000, 30012),
];

/// The 32-bit ARM register numbers (`CV_ARM_*`).
const ARM: &[(u16, u16)] = &[
    (0, 0),
    (10, 27),
    (40, 41),
    (50, 81),
    (90, 97),
    (128, 159),
    (200, 231),
    (300, 331),
    (400, 415),
    (30000, 30012),
];

/// The ARM64 register numbers (`CV_ARM64_*`).
const ARM64: &[(u16, u16)] = &[
    (0, 0),
    (10, 41),
    (50, 83),
    (90, 91),
    (100, 131),
    (140, 171),
    (180, 211),
    (220, 221),
    (230, 261),
    (270, 301),
    (310, 341),
    (350, 430),
    (30000, 30012),
];

/// The MIPS register numbers (`CV_M4_*`).
const MIPS: &[(u16, u16)] = &[(0, 0), (10, 43), (50, 51), (60, 92), (30000, 30012)];

/// Return the register ranges of a CPU (`CV_CPU_TYPE_e`), or `None` if the
/// CPU family has no table.
const fn ranges(cpu: CpuType) -> Option<&'static [(u16, u16)]> {
    match cpu.0 {
        // Intel 8080 to Pentium III.
        0x00..=0x07 => Some(X86),
        // MIPS to MIPS V.
        0x10..=0x18 => Some(MIPS),
        // ARM3 to ARM7, Thumb and ARMNT.
        0x60..=0x68 | 0xf0 | 0xf4 => Some(ARM),
        0xd0 => Some(X64),
        0xf6 => Some(ARM64),
        _ => None,
    }
}

/// Return `Some(true)` if `register` is a known register of `cpu`,
/// `Some(false)` if it is not, and `None` if the CPU has no table.
pub(super) fn known(cpu: CpuType, register: u16) -> Option<bool> {
    ranges(cpu).map(|ranges| {
        ranges
            .iter()
            .any(|&(first, last)| (first..=last).contains(&register))
    })
}
