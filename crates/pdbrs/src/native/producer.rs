//! The producer conventions of a native PDB writer.

/// The producer whose conventions a native writer follows.
///
/// Microsoft and LLVM write the same streams with small differences, for example in the
/// order of hash table entries and in the layout of the hash streams. Each writer that has
/// such a difference documents the effect of each variant.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Producer {
    /// The Microsoft PDB library (`mspdbcore.dll`), which `link.exe` uses.
    #[default]
    Microsoft,
    /// LLVM (`lld-link` and `llvm-pdbutil`).
    Llvm,
}
