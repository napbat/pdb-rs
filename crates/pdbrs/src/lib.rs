//! Building blocks for native Windows and Portable .NET PDB files.
//!
//! The API reads MSF 7.0 containers, native PDB metadata and framed debug
//! records. It decodes common data, public and procedure symbols. Other record
//! kinds remain available as raw bytes. Full semantic decoding is not complete.
//! Portable PDB parsing and PDB writing are not implemented.
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

pub mod error;
pub mod format;
pub mod io;
pub mod native;
pub mod portable;

pub use error::{Error, ErrorKind, Result};
