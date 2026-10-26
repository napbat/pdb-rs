//! The target CPU of a compile symbol.

/// A target CPU (`CV_CPU_TYPE_e` in `cvconst.h`), as in `S_COMPILE2` and `S_COMPILE3`.
///
/// The value set is open: the type keeps values without a constant. This is a different
/// value space from the PE/COFF machine type of the DBI header.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CpuType(pub u16);

impl CpuType {
    /// `CV_CFL_80386`.
    pub const INTEL_80386: Self = Self(0x03);
    /// `CV_CFL_PENTIUMIII`.
    pub const PENTIUM_III: Self = Self(0x07);
    /// `CV_CFL_IA64`.
    pub const IA64: Self = Self(0x80);
    /// `CV_CFL_AMD64` (also `CV_CFL_X64`).
    pub const X64: Self = Self(0xd0);
    /// `CV_CFL_ARMNT`.
    pub const ARMNT: Self = Self(0xf4);
    /// `CV_CFL_ARM64`.
    pub const ARM64: Self = Self(0xf6);
}
