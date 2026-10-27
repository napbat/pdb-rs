//! The record collections of a [`PdbEditor`](crate::portable::PdbEditor).
//!
//! The `*_mut` functions of the editor give these collections. Documents and import scopes
//! keep the order of addition, so their IDs are the row IDs of the output. The plan gives
//! local scopes and custom debug information their rows, so the collections give handles.

mod collections;
mod decode;
pub(super) mod model;
mod resolve;
mod tasks;

use alloc::collections::BTreeMap;

pub use collections::{
    CustomDebugInformations, Documents, EmptyListForm, ImportScopes, LocalScopes,
};

use crate::portable::debug::MethodDebugInformation;
use crate::portable::metadata::MethodDefId;

/// The methods with sequence points of an editor.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Methods {
    /// The sequence points of each method.
    records: BTreeMap<MethodDefId, MethodDebugInformation<'static>>,
    /// `true` if the `MethodDebugInformation` table has a row for each method, also if no
    /// method has sequence points.
    table: bool,
}
