//! The handles of a [`PdbBuilder`](crate::native::PdbBuilder) and the resolver of deferred records.
//!
//! The builder gives a handle for each item: [`TypeHandle`], [`IdHandle`],
//! [`ModuleHandle`], [`ModuleSymbolHandle`], [`SymbolHandle`] and
//! [`StreamHandle`]. A handle is a position in the builder, not an on-disk
//! value. A record can refer to other items in two ways:
//!
//! - A decoded value, for example a
//!   [`TypeLeaf`](crate::native::codeview::type_record::TypeLeaf), has final
//!   on-disk values. The type index and the ID index of a record are `0x1000`
//!   plus the position of its handle, so a record list in stream order keeps
//!   its indices.
//! - A deferred record is a function that gets a [`Resolver`] when the plan
//!   encodes it, for example the function of
//!   [`PdbBuilder::add_type_with`](crate::native::PdbBuilder::add_type_with).
//!   The resolver converts handles to on-disk values. The function appends the
//!   encoded item: one record for a type, an ID or a symbol, one complete C13
//!   subsection with its header and padding, or the bytes of a stream. The
//!   plan calls the function once.
//!
//! [`PdbBuilder::add_module`](crate::native::PdbBuilder::add_module) gives a
//! [`ModuleBuilder`] for the symbols and subsections of one module.

pub use super::build::{
    IdHandle, ModuleBuilder, ModuleHandle, ModuleSymbolHandle, Resolver, StreamHandle,
    SymbolHandle, TypeHandle,
};
