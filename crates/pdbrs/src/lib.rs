//! Building blocks for native Windows and Portable .NET PDB files.
//!
//! The API reads MSF 7.0 and MSFZ containers and native PDB streams. [`native::Pdb`]
//! decodes every modern `CodeView` record kind, numeric leaf and C13 subsection
//! that has a layout definition, and checks the references between records.
//! Record kinds without a layout remain available as raw bytes.
//! [`native::container::Container`] gives one stream reader for both containers.
//!
//! [`portable::Pdb`] reads standalone Portable PDBs: the metadata root, the
//! heaps, the `#Pdb` stream, the `#~` table layout and the raw rows of the
//! eight debug tables. Semantic decoding of the debug blobs is not complete.
//!
//! [`Pdb`] detects the format from the contents and opens the input with the
//! native or the portable backend. It rejects ordinary CLI metadata (for
//! example the metadata of an assembly) because it has no `#Pdb` stream.
//!
//! [`native::msf`] writes MSF 7.0 containers: exact copies, stream-preserving
//! repacks and new layouts of arbitrary streams. [`native::msfz`] writes MSFZ
//! containers, and [`native::container::Container`] converts between MSF 7.0
//! and MSFZ. Each native record and stream has an encoder.
//! [`native::write::PdbBuilder`] builds new native PDBs, and
//! [`native::PdbEditor`] edits native PDBs and rebuilds the
//! dependent indexes. [`portable::write`] writes Portable PDBs: exact copies
//! and new layouts from raw rows and heaps. [`portable::PdbBuilder`] and
//! [`portable::PdbEditor`] build and edit Portable PDBs from typed records.
//!
//! [`format::probe`] identifies containers, not valid PDBs. In particular,
//! CLI metadata is not necessarily a Portable PDB. Legacy MSF identification
//! does not imply support for decoding legacy records.
//!
//! Disable the default `std` feature to use `no_std + alloc`. Sequential I/O
//! comes from `nostdio`; [`io::SeekReader`] adapts seekable streams to [`io::ReadAt`].
//! Slice reads, probing and fixed-buffer cursor operations do not allocate.
//!
//! # Example
//!
//! ```
//! use pdbrs::format::{ContainerKind, probe};
//!
//! let bytes = b"BSJB";
//! assert_eq!(probe(bytes.as_slice())?, ContainerKind::CliMetadata);
//! # Ok::<(), pdbrs::Error>(())
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod compression;
mod crypto;
pub mod error;
pub mod format;
pub mod io;
pub mod native;
pub mod portable;
#[cfg(test)]
mod test_support;

pub use error::{Error, ErrorKind, Result};
pub use format::{Format, Pdb};
