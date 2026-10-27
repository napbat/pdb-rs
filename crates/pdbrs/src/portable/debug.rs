//! Records, blob codecs and validation of the Portable PDB debug tables.
//!
//! The layouts and rules come from `docs/design/specs/PortablePdb-Metadata.md` in
//! `dotnet/runtime` at commit `736475ca4a183e931d84cafb4954cff582b59ae9`. The encoders write
//! the form that Roslyn writes (MIT, commit `e3c31f15b79e226656e3b082a921d4d60127b1fa`,
//! `src/Compilers/Core/Portable/PEWriter/MetadataWriter.PortablePdb.cs`).
//!
//! # Records
//!
//! The records hold the data of the debug tables with public fields and without positions.
//! The [`Pdb`](super::Pdb) accessors return them, each next to its row ID, and a
//! [`PdbEditor`](super::PdbEditor) holds them:
//!
//! | Table | Record | Accessors |
//! | --- | --- | --- |
//! | `Document` | [`Document`] | `document`, `documents` |
//! | `MethodDebugInformation` | [`MethodDebugInformation`] | `method_debug_information`, `method_debug_informations` |
//! | `LocalScope` | [`LocalScope`] (with its variables and constants) | `local_scope`, `local_scopes` |
//! | `LocalVariable` | [`LocalVariable`] | `local_variable`, `local_variables` |
//! | `LocalConstant` | [`LocalConstant`] | `local_constant`, `local_constants` |
//! | `ImportScope` | [`ImportScope`] (with its [`Import`] list) | `import_scope`, `import_scopes` |
//! | `StateMachineMethod` | [`StateMachineMethod`] | `state_machine_method`, `state_machine_methods` |
//!
//! The `CustomDebugInformation` record is in [`custom`](super::custom).
//!
//! A blob that contains `#Blob` offsets is decoded: the document name is the joined name,
//! and the imports hold their names. The other blobs stay encoded, and the `decode` function
//! of the record decodes them on request: [`MethodDebugInformation::decode`] and
//! [`LocalConstant::decode`]. The raw rows and the heap offsets are the table layer in
//! [`metadata`](super::metadata).
//!
//! A local scope, a local variable and a local constant get their rows in the plan of an
//! editor, so a reference to them is a handle: [`LocalScopeHandle`], [`LocalVariableHandle`]
//! and [`LocalConstantHandle`].
//!
//! # Codecs
//!
//! The blob codecs borrow the blob bytes and check each value:
//!
//! - [`DocumentName`]: the separator and the `#Blob` offsets of the name parts.
//! - [`SequencePoints`]: a lazy iterator over the sequence points of a method.
//! - [`LocalConstantSignature`]: the custom modifiers and the value of a local constant.
//! - [`Imports`]: a lazy iterator over the [`RawImport`] values of an import scope.
//!
//! Each codec also has an encoder: [`DocumentName::encode_parts`],
//! [`SequencePoints::encode_parts`], [`LocalConstantSignature::encode_parts`] and
//! [`Imports::encode_parts`]. The encoders take resolved heap offsets and row IDs. They do not
//! add heap entries. [`split_document_name`] gives the name parts that need `#Blob` entries.
//!
//! The codec errors have no location. The `Pdb` accessors add the table, the row, the column
//! and the heap offset to the errors of the blobs that they decode.
//! [`Pdb::validate_debug`](super::Pdb::validate_debug) checks the ordering and reference rules
//! of all debug tables and returns a [`ValidationReport`].

mod constant;
mod document;
mod imports;
pub(crate) mod rows;
mod scope;
mod sequence;
mod type_handle;
mod validate;

pub use constant::{
    ConstantValue, CustomModifier, CustomModifiers, Decimal, GeneralValue, LocalConstant,
    LocalConstantSignature, PrimitiveConstant,
};
pub use document::{
    Document, DocumentName, DocumentNameParts, HashAlgorithm, Language, SplitDocumentName,
    split_document_name,
};
pub use imports::{Import, ImportScope, Imports, RawImport};
pub use rows::TypedRows;
pub use scope::{
    LocalConstantHandle, LocalScope, LocalScopeHandle, LocalVariable, LocalVariableHandle,
    StateMachineMethod,
};
pub use sequence::{
    HIDDEN_LINE, LocalSignatureForm, MAX_COLUMN, MAX_IL_OFFSET, MAX_LINE, MethodDebugInformation,
    SequencePoint, SequencePoints, SourceSpan,
};
pub use validate::{Outcome, Rule, RuleReport, ValidateOptions, ValidationReport};
