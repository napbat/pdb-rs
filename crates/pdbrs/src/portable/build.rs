//! The records and the handles of a [`PdbBuilder`](crate::portable::PdbBuilder).
//!
//! The builder holds its records as owned values: [`DocumentInfo`], [`MethodInfo`],
//! [`LocalScopeInfo`], [`LocalVariableInfo`], [`LocalConstantInfo`], [`ImportScopeInfo`] and
//! [`CustomDebugInfo`]. Records refer to each other through typed handles, not row IDs:
//!
//! - Documents and import scopes keep the order of addition, so their handles are the final
//!   [`DocumentId`](crate::portable::metadata::DocumentId) and
//!   [`ImportScopeId`](crate::portable::metadata::ImportScopeId).
//! - Local scopes, local variables, local constants and custom debug information get their
//!   rows in the plan, so they have the opaque handles [`LocalScopeHandle`],
//!   [`LocalVariableHandle`], [`LocalConstantHandle`] and [`CustomDebugHandle`].

pub use super::builder::{
    CustomDebugHandle, CustomDebugInfo, CustomDebugParent, DocumentInfo, ImportInfo,
    ImportScopeInfo, LocalConstantHandle, LocalConstantInfo, LocalScopeHandle, LocalScopeInfo,
    LocalVariableHandle, LocalVariableInfo, MethodInfo,
};
