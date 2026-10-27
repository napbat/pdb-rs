//! The record collections of a [`PdbEditor`] (#42).
//!
//! The `*_mut` functions of the editor give these collections. The
//! collections change only the model; the plan encodes the streams. See the
//! documentation of [`PdbEditor`] for the layouts,
//! the derived streams and the rejected edits.

pub(super) mod model;
mod module;
mod named;
pub(super) mod options;
pub(super) mod plan;
mod rebuild;
mod record;
mod remap;
pub(super) mod report;
mod source;
mod streams;
mod symbols;
mod tree;
mod types;

pub use module::ModuleContents;
pub use named::NamedStreams;
pub use record::SymbolRecord;
pub use streams::{DebugInfo, DebugStreamList, UnnamedStreams};
pub use symbols::{GlobalSymbols, PublicSymbols};
pub use tree::{ModuleSymbol, Scope};
pub use types::{IdList, TypeList};

use model::{InfoModel, ModuleSlot, Names, PdbEditor, missing};
use options::EditOptions;
use report::StreamChange;
