//! Building blocks for native Windows and Portable .NET PDB files.
//!
//! The current API provides positional byte access, signature probing and
//! format-specific identity types. It does not yet parse PDB streams, decode
//! debug records or write PDB files.
//!
//! [`format::probe`] identifies containers, not valid PDBs. In particular,
//! CLI metadata is not necessarily a Portable PDB. Legacy MSF identification
//! does not imply support for decoding legacy records.
//!
//! Disable the default `std` feature to use the library without the standard
//! library. Slice access, probing and identity operations need no allocator.
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

pub mod error;
pub mod format;
pub mod io;
pub mod native;
pub mod portable;

pub use error::{Error, ErrorKind, Result};
